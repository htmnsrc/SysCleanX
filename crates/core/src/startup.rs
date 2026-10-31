//! Startup programs: Run keys, Startup folders and logon/boot scheduled tasks.
//! Disabling works the same way as Task Manager (StartupApproved), so it is reversible.

use crate::util::{expand_env, split_command};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    RunUser,
    RunMachine,
    RunMachine32,
    FolderUser,
    FolderCommon,
    Task,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::RunUser => "Registry (user)",
            Source::RunMachine => "Registry (all users)",
            Source::RunMachine32 => "Registry (all users, 32-bit)",
            Source::FolderUser => "Startup folder (user)",
            Source::FolderCommon => "Startup folder (all users)",
            Source::Task => "Scheduled task",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StartupItem {
    pub name: String,
    pub command: String,
    pub source: Source,
    pub enabled: bool,
    /// The program it starts no longer exists.
    pub missing: bool,
    /// File path for folder items, task path for tasks.
    pub location: String,
}

/// True when the command clearly points at a program that no longer exists.
/// Unknown/relative programs are never reported as missing.
pub fn program_missing(command: &str) -> bool {
    let Some(expanded) = expand_env(command) else { return false };
    let (prog, _) = split_command(&expanded);
    let p = Path::new(prog.trim());
    if !p.is_absolute() {
        return false;
    }
    let lower = prog.to_lowercase();
    if lower.ends_with("rundll32.exe") || lower.contains("\\windowsapps\\") {
        return false;
    }
    // Only judge local drives that are actually present.
    match p.components().next() {
        Some(std::path::Component::Prefix(pre)) => {
            let root = PathBuf::from(format!("{}\\", pre.as_os_str().to_string_lossy()));
            root.exists() && !p.exists()
        }
        _ => !p.exists(),
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::reg::{open, open_rw, Hive};
    use std::borrow::Cow;
    use winreg::enums::*;
    use winreg::RegValue;

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN32: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
    const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

    fn run_location(source: Source) -> Option<(Hive, &'static str, &'static str)> {
        match source {
            Source::RunUser => Some((Hive::Hkcu, RUN, "Run")),
            Source::RunMachine => Some((Hive::Hklm, RUN, "Run")),
            Source::RunMachine32 => Some((Hive::Hklm, RUN32, "Run32")),
            Source::FolderUser => Some((Hive::Hkcu, "", "StartupFolder")),
            Source::FolderCommon => Some((Hive::Hklm, "", "StartupFolder")),
            Source::Task => None,
        }
    }

    fn approved(hive: Hive, sub: &str, name: &str) -> bool {
        open(hive, &format!("{APPROVED}\\{sub}"))
            .and_then(|k| k.get_raw_value(name).ok())
            .map(|v| v.bytes.first().map(|b| b & 1 == 0).unwrap_or(true))
            .unwrap_or(true)
    }

    pub fn startup_folders() -> Vec<(Source, PathBuf)> {
        let mut v = Vec::new();
        if let Some(p) = expand_env(r"%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup") {
            v.push((Source::FolderUser, PathBuf::from(p)));
        }
        if let Some(p) = expand_env(r"%ProgramData%\Microsoft\Windows\Start Menu\Programs\StartUp") {
            v.push((Source::FolderCommon, PathBuf::from(p)));
        }
        v
    }

    pub fn list() -> Vec<StartupItem> {
        let mut items = Vec::new();
        for source in [Source::RunUser, Source::RunMachine, Source::RunMachine32] {
            let (hive, path, sub) = run_location(source).unwrap();
            let Some(key) = open(hive, path) else { continue };
            for (name, _) in key.enum_values().flatten() {
                let Ok(command) = key.get_value::<String, _>(&name) else { continue };
                if name.is_empty() {
                    continue;
                }
                items.push(StartupItem {
                    enabled: approved(hive, sub, &name),
                    missing: program_missing(&command),
                    location: format!("{}\\{path}", hive.prefix()),
                    name,
                    command,
                    source,
                });
            }
        }
        for (source, dir) in startup_folders() {
            let (hive, _, sub) = run_location(source).unwrap();
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.eq_ignore_ascii_case("desktop.ini") || e.path().is_dir() {
                    continue;
                }
                items.push(StartupItem {
                    enabled: approved(hive, sub, &name),
                    missing: false,
                    command: e.path().display().to_string(),
                    location: e.path().display().to_string(),
                    name,
                    source,
                });
            }
        }
        items.extend(tasks());
        items
    }

    fn tasks() -> Vec<StartupItem> {
        let script = r#"Get-ScheduledTask | Where-Object { $_.TaskPath -notlike '\Microsoft\*' } | ForEach-Object {
  $t = $_
  $trig = @($t.Triggers | ForEach-Object { $_.CimClass.CimClassName }) -join ','
  if ($trig -match 'Logon|Boot') {
    $a = @($t.Actions | Where-Object { $_.Execute })
    [pscustomobject]@{ Name=$t.TaskName; Path=$t.TaskPath; State=[string]$t.State;
      Exec=(@($a | ForEach-Object { ('"' + $_.Execute.Trim('"') + '" ' + $_.Arguments).Trim() }) -join ' ; ') }
  }
} | ConvertTo-Json -Compress"#;
        let Ok(out) = crate::sys::powershell(script) else { return Vec::new() };
        crate::sys::json_list(&out)
            .iter()
            .map(|v| {
                let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
                let exec = s("Exec");
                StartupItem {
                    name: s("Name"),
                    missing: !exec.is_empty() && program_missing(&exec),
                    command: exec,
                    source: Source::Task,
                    enabled: s("State") != "Disabled",
                    location: format!("{}{}", s("Path"), s("Name")),
                }
            })
            .collect()
    }

    fn filetime_now() -> [u8; 8] {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        ((secs + 11_644_473_600) * 10_000_000).to_le_bytes()
    }

    pub fn set_enabled(item: &StartupItem, enabled: bool) -> Result<(), String> {
        if item.source == Source::Task {
            let flag = if enabled { "/ENABLE" } else { "/DISABLE" };
            return crate::sys::run_tool("schtasks.exe", &["/Change", "/TN", &item.location, flag]).map(|_| ());
        }
        let (hive, _, sub) = run_location(item.source).ok_or("unsupported")?;
        let key = match open_rw(hive, &format!("{APPROVED}\\{sub}")) {
            Some(k) => k,
            None => hive
                .key()
                .create_subkey_with_flags(format!("{APPROVED}\\{sub}"), KEY_READ | KEY_WRITE | KEY_WOW64_64KEY)
                .map(|(k, _)| k)
                .map_err(|e| e.to_string())?,
        };
        let mut bytes = vec![if enabled { 2u8 } else { 3u8 }, 0, 0, 0];
        bytes.extend(if enabled { [0u8; 8] } else { filetime_now() });
        key.set_raw_value(&item.name, &RegValue { bytes: Cow::Owned(bytes), vtype: REG_BINARY })
            .map_err(|e| format!("{}: {e}", item.name))
    }

    /// Delete the entry itself (used for broken entries). Registry values are backed up;
    /// folder shortcuts go to the Recycle Bin; tasks are exported to XML first.
    pub fn remove(item: &StartupItem, backup_dir: &Path) -> Result<(), String> {
        match item.source {
            Source::RunUser | Source::RunMachine | Source::RunMachine32 => {
                let (hive, path, _) = run_location(item.source).unwrap();
                crate::reg::delete_value_with_backup(hive, path, &item.name, backup_dir)
            }
            Source::FolderUser | Source::FolderCommon => crate::sys::recycle(&[PathBuf::from(&item.location)]),
            Source::Task => {
                let xml = crate::sys::run_tool("schtasks.exe", &["/Query", "/TN", &item.location, "/XML"])?;
                std::fs::create_dir_all(backup_dir).map_err(|e| e.to_string())?;
                let safe: String = item.name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
                let file = backup_dir.join(format!("{}-task-{safe}.xml", crate::sys::file_stamp()));
                std::fs::write(file, xml).map_err(|e| e.to_string())?;
                crate::sys::run_tool("schtasks.exe", &["/Delete", "/TN", &item.location, "/F"]).map(|_| ())
            }
        }
    }
}

#[cfg(windows)]
pub use imp::{list, remove, set_enabled, startup_folders};

#[cfg(not(windows))]
pub fn list() -> Vec<StartupItem> {
    Vec::new()
}
#[cfg(not(windows))]
pub fn set_enabled(_item: &StartupItem, _enabled: bool) -> Result<(), String> {
    Err("Windows only".into())
}
#[cfg(not(windows))]
pub fn remove(_item: &StartupItem, _backup_dir: &Path) -> Result<(), String> {
    Err("Windows only".into())
}
#[cfg(not(windows))]
pub fn startup_folders() -> Vec<(Source, PathBuf)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    #[test]
    fn missing_detection_is_conservative() {
        assert!(!super::program_missing("notepad.exe"));
        assert!(!super::program_missing("%SCX_UNSET%\\x.exe"));
        if cfg!(not(windows)) {
            assert!(super::program_missing("\"/definitely/not/here/app\" --min"));
            assert!(!super::program_missing("/bin/sh -c x"));
        }
    }
}
