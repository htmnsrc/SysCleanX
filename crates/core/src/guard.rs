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
}

/// Well-known user folders that are sealed for every profile under `X:\Users\*`.
const SEALED_PROFILE_DIRS: &[&str] = &[
    "documents", "desktop", "pictures", "videos", "music", "downloads", "favorites",
    "contacts", "saved games", "links", "searches", "3d objects",
];

impl Guard {
    pub fn new(protected: Vec<String>, sealed: Vec<String>) -> Self {
        Guard { protected, sealed }
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
        Guard { protected, sealed }
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
