//! Platform layer. Real implementations live in `win.rs`; `stub.rs` keeps the crate
//! compiling (and the engine testable) on other systems.

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::*;

#[cfg(not(windows))]
mod stub;
#[cfg(not(windows))]
pub use stub::*;

use crate::paths::system_tool;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct ProcInfo {
    pub pid: u32,
    pub parent: u32,
    /// Lowercase executable name, e.g. `chrome.exe`.
    pub name: String,
}

/// Lowercase names of running processes.
pub fn process_names() -> std::collections::HashSet<String> {
    processes().into_iter().map(|p| p.name).collect()
}

/// A `Command` that never flashes a console window.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Run a program, returning stdout on success or a readable error.
pub fn run(program: &Path, args: &[&str]) -> Result<String, String> {
    let out = command(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{}: {e}", program.display()))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        Ok(stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = if err.trim().is_empty() { stdout.trim() } else { err.trim() };
        Err(format!("{} failed ({}): {}", program.display(), out.status, msg.lines().take(3).collect::<Vec<_>>().join(" ")))
    }
}

/// Spawn a command and wait for it *and every process it started* to finish.
/// Returns the exit code of the original process.
pub fn run_wait_tree(mut cmd: Command, max: std::time::Duration) -> Result<Option<i32>, String> {
    let mut child = cmd.stdin(Stdio::null()).spawn().map_err(|e| e.to_string())?;
    let pid = child.id();
    let status = child.wait().map_err(|e| e.to_string())?;
    let start = std::time::Instant::now();
    let mut family = std::collections::HashSet::new();
    while process_tree_alive(pid, &mut family) && start.elapsed() < max {
        std::thread::sleep(std::time::Duration::from_millis(700));
    }
    Ok(status.code())
}

/// Run a Windows system tool from System32 by name.
pub fn run_tool(name: &str, args: &[&str]) -> Result<String, String> {
    run(&system_tool(name), args)
}

/// Run a PowerShell script (UTF-8 output). The script is passed base64-encoded so no
/// quoting issues can occur.
pub fn powershell(script: &str) -> Result<String, String> {
    let full = format!(
        "$ProgressPreference='SilentlyContinue';[Console]::OutputEncoding=[Text.Encoding]::UTF8;\n{script}"
    );
    let utf16: Vec<u8> = full.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let encoded = crate::util::base64(&utf16);
    let ps = system_tool("WindowsPowerShell\\v1.0\\powershell.exe");
    run(&ps, &["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &encoded])
}

/// Quote a string as a PowerShell single-quoted literal.
pub fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Parse PowerShell `ConvertTo-Json` output that may be a single object or an array.
pub fn json_list(out: &str) -> Vec<serde_json::Value> {
    match serde_json::from_str::<serde_json::Value>(out.trim()) {
        Ok(serde_json::Value::Array(v)) => v,
        Ok(serde_json::Value::Null) | Err(_) => Vec::new(),
        Ok(v) => vec![v],
    }
}

pub fn time_stamp() -> String {
    let [y, mo, d, h, mi, s] = local_time();
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

pub fn file_stamp() -> String {
    let [y, mo, d, h, mi, s] = local_time();
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// UTC time split into fields (fallback when no local clock API is available).
#[allow(dead_code)]
fn utc_time() -> [u16; 6] {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    crate::util::civil(secs)
}

/// Create a System Restore point. Fails if System Protection is off.
pub fn create_restore_point(description: &str) -> Result<(), String> {
    let script = format!(
        "$ErrorActionPreference='Stop'; Checkpoint-Computer -Description {} -RestorePointType 'MODIFY_SETTINGS'",
        ps_quote(description)
    );
    powershell(&script).map(|_| ())
}

/// Export a registry key to a .reg backup file before modifying it.
pub fn backup_registry_key(key: &str, backup_dir: &Path) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(backup_dir).map_err(|e| e.to_string())?;
    let safe: String = key
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .chars()
        .rev()
        .take(60)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let file = backup_dir.join(format!("{}-{safe}.reg", file_stamp()));
    let file_s = file.to_string_lossy().into_owned();
    run_tool("reg.exe", &["export", key, &file_s, "/y"])?;
    Ok(file)
}

/// Task created by older builds (weekly CLI clean). The CLI no longer exists.
const LEGACY_SCHEDULE_TASK: &str = "SysCleanX\\Weekly Clean";

/// Remove the weekly-clean task left by older builds so Windows doesn't keep trying to run
/// a program that no longer exists. Needs admin; silently does nothing otherwise.
pub fn remove_legacy_schedule() {
    if cfg!(windows) && run_tool("schtasks.exe", &["/Query", "/TN", LEGACY_SCHEDULE_TASK]).is_ok() {
        let _ = run_tool("schtasks.exe", &["/Delete", "/TN", LEGACY_SCHEDULE_TASK, "/F"]);
    }
}

/// Hibernation file size if hibernation is enabled.
pub fn hiberfil_size() -> Option<u64> {
    let drive = std::env::var("SystemDrive").ok()?;
    std::fs::metadata(format!("{drive}\\hiberfil.sys")).ok().map(|m| m.len())
}

pub fn set_hibernation(enabled: bool) -> Result<(), String> {
    run_tool("powercfg.exe", &["/hibernate", if enabled { "on" } else { "off" }]).map(|_| ())
}

#[cfg(test)]
mod tests {
    #[test]
    fn json_list_handles_shapes() {
        assert_eq!(super::json_list("[1,2]").len(), 2);
        assert_eq!(super::json_list("{\"a\":1}").len(), 1);
        assert_eq!(super::json_list("").len(), 0);
    }

    #[test]
    fn utc_time_is_sane() {
        let t = super::utc_time();
        assert!(t[0] >= 2024 && (1..=12).contains(&t[1]) && (1..=31).contains(&t[2]));
    }
}
