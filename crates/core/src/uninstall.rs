//! Installed programs (registry), uninstalling, and leftover detection.

use crate::fsops::size_of;
use crate::guard::Guard;
use crate::progress::Progress;
use crate::util::{expand_env, key_within, norm_key, norm_name, split_command};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct App {
    pub name: String,
    pub publisher: String,
    pub version: String,
    /// `YYYY-MM-DD` or empty.
    pub install_date: String,
    pub size: u64,
    pub uninstall: String,
    pub quiet_uninstall: Option<String>,
    pub install_location: Option<String>,
    /// Full key, e.g. `HKLM\SOFTWARE\...\Uninstall\{GUID}`.
    pub reg_key: String,
    /// System component, update or entry without an uninstaller.
    pub system: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftoverKind {
    Folder,
    Registry,
}

#[derive(Debug, Clone)]
pub struct Leftover {
    pub kind: LeftoverKind,
    pub path: String,
    pub size: u64,
    pub checked: bool,
    pub reason: String,
}

/// Turn `/I{GUID}` (repair/modify) into `/X{GUID}` (uninstall) for msiexec.
fn msi_uninstall_args(args: &str) -> String {
    args.split_whitespace()
        .map(|tok| {
            let lower = tok.to_ascii_lowercase();
            if lower == "/i" {
                "/X".to_string()
            } else if lower.starts_with("/i{") {
                format!("/X{}", &tok[2..])
            } else {
                tok.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// (program, raw args) to run for uninstalling `app`.
pub fn build_command(app: &App, quiet: bool) -> Option<(String, String)> {
    let raw = match (&app.quiet_uninstall, quiet) {
        (Some(q), true) => q.clone(),
        _ => app.uninstall.clone(),
    };
    if raw.trim().is_empty() {
        return None;
    }
    let (prog, args) = split_command(&raw);
    let prog = expand_env(&prog).unwrap_or(prog);
    let exe = Path::new(&prog).file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if exe == "msiexec.exe" || exe == "msiexec" {
        let mut args = msi_uninstall_args(&args);
        if quiet && app.quiet_uninstall.is_none() {
            args.push_str(" /passive /norestart");
        }
        return Some((crate::paths::system_tool("msiexec.exe").display().to_string(), args));
    }
    Some((prog, args))
}

/// Run the uninstaller and wait until it (and anything it spawned) has exited.
pub fn uninstall(app: &App, quiet: bool) -> Result<(), String> {
    let (prog, args) = build_command(app, quiet).ok_or("this program has no uninstaller")?;
    #[allow(unused_mut)]
    let mut cmd = std::process::Command::new(&prog);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.raw_arg(&args);
    }
    #[cfg(not(windows))]
    cmd.args(args.split_whitespace());
    crate::sys::run_wait_tree(cmd, std::time::Duration::from_secs(2 * 3600)).map(|_| ())
}

const VERSIONISH: &[&str] = &["x64", "x86", "64bit", "32bit", "64-bit", "32-bit", "amd64", "arm64", "win64", "win32"];

/// Normalized names an app's folders/keys are likely to use.
pub fn app_keys(app: &App) -> Vec<String> {
    let mut name = String::new();
    let mut depth = 0;
    for c in app.name.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            _ if depth == 0 => name.push(c),
            _ => {}
        }
    }
    let words: Vec<&str> = name
        .split_whitespace()
        .filter(|w| {
            let lw = w.to_lowercase();
            !VERSIONISH.contains(&lw.trim_matches(|c: char| !c.is_alphanumeric()))
                && !(w.chars().any(|c| c.is_ascii_digit()) && w.chars().all(|c| c.is_ascii_digit() || c == '.' || c == 'v'))
                && lw != "-"
        })
        .collect();
    let mut keys = vec![norm_name(&words.join(" ")), norm_name(&app.name)];
    if let Some(loc) = &app.install_location {
        if let Some(leaf) = Path::new(loc.trim_matches('"')).file_name() {
            keys.push(norm_name(&leaf.to_string_lossy()));
        }
    }
    keys.retain(|k| k.len() >= 4);
    keys.sort();
    keys.dedup();
    keys
}

const NEVER_MATCH: &[&str] = &[
    "microsoft", "windows", "classes", "policies", "wow6432node", "clients", "registeredapplications",
    "common", "commonfiles", "programs", "packages", "temp", "intel", "nvidia", "amd", "google",
    "mozilla", "apple", "adobe", "oracle", "java", "python", "steam", "startmenu", "users", "default",
];

fn folder_roots() -> Vec<PathBuf> {
    [
        "%ProgramFiles%",
        "%ProgramFiles(x86)%",
        "%ProgramData%",
        "%APPDATA%",
        "%LOCALAPPDATA%",
        "%LOCALAPPDATA%\\Programs",
        "%USERPROFILE%\\AppData\\LocalLow",
        "%APPDATA%\\Microsoft\\Windows\\Start Menu\\Programs",
        "%ProgramData%\\Microsoft\\Windows\\Start Menu\\Programs",
    ]
    .iter()
    .filter_map(|s| expand_env(s))
    .map(PathBuf::from)
    .filter(|p| p.is_dir())
    .collect()
}

fn child_dirs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.metadata().map(|m| m.is_dir() && !crate::paths::is_link(&m)).unwrap_or(false))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

/// Find folders and registry keys left behind by `app`. `others` are the apps that are
/// still installed; anything that could belong to them is never suggested.
pub fn find_leftovers(app: &App, others: &[App], guard: &Guard) -> Vec<Leftover> {
    let keys = app_keys(app);
    let publisher = norm_name(&app.publisher);
    let other_names: HashSet<String> = others.iter().flat_map(app_keys).collect();
    let other_locations: Vec<String> = others
        .iter()
        .filter_map(|o| o.install_location.as_deref())
        .map(|l| norm_key(Path::new(l.trim_matches('"'))))
        .filter(|k| k.len() > 3)
        .collect();
    let matches = |name: &str| {
        let n = norm_name(name);
        keys.contains(&n) && !other_names.contains(&n) && !NEVER_MATCH.contains(&n.as_str())
    };
    let progress = Progress::default();
    let mut found: Vec<Leftover> = Vec::new();
    let mut seen = HashSet::new();
    let mut add_folder = |p: PathBuf, reason: &str, checked: bool| {
        let k = norm_key(&p);
        let shared = other_locations.iter().any(|o| key_within(o, &k) || key_within(&k, o));
        if shared || guard.check_delete(&p).is_err() || !seen.insert(k) {
            return;
        }
        let size = size_of(&p, &progress).bytes;
        found.push(Leftover { kind: LeftoverKind::Folder, path: p.display().to_string(), size, checked, reason: reason.into() });
    };

    if let Some(loc) = &app.install_location {
        let p = PathBuf::from(loc.trim_matches('"'));
        if p.is_dir() {
            add_folder(p, "Install folder", true);
        }
    }
    for root in folder_roots() {
        for child in child_dirs(&root) {
            let name = child.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if matches(&name) {
                add_folder(child, "Matches program name", true);
            } else if publisher.len() >= 3 && norm_name(&name) == publisher {
                for sub in child_dirs(&child) {
                    let sub_name = sub.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if matches(&sub_name) {
                        add_folder(sub, "Matches publisher and program name", true);
                    }
                }
            }
        }
    }

    #[cfg(windows)]
    {
        use crate::reg::{open, subkeys, Hive};
        for (hive, base) in [(Hive::Hkcu, "Software"), (Hive::Hklm, "SOFTWARE"), (Hive::Hklm, "SOFTWARE\\WOW6432Node")] {
            let Some(root) = open(hive, base) else { continue };
            for sub in subkeys(&root) {
                let full = format!("{}\\{base}\\{sub}", hive.prefix());
                if matches(&sub) {
                    found.push(Leftover { kind: LeftoverKind::Registry, path: full, size: 0, checked: true, reason: "Matches program name".into() });
                } else if publisher.len() >= 3 && norm_name(&sub) == publisher && !NEVER_MATCH.contains(&publisher.as_str()) {
                    if let Some(pk) = open(hive, &format!("{base}\\{sub}")) {
                        for s2 in subkeys(&pk) {
                            if matches(&s2) {
                                found.push(Leftover {
                                    kind: LeftoverKind::Registry,
                                    path: format!("{full}\\{s2}"),
                                    size: 0,
                                    checked: true,
                                    reason: "Matches publisher and program name".into(),
                                });
                            }
                        }
                    }
                }
            }
        }
        if key_exists(&app.reg_key) {
            found.push(Leftover {
                kind: LeftoverKind::Registry,
                path: app.reg_key.clone(),
                size: 0,
                checked: true,
                reason: "Uninstall entry still present".into(),
            });
        }
    }
    found
}

/// Folders go to the Recycle Bin; registry keys are backed up then deleted.
pub fn remove_leftovers(items: &[Leftover], guard: &Guard, backup_dir: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let folders: Vec<PathBuf> = items
        .iter()
        .filter(|l| l.kind == LeftoverKind::Folder)
        .map(|l| PathBuf::from(&l.path))
        .filter(|p| match guard.check_delete(p) {
            Ok(()) => true,
            Err(e) => {
                errors.push(e);
                false
            }
        })
        .collect();
    if let Err(e) = crate::sys::recycle(&folders) {
        errors.push(e);
    }
    for l in items.iter().filter(|l| l.kind == LeftoverKind::Registry) {
        if let Err(e) = delete_registry_path(&l.path, backup_dir) {
            errors.push(e);
        }
    }
    errors
}

#[cfg(windows)]
fn split_full_key(full: &str) -> Option<(crate::reg::Hive, &str)> {
    let (h, rest) = full.split_once('\\')?;
    let hive = match h {
        "HKLM" => crate::reg::Hive::Hklm,
        "HKCU" => crate::reg::Hive::Hkcu,
        _ => return None,
    };
    Some((hive, rest))
}

#[cfg(windows)]
pub fn key_exists(full: &str) -> bool {
    split_full_key(full).and_then(|(h, p)| crate::reg::open(h, p)).is_some()
}

#[cfg(not(windows))]
pub fn key_exists(_full: &str) -> bool {
    false
}

#[cfg(windows)]
pub fn delete_registry_path(full: &str, backup_dir: &Path) -> Result<(), String> {
    let (hive, path) = split_full_key(full).ok_or_else(|| format!("invalid key {full}"))?;
    // Refuse to delete top-level hives or bare Software keys.
    if path.matches('\\').count() < 1 {
        return Err(format!("refusing to delete {full}"));
    }
    crate::reg::delete_key_with_backup(hive, path, backup_dir)
}

#[cfg(not(windows))]
pub fn delete_registry_path(_full: &str, _backup_dir: &Path) -> Result<(), String> {
    Err("Windows only".into())
}

#[cfg(windows)]
pub fn list_apps() -> Vec<App> {
    use crate::reg::{dword, open, string, subkeys, Hive};
    const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
    const UNINSTALL32: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";
    let mut apps = Vec::new();
    let mut seen = HashSet::new();
    for (hive, base) in [(Hive::Hklm, UNINSTALL), (Hive::Hklm, UNINSTALL32), (Hive::Hkcu, UNINSTALL), (Hive::Hkcu, UNINSTALL32)] {
        let Some(root) = open(hive, base) else { continue };
        for sub in subkeys(&root) {
            let path = format!("{base}\\{sub}");
            let Some(k) = open(hive, &path) else { continue };
            let Some(name) = string(&k, "DisplayName") else { continue };
            let version = string(&k, "DisplayVersion").unwrap_or_default();
            if !seen.insert((name.to_lowercase(), version.clone())) {
                continue;
            }
            let uninstall = string(&k, "UninstallString").unwrap_or_default();
            let release = string(&k, "ReleaseType").unwrap_or_default().to_lowercase();
            let system = dword(&k, "SystemComponent") == Some(1)
                || string(&k, "ParentKeyName").is_some()
                || release.contains("update")
                || release.contains("hotfix")
                || uninstall.is_empty();
            let date = string(&k, "InstallDate").unwrap_or_default();
            let install_date = if date.len() == 8 && date.chars().all(|c| c.is_ascii_digit()) {
                format!("{}-{}-{}", &date[..4], &date[4..6], &date[6..])
            } else {
                String::new()
            };
            apps.push(App {
                name,
                publisher: string(&k, "Publisher").unwrap_or_default(),
                version,
                install_date,
                size: dword(&k, "EstimatedSize").unwrap_or(0) as u64 * 1024,
                uninstall,
                quiet_uninstall: string(&k, "QuietUninstallString"),
                install_location: string(&k, "InstallLocation"),
                reg_key: format!("{}\\{path}", hive.prefix()),
                system,
            });
        }
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

#[cfg(not(windows))]
pub fn list_apps() -> Vec<App> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str) -> App {
        App { name: name.into(), uninstall: "x".into(), ..Default::default() }
    }

    #[test]
    fn msi_commands_become_uninstall() {
        let a = App { uninstall: "MsiExec.exe /I{1234-ABCD}".into(), ..app("Thing") };
        let (prog, args) = build_command(&a, false).unwrap();
        assert!(prog.to_lowercase().ends_with("msiexec.exe"));
        assert_eq!(args, "/X{1234-ABCD}");
        let (_, quiet) = build_command(&a, true).unwrap();
        assert_eq!(quiet, "/X{1234-ABCD} /passive /norestart");
    }

    #[test]
    fn quiet_string_preferred() {
        let a = App { quiet_uninstall: Some("\"C:\\A\\u.exe\" /S".into()), uninstall: "\"C:\\A\\u.exe\"".into(), ..app("A") };
        assert_eq!(build_command(&a, true).unwrap(), ("C:\\A\\u.exe".into(), "/S".into()));
        assert_eq!(build_command(&a, false).unwrap(), ("C:\\A\\u.exe".into(), "".into()));
    }

    #[test]
    fn name_keys_strip_versions_and_arch() {
        assert!(app_keys(&app("7-Zip 23.01 (x64)")).contains(&"7zip".to_string()));
        let k = app_keys(&app("Notepad++ (64-bit x64)"));
        assert!(k.contains(&"notepad".to_string()), "{k:?}");
        let k = app_keys(&app("VLC media player 3.0.20"));
        assert!(k.contains(&"vlcmediaplayer".to_string()), "{k:?}");
        let mut a = app("Some Tool v2.1");
        a.install_location = Some("C:\\Program Files\\SomeTool".into());
        let k = app_keys(&a);
        assert!(k.contains(&"sometool".to_string()), "{k:?}");
    }

    #[test]
    fn leftovers_skip_shared_and_other_apps() {
        let root = std::env::temp_dir().join(format!("scx-left-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Coolapp/data")).unwrap();
        std::fs::create_dir_all(root.join("Otherapp")).unwrap();
        let mut gone = app("CoolApp");
        gone.install_location = Some(root.join("Coolapp").display().to_string());
        let mut other = app("OtherApp");
        other.install_location = Some(root.join("Otherapp").display().to_string());
        let found = find_leftovers(&gone, &[other], &Guard::default());
        assert!(found.iter().any(|l| l.path.ends_with("Coolapp") && l.checked));
        assert!(!found.iter().any(|l| l.path.ends_with("Otherapp")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
