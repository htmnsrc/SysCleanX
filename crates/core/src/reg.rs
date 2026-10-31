//! Thin registry helpers (Windows only).

use winreg::enums::*;
use winreg::{RegKey, HKCU, HKLM};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hive {
    Hklm,
    Hkcu,
}

impl Hive {
    pub fn key(self) -> &'static RegKey {
        match self {
            Hive::Hklm => HKLM,
            Hive::Hkcu => HKCU,
        }
    }
    pub fn prefix(self) -> &'static str {
        match self {
            Hive::Hklm => "HKLM",
            Hive::Hkcu => "HKCU",
        }
    }
}

/// Open for reading. HKLM paths are opened in the 64-bit view explicitly; use the
/// `WOW6432Node` path to read the 32-bit view.
pub fn open(hive: Hive, path: &str) -> Option<RegKey> {
    hive.key().open_subkey_with_flags(path, KEY_READ | KEY_WOW64_64KEY).ok()
}

pub fn open_rw(hive: Hive, path: &str) -> Option<RegKey> {
    hive.key().open_subkey_with_flags(path, KEY_READ | KEY_WRITE | KEY_WOW64_64KEY).ok()
}

pub fn string(key: &RegKey, name: &str) -> Option<String> {
    key.get_value::<String, _>(name).ok().map(|s| s.trim_matches('\0').trim().to_string()).filter(|s| !s.is_empty())
}

pub fn dword(key: &RegKey, name: &str) -> Option<u32> {
    key.get_value::<u32, _>(name).ok()
}

pub fn subkeys(key: &RegKey) -> Vec<String> {
    key.enum_keys().flatten().collect()
}

pub fn value_names(key: &RegKey) -> Vec<String> {
    key.enum_values().flatten().map(|(n, _)| n).collect()
}

/// Delete a whole key (with backup). `full` is e.g. `HKLM\SOFTWARE\X`.
pub fn delete_key_with_backup(hive: Hive, path: &str, backup_dir: &std::path::Path) -> Result<(), String> {
    let full = format!("{}\\{path}", hive.prefix());
    crate::sys::backup_registry_key(&full, backup_dir)?;
    let (parent, leaf) = path.rsplit_once('\\').ok_or("invalid key path")?;
    let parent = hive
        .key()
        .open_subkey_with_flags(parent, KEY_ALL_ACCESS | KEY_WOW64_64KEY)
        .map_err(|e| format!("{full}: {e}"))?;
    parent.delete_subkey_all(leaf).map_err(|e| format!("{full}: {e}"))
}

/// Delete one value (with backup of its key).
pub fn delete_value_with_backup(hive: Hive, path: &str, value: &str, backup_dir: &std::path::Path) -> Result<(), String> {
    let full = format!("{}\\{path}", hive.prefix());
    crate::sys::backup_registry_key(&full, backup_dir)?;
    let key = open_rw(hive, path).ok_or_else(|| format!("cannot open {full}"))?;
    key.delete_value(value).map_err(|e| format!("{full}\\{value}: {e}"))
}
