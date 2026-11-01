//! Broken leftovers: dead shortcuts, startup entries, uninstall entries, tasks and
//! services pointing at missing programs, plus orphaned AppData folders (review only).

use crate::fsops::size_of;
use crate::guard::Guard;
use crate::progress::Progress;
use crate::startup::{self, program_missing, Source, StartupItem};
use crate::uninstall::{app_keys, App};
use crate::util::{expand_env, norm_name};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Shortcut,
    StartupEntry,
    UninstallEntry,
    Task,
    Service,
    AppData,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Shortcut => "Broken shortcuts",
            Kind::StartupEntry => "Broken startup entries",
            Kind::UninstallEntry => "Orphaned uninstall entries",
            Kind::Task => "Orphaned scheduled tasks",
            Kind::Service => "Orphaned services",
            Kind::AppData => "Possibly orphaned app data (review carefully)",
        }
    }
    pub fn fix_label(self) -> &'static str {
        match self {
            Kind::Shortcut | Kind::AppData => "moved to Recycle Bin",
            Kind::StartupEntry | Kind::UninstallEntry | Kind::Service => "deleted (registry backed up)",
            Kind::Task => "deleted (task exported to XML)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub kind: Kind,
    pub title: String,
    pub detail: String,
    /// Path, registry key, service name or task path, depending on `kind`.
    pub target: String,
    pub size: u64,
    pub checked: bool,
    startup: Option<StartupItem>,
}

impl Finding {
    pub fn new(kind: Kind, title: String, detail: String, target: String, checked: bool) -> Self {
        Finding { kind, title, detail, target, size: 0, checked, startup: None }
    }
}

pub fn scan(apps: &[App], progress: &Progress) -> Vec<Finding> {
    let mut out = Vec::new();
    progress.set_label("Checking shortcuts…");
    out.extend(shortcuts());
    progress.set_label("Checking startup entries and tasks…");
    let startup_items = startup::list();
    for item in startup_items.iter().filter(|i| i.missing && i.source != Source::Task).cloned() {
        let mut f = Finding::new(Kind::StartupEntry, item.name.clone(), item.command.clone(), item.location.clone(), true);
        f.startup = Some(item);
        out.push(f);
    }
    out.extend(tasks());
    progress.set_label("Checking uninstall entries…");
    for app in apps {
        let (prog, _) = crate::util::split_command(&app.uninstall);
        let lower = prog.to_lowercase();
        // MSI entries, system components, Microsoft runtimes and installer bundles (whose
        // cached setup can be gone while the program is still installed) are never flagged.
        let protected = lower.contains("msiexec")
            || lower.contains("\\package cache\\")
            || app.system
            || app.publisher.to_lowercase().starts_with("microsoft");
        let location_exists = app.install_location.as_deref().map(|l| Path::new(l.trim_matches('"')).exists()).unwrap_or(false);
        if !app.uninstall.is_empty() && !protected && !location_exists && program_missing(&app.uninstall) {
            out.push(Finding::new(Kind::UninstallEntry, app.name.clone(), format!("Uninstaller missing: {prog}"), app.reg_key.clone(), true));
        }
    }
    progress.set_label("Checking services…");
    out.extend(services());
    progress.set_label("Looking for orphaned app data…");
    out.extend(orphaned_appdata(apps, &startup_items, progress));
    out
}

/// Apply fixes. Returns human-readable errors.
pub fn fix(findings: &[Finding], guard: &Guard, backup_dir: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let mut recycle = Vec::new();
    for f in findings {
        let r = match f.kind {
            Kind::Shortcut => {
                recycle.push(PathBuf::from(&f.target));
                Ok(())
            }
            Kind::AppData => match guard.check_delete(Path::new(&f.target)) {
                Ok(()) => {
                    recycle.push(PathBuf::from(&f.target));
                    Ok(())
                }
                Err(e) => Err(e),
            },
            Kind::StartupEntry => match &f.startup {
                Some(item) => startup::remove(item, backup_dir),
                None => Err("missing entry data".into()),
            },
            Kind::UninstallEntry => crate::uninstall::delete_registry_path(&f.target, backup_dir),
            Kind::Task => startup::remove(
                &StartupItem {
                    name: f.title.clone(),
                    command: f.detail.clone(),
                    source: Source::Task,
                    enabled: true,
                    missing: true,
                    location: f.target.clone(),
                },
                backup_dir,
            ),
            Kind::Service => delete_service(&f.target, backup_dir),
        };
        if let Err(e) = r {
            errors.push(format!("{}: {e}", f.title));
        }
    }
    if let Err(e) = crate::sys::recycle(&recycle) {
        errors.push(e);
    }
    errors
}

fn shortcuts() -> Vec<Finding> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let recursive: Vec<String> = [
        r"%APPDATA%\Microsoft\Windows\Start Menu",
        r"%ProgramData%\Microsoft\Windows\Start Menu",
        r"%APPDATA%\Microsoft\Internet Explorer\Quick Launch",
    ]
    .iter()
    .filter_map(|s| expand_env(s))
    .collect();
    let flat: Vec<String> = [r"%USERPROFILE%\Desktop", r"%PUBLIC%\Desktop"].iter().filter_map(|s| expand_env(s)).collect();
    let list = |v: &[String]| v.iter().map(|s| crate::sys::ps_quote(s)).collect::<Vec<_>>().join(",");
    let script = format!(
        r#"$sh = New-Object -ComObject WScript.Shell
$files = @()
foreach($r in @({rec})) {{ if(Test-Path -LiteralPath $r) {{ $files += Get-ChildItem -LiteralPath $r -Filter *.lnk -Recurse -File -ErrorAction SilentlyContinue }} }}
foreach($r in @({flat})) {{ if(Test-Path -LiteralPath $r) {{ $files += Get-ChildItem -LiteralPath $r -Filter *.lnk -File -ErrorAction SilentlyContinue }} }}
@($files | ForEach-Object {{ try {{ [pscustomobject]@{{ Lnk=$_.FullName; Target=$sh.CreateShortcut($_.FullName).TargetPath }} }} catch {{}} }}) | ConvertTo-Json -Compress"#,
        rec = list(&recursive),
        flat = list(&flat)
    );
    let Ok(out) = crate::sys::powershell(&script) else { return Vec::new() };
    let fixed: HashSet<String> = crate::sys::fixed_drives().iter().map(|d| d.display().to_string().to_lowercase()).collect();
    crate::sys::json_list(&out)
        .iter()
        .filter_map(|v| {
            let lnk = v["Lnk"].as_str()?;
            let target = v["Target"].as_str()?.trim();
            let tp = Path::new(target);
            let drive = target.get(..3)?.to_lowercase();
            if target.is_empty() || !tp.is_absolute() || !fixed.contains(&drive) || tp.exists() {
                return None;
            }
            let name = Path::new(lnk).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            Some(Finding::new(Kind::Shortcut, name, format!("Points to missing {target}"), lnk.to_string(), true))
        })
        .collect()
}

fn tasks() -> Vec<Finding> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let script = r#"Get-ScheduledTask | Where-Object { $_.TaskPath -notlike '\Microsoft\*' } | ForEach-Object {
  $t = $_; $a = @($t.Actions | Where-Object { $_.Execute }) | Select-Object -First 1
  if ($a) { [pscustomobject]@{ Name=$t.TaskName; Path=$t.TaskPath; Exec=('"' + $a.Execute.Trim('"') + '" ' + $a.Arguments).Trim() } }
} | ConvertTo-Json -Compress"#;
    let Ok(out) = crate::sys::powershell(script) else { return Vec::new() };
    crate::sys::json_list(&out)
        .iter()
        .filter_map(|v| {
            let exec = v["Exec"].as_str()?.to_string();
            if !program_missing(&exec) {
                return None;
            }
            let name = v["Name"].as_str()?.to_string();
            let location = format!("{}{}", v["Path"].as_str().unwrap_or("\\"), name);
            Some(Finding::new(Kind::Task, name, exec, location, true))
        })
        .collect()
}

#[cfg(windows)]
fn services() -> Vec<Finding> {
    use crate::reg::{dword, open, string, subkeys, Hive};
    const BASE: &str = r"SYSTEM\CurrentControlSet\Services";
    let Some(root) = open(Hive::Hklm, BASE) else { return Vec::new() };
    let windir = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()).to_lowercase();
    let mut out = Vec::new();
    for name in subkeys(&root) {
        let Some(k) = open(Hive::Hklm, &format!("{BASE}\\{name}")) else { continue };
        let ty = dword(&k, "Type").unwrap_or(0);
        if ty & 0x30 == 0 {
            continue; // drivers and other kernel-level entries are never touched
        }
        let Some(image) = string(&k, "ImagePath") else { continue };
        let lower = image.to_lowercase();
        if lower.contains("svchost") || lower.starts_with("\\systemroot") || lower.contains(&windir) || lower.contains("%systemroot%") || lower.contains("%windir%") {
            continue;
        }
        if program_missing(&image) {
            let display = string(&k, "DisplayName").unwrap_or_else(|| name.clone());
            out.push(Finding::new(Kind::Service, display, image, name.clone(), false));
        }
    }
    out
}

#[cfg(not(windows))]
fn services() -> Vec<Finding> {
    Vec::new()
}

fn delete_service(name: &str, backup_dir: &Path) -> Result<(), String> {
    crate::sys::backup_registry_key(&format!("HKLM\\SYSTEM\\CurrentControlSet\\Services\\{name}"), backup_dir)?;
    crate::sys::run_tool("sc.exe", &["delete", name]).map(|_| ())
}

/// Folder names in AppData/ProgramData that belong to Windows or shared tooling.
const KNOWN_DATA: &[&str] = &[
    "microsoft", "packages", "temp", "programs", "comms", "connecteddevicesplatform", "d3dscache",
    "crashdumps", "publishers", "peerdistrepub", "virtualstore", "nvidia", "nvidiacorporation", "amd",
    "intel", "adobe", "google", "mozilla", "packagecache", "softwaredistribution", "ssh", "usoprivate",
    "usoshared", "windowsholographicdevices", "applicationdata", "desktop", "documents", "startmenu",
    "templates", "identities", "sun", "oracle", "apple", "appleinc", "squirreltemp", "npm", "npmcache",
    "pip", "yarn", "nuget", "gobuild", "pnpm", "deno", "composer", "gradle", "electron", "crashpad",
    "fontconfig", "pki", "history", "default", "public", "media", "jedi", "pypa", "python", "node",
    "github", "githubdesktop", "jetbrains", "vstelemetry", "visualstudio", "programdata", "regid",
    "windowsapps", "windows", "system", "dbg", "symbols", "cache", "logs", "updates",
];

fn newest_mtime(dir: &Path) -> Option<SystemTime> {
    let mut newest = std::fs::metadata(dir).ok()?.modified().ok()?;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten().take(200) {
            if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                newest = newest.max(t);
            }
        }
    }
    Some(newest)
}

fn orphaned_appdata(apps: &[App], startup_items: &[StartupItem], progress: &Progress) -> Vec<Finding> {
    let mut known: Vec<String> = apps.iter().flat_map(app_keys).collect();
    known.extend(apps.iter().map(|a| norm_name(&a.publisher)).filter(|p| p.len() >= 4));
    known.extend(
        crate::sys::processes()
            .iter()
            .map(|p| norm_name(p.name.trim_end_matches(".exe")))
            .filter(|n| n.len() >= 4),
    );
    known.extend(startup_items.iter().map(|s| norm_name(&s.command)));
    let roots: Vec<PathBuf> = ["%APPDATA%", "%LOCALAPPDATA%", "%ProgramData%"]
        .iter()
        .filter_map(|s| expand_env(s))
        .map(PathBuf::from)
        .collect();
    let cutoff = SystemTime::now() - Duration::from_secs(90 * 24 * 3600);
    let mut out = Vec::new();
    for root in roots {
        let Ok(rd) = std::fs::read_dir(&root) else { continue };
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if !md.is_dir() || crate::paths::is_link(&md) {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let n = norm_name(&name);
            if n.len() < 4 || name.starts_with('.') || KNOWN_DATA.iter().any(|k| n == *k || n.starts_with("regid")) {
                continue;
            }
            let related = known.iter().any(|k| k.len() >= 4 && (k.contains(&n) || n.contains(k.as_str())));
            if related || newest_mtime(&e.path()).map(|t| t > cutoff).unwrap_or(true) {
                continue;
            }
            let size = size_of(&e.path(), progress).bytes;
            let mut f = Finding::new(
                Kind::AppData,
                name,
                "No installed program matches this folder".into(),
                e.path().display().to_string(),
                false,
            );
            f.size = size;
            out.push(f);
        }
    }
    out
}
