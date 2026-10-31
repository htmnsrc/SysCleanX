//! Windows implementations (Win32 via windows-sys).

use super::ProcInfo;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, SYSTEMTIME};
use windows_sys::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, MoveFileExW, MOVEFILE_DELAY_UNTIL_REBOOT,
};
use windows_sys::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, ControlService, OpenSCManagerW, OpenServiceW, QueryServiceStatus, StartServiceW,
    SC_MANAGER_CONNECT, SERVICE_CONTROL_STOP, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_START, SERVICE_STATUS,
    SERVICE_STOP, SERVICE_STOPPED, SERVICE_STOP_PENDING,
};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::UI::Shell::{
    IsUserAnAdmin, SHEmptyRecycleBinW, SHFileOperationW, SHQueryRecycleBinW, ShellExecuteW, FOF_ALLOWUNDO,
    FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI,
    SHERB_NOSOUND, SHFILEOPSTRUCTW, SHQUERYRBINFO,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const DRIVE_FIXED: u32 = 3;

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

pub fn is_admin() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}

/// Relaunch the current exe elevated via UAC. Returns true if the new process started.
pub fn relaunch_elevated(args: &str) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let verb = wide("runas");
    let file = wide(exe.as_os_str());
    let params = wide(args);
    let r = unsafe { ShellExecuteW(null_mut(), verb.as_ptr(), file.as_ptr(), params.as_ptr(), null(), SW_SHOWNORMAL) };
    r as usize > 32
}

pub fn processes() -> Vec<ProcInfo> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            out.push(ProcInfo {
                pid: e.th32ProcessID,
                parent: e.th32ParentProcessID,
                name: String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase(),
            });
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
    }
    out
}

/// True while any descendant of the (already exited) process `root` is alive. `family`
/// accumulates PIDs across calls so grandchildren of exited parents are still recognised
/// (uninstallers often copy themselves to %TEMP%, relaunch and exit).
pub fn process_tree_alive(root: u32, family: &mut std::collections::HashSet<u32>) -> bool {
    let procs = processes();
    family.insert(root);
    loop {
        let before = family.len();
        for p in &procs {
            if family.contains(&p.parent) {
                family.insert(p.pid);
            }
        }
        if family.len() == before {
            break;
        }
    }
    // The root PID itself may have been reused by an unrelated process; ignore it.
    procs.iter().any(|p| p.pid != root && family.contains(&p.pid))
}

struct ScHandle(windows_sys::Win32::System::Services::SC_HANDLE);
impl Drop for ScHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseServiceHandle(self.0) };
        }
    }
}

fn open_service(name: &str, access: u32) -> Result<(ScHandle, ScHandle), String> {
    unsafe {
        let scm = ScHandle(OpenSCManagerW(null(), null(), SC_MANAGER_CONNECT));
        if scm.0.is_null() {
            return Err("cannot open service manager".into());
        }
        let n = wide(name);
        let svc = ScHandle(OpenServiceW(scm.0, n.as_ptr(), access));
        if svc.0.is_null() {
            return Err(format!("cannot open service {name}"));
        }
        Ok((scm, svc))
    }
}

fn service_state(svc: &ScHandle) -> Option<u32> {
    unsafe {
        let mut st: SERVICE_STATUS = std::mem::zeroed();
        (QueryServiceStatus(svc.0, &mut st) != 0).then_some(st.dwCurrentState)
    }
}

/// Stop a service and wait for it. Returns whether it was running before.
pub fn stop_service(name: &str) -> Result<bool, String> {
    let (_scm, svc) = open_service(name, SERVICE_QUERY_STATUS | SERVICE_STOP)?;
    if service_state(&svc) == Some(SERVICE_STOPPED) {
        return Ok(false);
    }
    let accepted = unsafe {
        let mut st: SERVICE_STATUS = std::mem::zeroed();
        ControlService(svc.0, SERVICE_CONTROL_STOP, &mut st) != 0
    };
    if !accepted && service_state(&svc) != Some(SERVICE_STOP_PENDING) {
        return Err(format!("Windows refused to stop service {name}"));
    }
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(30) {
        if service_state(&svc) == Some(SERVICE_STOPPED) {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!("service {name} did not stop in time"))
}

pub fn start_service(name: &str) -> Result<(), String> {
    let (_scm, svc) = open_service(name, SERVICE_QUERY_STATUS | SERVICE_START)?;
    if service_state(&svc) == Some(SERVICE_RUNNING) {
        return Ok(());
    }
    if unsafe { StartServiceW(svc.0, 0, null()) } == 0 {
        return Err(format!("could not restart service {name}"));
    }
    Ok(())
}

/// (bytes, items) in the Recycle Bin across all drives.
pub fn recycle_bin_query() -> Result<(u64, u64), String> {
    unsafe {
        let mut info: SHQUERYRBINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHQUERYRBINFO>() as u32;
        let hr = SHQueryRecycleBinW(null(), &mut info);
        if hr < 0 {
            return Err(format!("SHQueryRecycleBin failed: 0x{hr:08x}"));
        }
        Ok((info.i64Size.max(0) as u64, info.i64NumItems.max(0) as u64))
    }
}

pub fn recycle_bin_empty() -> Result<(), String> {
    let hr = unsafe { SHEmptyRecycleBinW(null_mut(), null(), SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND) };
    // E_UNEXPECTED is returned when the bin is already empty.
    if hr >= 0 || hr as u32 == 0x8000_FFFF {
        Ok(())
    } else {
        Err(format!("SHEmptyRecycleBin failed: 0x{hr:08x}"))
    }
}

/// (free, total) bytes of the volume holding `path`.
pub fn disk_space(path: &Path) -> Option<(u64, u64)> {
    let p = wide(path.as_os_str());
    let (mut avail, mut total) = (0u64, 0u64);
    let ok = unsafe { GetDiskFreeSpaceExW(p.as_ptr(), &mut avail, &mut total, null_mut()) };
    (ok != 0 && total > 0).then_some((avail, total))
}

pub fn free_space(path: &Path) -> Option<u64> {
    let p = wide(path.as_os_str());
    let mut avail = 0u64;
    let ok = unsafe { GetDiskFreeSpaceExW(p.as_ptr(), &mut avail, null_mut(), null_mut()) };
    (ok != 0).then_some(avail)
}

pub fn delete_on_reboot(path: &Path) -> bool {
    let p = wide(path.as_os_str());
    unsafe { MoveFileExW(p.as_ptr(), null(), MOVEFILE_DELAY_UNTIL_REBOOT) != 0 }
}

/// Send files/folders to the Recycle Bin.
pub fn recycle(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut from: Vec<u16> = Vec::new();
    for p in paths {
        from.extend(p.as_os_str().encode_wide());
        from.push(0);
    }
    from.push(0);
    // Shell operations expect an initialised (STA) COM apartment on the calling thread.
    let com = unsafe { CoInitializeEx(std::ptr::null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32) };
    let mut op: SHFILEOPSTRUCTW = unsafe { std::mem::zeroed() };
    op.wFunc = FO_DELETE;
    op.pFrom = from.as_ptr();
    // FOF_WANTNUKEWARNING: if an item cannot be recycled (USB/network drive), Windows asks
    // before deleting it permanently instead of silently destroying it.
    const FOF_WANTNUKEWARNING: u32 = 0x4000;
    op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING) as u16;
    let r = unsafe { SHFileOperationW(&mut op) };
    if com >= 0 {
        unsafe { CoUninitialize() };
    }
    if r != 0 {
        return Err(format!("could not move to Recycle Bin (code {r})"));
    }
    if op.fAnyOperationsAborted != 0 {
        return Err("operation was aborted".into());
    }
    Ok(())
}

pub fn local_time() -> [u16; 6] {
    unsafe {
        let mut t: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut t);
        [t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond]
    }
}

pub fn fixed_drives() -> Vec<PathBuf> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| PathBuf::from(format!("{}:\\", (b'A' + i) as char)))
        .filter(|p| unsafe { GetDriveTypeW(wide(p.as_os_str()).as_ptr()) } == DRIVE_FIXED)
        .collect()
}

/// Open Explorer with the item selected (or open a folder).
pub fn reveal(path: &Path) {
    use std::os::windows::process::CommandExt;
    let explorer = std::env::var("SystemRoot").map(|r| PathBuf::from(r).join("explorer.exe")).unwrap_or_else(|_| "explorer.exe".into());
    let _ = std::process::Command::new(explorer).raw_arg(format!("/select,\"{}\"", path.display())).spawn();
}

pub fn open(path: &Path) {
    let explorer = std::env::var("SystemRoot").map(|r| PathBuf::from(r).join("explorer.exe")).unwrap_or_else(|_| "explorer.exe".into());
    let _ = std::process::Command::new(explorer).arg(path).spawn();
}
