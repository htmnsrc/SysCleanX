//! Hard safety net: paths the engine must never delete, independent of target data.
//!
//! * `protected`: these folders can never be deleted, never be emptied wholesale, and no
//!   ancestor of them can be deleted/emptied. Filtered purges (e.g. only `*.pf`) inside
//!   them are allowed, as is emptying a *child* folder (e.g. `C:\Windows\Temp`).
//! * `sealed`: nothing at or inside these folders is ever deleted. The walker skips them.

use crate::util::{expand_env, key_within, norm_key};
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct Guard {
    protected: Vec<String>,
    sealed: Vec<String>,
    /// Windows folder (manual deletion never reaches inside it).
    system_root: Option<String>,
    /// Program Files, ProgramData, per-user Programs: manual deletion warns here.
    program_dirs: Vec<String>,
}

/// Well-known user folders that are sealed for every profile under `X:\Users\*`.
const SEALED_PROFILE_DIRS: &[&str] = &[
    "documents", "desktop", "pictures", "videos", "music", "downloads", "favorites",
    "contacts", "saved games", "links", "searches", "3d objects",
];

impl Guard {
    pub fn new(protected: Vec<String>, sealed: Vec<String>) -> Self {
        Guard { protected, sealed, system_root: None, program_dirs: Vec::new() }
    }

    /// Set the folders used by the manual-deletion rules.
    pub fn with_manual(mut self, system_root: Option<String>, program_dirs: Vec<String>) -> Self {
        self.system_root = system_root;
        self.program_dirs = program_dirs;
        self
    }

    /// Guard built from the current Windows environment.
    pub fn system() -> Self {
        let keys = |specs: &[&str]| -> Vec<String> {
            specs
                .iter()
                .filter_map(|s| expand_env(s))
                .map(|s| norm_key(Path::new(&s)))
                .collect()
        };
        let mut protected = keys(&[
            "%SystemRoot%",
            "%SystemRoot%\\System32",
            "%SystemRoot%\\SysWOW64",
            "%SystemRoot%\\WinSxS",
            "%SystemRoot%\\Logs",
            "%SystemRoot%\\Prefetch",
            "%SystemRoot%\\SoftwareDistribution",
            "%ProgramFiles%",
            "%ProgramFiles(x86)%",
            "%ProgramW6432%",
            "%ProgramData%",
            "%ProgramData%\\Microsoft",
            "%SystemDrive%\\Users",
            "%PUBLIC%",
            "%USERPROFILE%",
            "%USERPROFILE%\\AppData",
            "%USERPROFILE%\\AppData\\LocalLow",
            "%APPDATA%",
            "%APPDATA%\\Microsoft",
            "%LOCALAPPDATA%",
            "%LOCALAPPDATA%\\Microsoft",
            "%LOCALAPPDATA%\\Packages",
            "%LOCALAPPDATA%\\Programs",
            "%SystemDrive%\\Recovery",
            "%SystemDrive%\\Boot",
        ]);
        let mut sealed = keys(&[
            "%SystemRoot%\\System32",
            "%SystemRoot%\\SysWOW64",
            "%SystemRoot%\\WinSxS",
            "%SystemRoot%\\Installer",
            "%SystemRoot%\\servicing",
            "%SystemRoot%\\Boot",
            "%SystemRoot%\\Fonts",
            "%SystemRoot%\\assembly",
            "%SystemRoot%\\Microsoft.NET",
            "%SystemRoot%\\SystemApps",
            "%ProgramFiles%\\WindowsApps",
            "%SystemDrive%\\Recovery",
            "%SystemDrive%\\EFI",
            "%PUBLIC%\\Documents",
            "%PUBLIC%\\Desktop",
            "%OneDrive%",
            "%OneDriveConsumer%",
            "%OneDriveCommercial%",
        ]);
        if let Some(profile) = expand_env("%USERPROFILE%") {
            for d in SEALED_PROFILE_DIRS {
                sealed.push(norm_key(&Path::new(&profile).join(d)));
            }
        }
        protected.sort();
        protected.dedup();
        sealed.sort();
        sealed.dedup();
        let system_root = keys(&["%SystemRoot%"]).into_iter().next();
        let program_dirs = keys(&["%ProgramFiles%", "%ProgramFiles(x86)%", "%ProgramW6432%", "%ProgramData%", "%LOCALAPPDATA%\\Programs"]);
        Guard { protected, sealed, system_root, program_dirs }
    }

    /// True if nothing at or below this normalized key may be deleted.
    pub fn is_sealed(&self, key: &str) -> bool {
        if self.sealed.iter().any(|s| key_within(key, s)) {
            return true;
        }
        let parts: Vec<&str> = key.split('/').filter(|s| !s.is_empty()).collect();
        parts.len() >= 4
            && parts[0].ends_with(':')
            && parts[1] == "users"
            && (SEALED_PROFILE_DIRS.contains(&parts[3]) || parts[3].starts_with("onedrive"))
    }

    fn base_check(&self, path: &Path) -> Result<String, String> {
        let key = norm_key(path);
        if !path.is_absolute() {
            return Err(format!("not an absolute path: {}", path.display()));
        }
        if key.starts_with("//") {
            return Err(format!("network paths are never cleaned: {}", path.display()));
        }
        if path.parent().is_none() || key.ends_with(":/") || key.len() <= 3 {
            return Err(format!("refusing to touch a drive root: {}", path.display()));
        }
        if self.is_sealed(&key) {
            return Err(format!("sealed location: {}", path.display()));
        }
        Ok(key)
    }

    /// Ok if `path` itself may be deleted, or emptied of *all* its contents.
    pub fn check_delete(&self, path: &Path) -> Result<(), String> {
        let key = self.base_check(path)?;
        if self.protected.iter().any(|p| key_within(p, &key)) {
            return Err(format!("protected location: {}", path.display()));
        }
        let parts: Vec<&str> = key.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() >= 2 && parts[0].ends_with(':') && parts[1] == "users"
            && (parts.len() <= 3 || (parts.len() == 4 && parts[3] == "appdata")) {
                return Err(format!("protected user profile folder: {}", path.display()));
            }
        Ok(())
    }

    /// Rules for deletions the user picks by hand (Disk usage). Much wider than automatic
    /// cleaning (your own files anywhere are fine; they go to the Recycle Bin), but Windows
    /// itself and folders it depends on are refused. `Ok(Some(warning))` means allowed but
    /// worth a warning.
    pub fn check_manual(&self, path: &Path) -> Result<Option<&'static str>, String> {
        if !path.is_absolute() {
            return Err("Only full paths can be deleted.".into());
        }
        self.manual_rule(&norm_key(path))
    }

    pub fn manual_rule(&self, key: &str) -> Result<Option<&'static str>, String> {
        const ROOT_FILES: &[&str] = &["pagefile.sys", "hiberfil.sys", "swapfile.sys", "dumpstack.log", "dumpstack.log.tmp", "bootmgr", "bootnxt"];
        const SYSTEM_TOP: &[&str] = &["system volume information", "recovery", "boot", "efi", "config.msi", "windows.old"];
        if key.starts_with("//") {
            return Err("Network locations can't be deleted from here.".into());
        }
        let parts: Vec<&str> = key.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() <= 1 {
            return Err("A whole drive can't be deleted.".into());
        }
        if self.protected.iter().any(|p| key_within(p, key)) {
            return Err("Windows needs this folder, so it can't be deleted.".into());
        }
        if let Some(root) = &self.system_root {
            if key_within(key, root) {
                return Err("Files inside the Windows folder are managed by Windows.".into());
            }
        }
        let top = parts[1];
        if top.starts_with('$') || SYSTEM_TOP.contains(&top) {
            return Err(if top == "windows.old" {
                "Remove the previous Windows installation from Clean → Advanced instead.".into()
            } else {
                "This is a hidden Windows system folder.".into()
            });
        }
        if parts.len() == 2 && ROOT_FILES.contains(&top) {
            return Err(if top == "hiberfil.sys" {
                "Turn hibernation off in Tools → Repair to remove this file.".into()
            } else {
                "This file is managed by Windows.".into()
            });
        }
        if (top == "programdata" && parts.get(2) == Some(&"microsoft")) || parts.get(2) == Some(&"windowsapps") {
            return Err("This folder is managed by Windows.".into());
        }
        if top == "users" {
            if parts.len() == 3 {
                return Err("A whole user profile can't be deleted from here.".into());
            }
            if parts.len() == 4 && (SEALED_PROFILE_DIRS.contains(&parts[3]) || parts[3].starts_with("onedrive")) {
                return Err("Windows needs this folder. Open it and delete what's inside instead.".into());
            }
            if parts.get(3) == Some(&"appdata") {
                return Ok(Some("Apps keep their data here. Deleting it can reset or break the app that owns it."));
            }
        }
        if self.program_dirs.iter().any(|d| key_within(key, d)) {
            return Ok(Some("This belongs to an installed program. Uninstalling it from Apps → Programs is safer than deleting its files."));
        }
        Ok(None)
    }

    /// Ok if the engine may purge inside `path`. `filtered` purges (name patterns,
    /// never removing folders) may run inside protected folders; full purges may not.
    pub fn check_purge(&self, path: &Path, filtered: bool) -> Result<(), String> {
        if filtered {
            self.base_check(path).map(|_| ())
        } else {
            self.check_delete(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_deletion_rules() {
        let g = Guard::new(
            vec!["c:/windows".into(), "c:/program files".into(), "c:/programdata".into(), "c:/users".into(), "c:/users/bob".into()],
            vec![],
        )
        .with_manual(Some("c:/windows".into()), vec!["c:/program files".into(), "c:/programdata".into()]);
        let blocked = [
            "c:/", "c:/windows", "c:/windows/temp/x.log", "c:/program files", "c:/users", "c:/users/bob", "c:/users/amy",
            "c:/users/bob/documents", "c:/users/bob/onedrive - work", "c:/pagefile.sys", "c:/hiberfil.sys",
            "c:/$recycle.bin/s-1-5/x", "c:/system volume information", "c:/programdata/microsoft/windows defender",
            "c:/program files/windowsapps/x", "c:/windows.old", "//server/share/x",
        ];
        for k in blocked {
            assert!(g.manual_rule(k).is_err(), "{k} must be blocked");
        }
        assert_eq!(g.manual_rule("c:/users/bob/documents/holiday.mp4"), Ok(None));
        assert_eq!(g.manual_rule("c:/users/bob/downloads/old-installers"), Ok(None));
        assert_eq!(g.manual_rule("d:/games/old game"), Ok(None));
        assert!(matches!(g.manual_rule("c:/program files/someapp"), Ok(Some(_))));
        assert!(matches!(g.manual_rule("c:/users/bob/appdata/local/someapp"), Ok(Some(_))));
    }

    #[test]
    fn blocks_dangerous_paths() {
        let (w, s32, users, lad) = if cfg!(windows) {
            ("C:\\Windows", "C:\\Windows\\System32", "C:\\Users", "C:\\Users\\Bob\\AppData\\Local")
        } else {
            ("/c/windows", "/c/windows/system32", "/c/users", "/c/users/bob/appdata/local")
        };
        let k = |s: &str| norm_key(Path::new(s));
        let g = Guard::new(vec![k(w), k(s32), k(users), k(lad)], vec![k(s32)]);
        let p = |s: &str| -> std::path::PathBuf {
            if cfg!(windows) { s.into() } else { format!("/c/{}", s.replace('\\', "/").to_lowercase()).into() }
        };
        assert!(g.check_delete(Path::new(if cfg!(windows) { "C:\\" } else { "/" })).is_err());
        assert!(g.check_delete(&p("Windows")).is_err());
        assert!(g.check_delete(&p("Windows\\System32\\drivers")).is_err());
        assert!(g.check_purge(&p("Windows\\System32\\x"), true).is_err());
        assert!(g.check_delete(&p("Windows\\Temp")).is_ok());
        assert!(g.check_delete(&p("Users\\Bob\\AppData\\Local")).is_err());
        assert!(g.check_purge(&p("Users\\Bob\\AppData\\Local"), true).is_ok());
        assert!(g.check_delete(&p("Users\\Bob\\AppData\\Local\\Temp")).is_ok());
        assert!(g.check_delete(Path::new("relative/path")).is_err());
        if cfg!(windows) {
            assert!(g.check_delete(&p("Users\\Bob")).is_err());
            assert!(g.check_delete(&p("Users\\Bob\\Documents\\x")).is_err());
            assert!(g.check_delete(&p("Users\\Bob\\OneDrive - Work")).is_err());
        }
    }
}
