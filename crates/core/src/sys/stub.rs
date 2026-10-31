//! Non-Windows stand-ins so the engine and UI can be developed and tested anywhere.

use super::ProcInfo;
use std::path::{Path, PathBuf};

const UNSUPPORTED: &str = "only available on Windows";

pub fn is_admin() -> bool {
    false
}
pub fn relaunch_elevated(_args: &str) -> bool {
    false
}
pub fn processes() -> Vec<ProcInfo> {
    Vec::new()
}
pub fn process_tree_alive(_root: u32, _family: &mut std::collections::HashSet<u32>) -> bool {
    false
}
pub fn stop_service(_name: &str) -> Result<bool, String> {
    Err(UNSUPPORTED.into())
}
pub fn start_service(_name: &str) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}
pub fn recycle_bin_query() -> Result<(u64, u64), String> {
    Err(UNSUPPORTED.into())
}
pub fn recycle_bin_empty() -> Result<(), String> {
    Err(UNSUPPORTED.into())
}
pub fn disk_space(_path: &Path) -> Option<(u64, u64)> {
    None
}
pub fn free_space(_path: &Path) -> Option<u64> {
    None
}
pub fn delete_on_reboot(_path: &Path) -> bool {
    false
}
pub fn recycle(_paths: &[PathBuf]) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}
pub fn local_time() -> [u16; 6] {
    super::utc_time()
}
pub fn fixed_drives() -> Vec<PathBuf> {
    vec![PathBuf::from("/")]
}
pub fn reveal(path: &Path) {
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
}
pub fn open(path: &Path) {
    let _ = std::process::Command::new("open").arg(path).spawn();
}
