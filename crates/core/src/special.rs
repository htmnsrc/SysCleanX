//! Targets that need Windows APIs or tools instead of plain file deletion.

use crate::fsops::{size_of, Tally};
use crate::log::DeletionLog;
use crate::paths::resolve;
use crate::progress::Progress;
use crate::sys;
use crate::targets::{Special, Target};
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct SpecialScan {
    pub bytes: u64,
    pub count: u64,
    /// False when `bytes` is a rough estimate or unknown.
    pub size_known: bool,
    /// Something is there to clean even if the size could not be measured.
    pub present: bool,
    pub details: Vec<(String, u64)>,
    pub note: Option<String>,
}

fn system_drive() -> PathBuf {
    PathBuf::from(format!("{}\\", std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into())))
}

/// Run `f` and report the free space it recovered on the system drive.
fn measure_free<F: FnOnce() -> Result<(), String>>(f: F) -> Result<u64, String> {
    let drive = system_drive();
    let before = sys::free_space(&drive);
    f()?;
    let after = sys::free_space(&drive);
    Ok(match (before, after) {
        (Some(b), Some(a)) => a.saturating_sub(b),
        _ => 0,
    })
}

pub fn scan(t: &Target, special: Special, progress: &Progress) -> SpecialScan {
    if !cfg!(windows) {
        return SpecialScan { note: Some("Windows only".into()), ..Default::default() };
    }
    let r = match special {
        Special::RecycleBin => sys::recycle_bin_query().map(|(bytes, count)| SpecialScan {
            bytes,
            count,
            size_known: true,
            present: count > 0,
            ..Default::default()
        }),
        Special::ExplorerMru => Ok(mru::scan()),
        Special::Cleanmgr => {
            let mut s = SpecialScan { size_known: true, ..Default::default() };
            for spec in &t.paths {
                for p in resolve(spec) {
                    let tally = size_of(&p, progress);
                    let unreadable_or_nonempty = std::fs::read_dir(&p).map(|mut r| r.next().is_some()).unwrap_or(true);
                    if tally.files > 0 || unreadable_or_nonempty {
                        s.present = true;
                    }
                    if tally.files == 0 && unreadable_or_nonempty {
                        s.size_known = false;
                    }
                    s.bytes += tally.bytes;
                    s.count += tally.files;
                    s.details.push((p.display().to_string(), tally.bytes));
                }
            }
            Ok(s)
        }
        Special::Dism => Ok(SpecialScan {
            present: true,
            note: Some("Size is only known after cleanup (analysis takes minutes)".into()),
            ..Default::default()
        }),
        Special::EventLogs => {
            let dir = crate::paths::system_tool("winevt\\Logs");
            let tally = size_of(&dir, progress);
            Ok(SpecialScan { bytes: tally.bytes, count: tally.files, size_known: true, present: tally.files > 0, ..Default::default() })
        }
        Special::RestorePoints => restore_points::scan(),
        Special::InstallerOrphans => installer::find_orphans().map(|orphans| SpecialScan {
            bytes: orphans.iter().map(|o| o.1).sum(),
            count: orphans.len() as u64,
            size_known: true,
            present: !orphans.is_empty(),
            details: orphans.iter().map(|(p, s)| (p.display().to_string(), *s)).collect(),
            note: None,
        }),
        Special::OldDrivers => drivers::find_old(progress).map(|old| SpecialScan {
            bytes: old.iter().map(|d| d.size).sum(),
            count: old.len() as u64,
            size_known: true,
            present: !old.is_empty(),
            details: old.iter().map(|d| (d.label(), d.size)).collect(),
            note: None,
        }),
    };
    r.unwrap_or_else(|e| SpecialScan { note: Some(e), ..Default::default() })
}

pub fn clean(t: &Target, special: Special, progress: &Progress, log: &DeletionLog) -> Result<Tally, String> {
    if !cfg!(windows) {
        return Err("Windows only".into());
    }
    match special {
        Special::RecycleBin => {
            let (bytes, count) = sys::recycle_bin_query().unwrap_or((0, 0));
            sys::recycle_bin_empty()?;
            log.note(&format!("Recycle Bin emptied ({count} items, {bytes} bytes)"));
            Ok(Tally { files: count, bytes, ..Default::default() })
        }
        Special::ExplorerMru => mru::clean(log),
        Special::Cleanmgr => {
            let before = scan(t, special, progress);
            let freed = measure_free(|| cleanmgr::run(&t.cleanmgr))?;
            log.note(&format!("Disk Cleanup ({}) freed {freed} bytes", t.cleanmgr.join(", ")));
            Ok(Tally { files: before.count, bytes: freed, ..Default::default() })
        }
        Special::Dism => {
            let freed = measure_free(|| {
                let mut cmd = sys::command(crate::paths::system_tool("dism.exe"));
                cmd.args(["/Online", "/Cleanup-Image", "/StartComponentCleanup", "/Quiet", "/NoRestart"]);
                sys::run_wait_tree(cmd, std::time::Duration::from_secs(3 * 3600)).map(|_| ())
            })?;
            log.note(&format!("DISM StartComponentCleanup freed {freed} bytes"));
            Ok(Tally { bytes: freed, ..Default::default() })
        }
        Special::EventLogs => {
            let before = scan(t, special, progress);
            sys::powershell(
                "$s=[System.Diagnostics.Eventing.Reader.EventLogSession]::GlobalSession;\
                 foreach($n in $s.GetLogNames()){ try { $s.ClearLog($n) } catch {} }",
            )?;
            let after = scan(t, special, progress);
            log.note("All event logs cleared");
            Ok(Tally { files: before.count, bytes: before.bytes.saturating_sub(after.bytes), ..Default::default() })
        }
        Special::RestorePoints => {
            let mut removed = 0;
            let freed = measure_free(|| {
                removed = restore_points::delete_all_but_newest()?;
                Ok(())
            })?;
            log.note(&format!("Deleted {removed} restore points"));
            Ok(Tally { files: removed, bytes: freed, ..Default::default() })
        }
        Special::InstallerOrphans => installer::remove_orphans(log),
        Special::OldDrivers => drivers::remove_old(progress, log),
    }
}

/// Run, address bar and search history in HKCU.
mod mru {
    use super::*;

    #[cfg(windows)]
    const KEYS: &[&str] = &[
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\RunMRU",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\TypedPaths",
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\WordWheelQuery",
    ];

    #[cfg(windows)]
    pub fn scan() -> SpecialScan {
        use crate::reg::{open, subkeys, value_names, Hive};
        let mut count = 0u64;
        for k in KEYS {
            if let Some(key) = open(Hive::Hkcu, k) {
                count += value_names(&key).iter().filter(|n| !n.starts_with("MRUList")).count() as u64;
                count += subkeys(&key).len() as u64;
            }
        }
        SpecialScan { count, size_known: true, present: count > 0, ..Default::default() }
    }

    #[cfg(windows)]
    pub fn clean(log: &DeletionLog) -> Result<Tally, String> {
        use crate::reg::{open_rw, subkeys, value_names, Hive};
        let mut removed = 0u64;
        for k in KEYS {
            let Some(key) = open_rw(Hive::Hkcu, k) else { continue };
            for name in value_names(&key) {
                if key.delete_value(&name).is_ok() && !name.starts_with("MRUList") {
                    removed += 1;
                }
            }
            for sub in subkeys(&key) {
                if key.delete_subkey_all(&sub).is_ok() {
                    removed += 1;
                }
            }
        }
        log.note(&format!("Cleared {removed} Explorer history entries"));
        Ok(Tally { files: removed, ..Default::default() })
    }

    #[cfg(not(windows))]
    pub fn scan() -> SpecialScan {
        SpecialScan::default()
    }
    #[cfg(not(windows))]
    pub fn clean(_log: &DeletionLog) -> Result<Tally, String> {
        Err("Windows only".into())
    }
}

/// Windows Disk Cleanup automation (`cleanmgr /sagerun`).
mod cleanmgr {
    #[cfg(windows)]
    pub fn run(handlers: &[String]) -> Result<(), String> {
        use crate::reg::{open, open_rw, subkeys, Hive};
        const BASE: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\VolumeCaches";
        const FLAG: &str = "StateFlags0117";
        // Clear our flag everywhere first: a flag left by an interrupted run must never make
        // this run clean something the user did not select (e.g. Windows.old).
        if let Some(root) = open(Hive::Hklm, BASE) {
            for h in subkeys(&root) {
                if let Some(k) = open_rw(Hive::Hklm, &format!("{BASE}\\{h}")) {
                    let _ = k.delete_value(FLAG);
                }
            }
        }
        let mut set = Vec::new();
        for h in handlers {
            if let Some(k) = open_rw(Hive::Hklm, &format!("{BASE}\\{h}")) {
                if k.set_value(FLAG, &2u32).is_ok() {
                    set.push(k);
                }
            }
        }
        if set.is_empty() {
            return Err("Disk Cleanup handlers are not available on this system".into());
        }
        let mut cmd = crate::sys::command(crate::paths::system_tool("cleanmgr.exe"));
        cmd.arg("/sagerun:117");
        let r = crate::sys::run_wait_tree(cmd, std::time::Duration::from_secs(2 * 3600));
        for k in set {
            let _ = k.delete_value(FLAG);
        }
        r.map(|_| ())
    }

    #[cfg(not(windows))]
    pub fn run(_handlers: &[String]) -> Result<(), String> {
        Err("Windows only".into())
    }
}

mod restore_points {
    use super::*;

    pub fn scan() -> Result<SpecialScan, String> {
        let out = sys::powershell(
            "$vol=(Get-CimInstance -ClassName Win32_Volume | Where-Object { $_.DriveLetter -eq $env:SystemDrive }).DeviceID;\
             $s=@(Get-CimInstance -ClassName Win32_ShadowCopy -ErrorAction Stop | Where-Object { $_.VolumeName -eq $vol });\
             $u=(Get-CimInstance -ClassName Win32_ShadowStorage -ErrorAction SilentlyContinue | Where-Object { $_.Volume.DeviceID -eq $vol } | Measure-Object -Property UsedSpace -Sum).Sum;\
             [pscustomobject]@{Count=$s.Count; Used=[int64]$u} | ConvertTo-Json -Compress",
        )?;
        let v: serde_json::Value = serde_json::from_str(out.trim()).map_err(|e| e.to_string())?;
        let count = v["Count"].as_u64().unwrap_or(0);
        let used = v["Used"].as_u64().unwrap_or(0);
        let deletable = count.saturating_sub(1);
        Ok(SpecialScan {
            bytes: if count > 0 { used / count * deletable } else { 0 },
            count: deletable,
            size_known: false,
            present: deletable > 0,
            details: vec![],
            note: Some(format!("{count} restore point(s); the newest is kept")),
        })
    }

    pub fn delete_all_but_newest() -> Result<u64, String> {
        let out = sys::powershell(
            "$ErrorActionPreference='Stop';\
             $vol=(Get-CimInstance -ClassName Win32_Volume | Where-Object { $_.DriveLetter -eq $env:SystemDrive }).DeviceID;\
             if(-not $vol){ throw 'system volume not found' };\
             $s=@(Get-CimInstance -ClassName Win32_ShadowCopy | Where-Object { $_.VolumeName -eq $vol } | Sort-Object InstallDate);\
             $n=0; if($s.Count -gt 1){ $s | Select-Object -First ($s.Count-1) | ForEach-Object { Remove-CimInstance -InputObject $_; $n++ } };\
             $n",
        )?;
        Ok(out.trim().parse().unwrap_or(0))
    }
}

/// Orphaned packages in %SystemRoot%\Installer. Fails closed: any doubt => nothing removed.
mod installer {
    use super::*;
    use crate::util::norm_key;
    use std::collections::HashSet;

    const SCRIPT: &str = r#"$ErrorActionPreference='Stop'
$i = New-Object -ComObject WindowsInstaller.Installer
$t = $i.GetType()
$out = New-Object System.Collections.Generic.List[string]
foreach($p in $t.InvokeMember('ProductsEx','GetProperty',$null,$i,@('', 's-1-1-0', 7))) {
  try { $lp = $p.GetType().InvokeMember('InstallProperty','GetProperty',$null,$p,@('LocalPackage')); if($lp){ $out.Add($lp) } } catch {}
}
foreach($p in $t.InvokeMember('PatchesEx','GetProperty',$null,$i,@('', 's-1-1-0', 7, 15))) {
  try { $lp = $p.GetType().InvokeMember('PatchProperty','GetProperty',$null,$p,@('LocalPackage')); if($lp){ $out.Add($lp) } } catch {}
}
ConvertTo-Json -InputObject @($out) -Compress"#;

    fn installer_dir() -> PathBuf {
        PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())).join("Installer")
    }

    pub fn find_orphans() -> Result<Vec<(PathBuf, u64)>, String> {
        let out = sys::powershell(SCRIPT)?;
        let registered: HashSet<String> = sys::json_list(&out)
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| norm_key(std::path::Path::new(s)))
            .collect();
        if registered.is_empty() {
            return Err("Could not read the installed package list; nothing will be removed".into());
        }
        let mut all = Vec::new();
        for e in std::fs::read_dir(installer_dir()).map_err(|e| e.to_string())?.flatten() {
            let Ok(md) = e.metadata() else { continue };
            let p = e.path();
            let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
            if md.is_file() && (ext == "msi" || ext == "msp") {
                all.push((p, md.len()));
            }
        }
        let orphans: Vec<_> = all.iter().filter(|(p, _)| !registered.contains(&norm_key(p))).cloned().collect();
        if !all.is_empty() && orphans.len() == all.len() {
            return Err("Safety stop: every package looked orphaned, so the check is not trustworthy".into());
        }
        Ok(orphans)
    }

    pub fn remove_orphans(log: &DeletionLog) -> Result<Tally, String> {
        let mut t = Tally::default();
        for (p, size) in find_orphans()? {
            match std::fs::remove_file(&p) {
                Ok(()) => {
                    log.deleted(&p, size);
                    t.files += 1;
                    t.bytes += size;
                }
                Err(e) => {
                    log.skipped(&p, &e.to_string());
                    t.skipped += 1;
                }
            }
        }
        Ok(t)
    }
}

/// Superseded third-party driver packages (pnputil refuses to remove drivers in use).
mod drivers {
    use super::*;
    use std::collections::HashMap;

    #[derive(Debug, Clone)]
    pub struct Driver {
        pub published: String,
        pub original: String,
        pub provider: String,
        pub class: String,
        pub version: String,
        pub date: String,
        pub size: u64,
    }

    impl Driver {
        pub fn label(&self) -> String {
            format!("{} {} ({}, {}, {})", self.provider, self.class, self.version, self.date, self.published)
        }
        fn version_key(&self) -> Vec<u64> {
            self.version.split('.').map(|p| p.parse().unwrap_or(0)).collect()
        }
    }

    fn list() -> Result<Vec<Driver>, String> {
        let out = sys::powershell(
            "$ErrorActionPreference='Stop'; Get-WindowsDriver -Online | Select-Object \
             @{n='Driver';e={$_.Driver}}, @{n='Original';e={$_.OriginalFileName}}, \
             @{n='Provider';e={$_.ProviderName}}, @{n='Class';e={$_.ClassName}}, \
             @{n='Version';e={[string]$_.Version}}, @{n='Date';e={$_.Date.ToString('yyyyMMdd')}} | ConvertTo-Json -Compress",
        )?;
        let s = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or("").to_string();
        Ok(sys::json_list(&out)
            .iter()
            .map(|v| Driver {
                published: s(v, "Driver"),
                original: s(v, "Original"),
                provider: s(v, "Provider"),
                class: s(v, "Class"),
                version: s(v, "Version"),
                date: s(v, "Date"),
                size: 0,
            })
            .filter(|d| d.published.to_lowercase().starts_with("oem") && !d.original.is_empty())
            .collect())
    }

    /// Group by (inf name, provider, class); everything but the newest is "old".
    pub fn pick_old(drivers: Vec<Driver>) -> Vec<Driver> {
        let mut groups: HashMap<(String, String, String), Vec<Driver>> = HashMap::new();
        for d in drivers {
            let inf = d.original.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase();
            groups.entry((inf, d.provider.to_lowercase(), d.class.to_lowercase())).or_default().push(d);
        }
        let mut old = Vec::new();
        for (_, mut v) in groups {
            if v.len() < 2 {
                continue;
            }
            v.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.version_key().cmp(&a.version_key())));
            old.extend(v.into_iter().skip(1));
        }
        old
    }

    pub fn find_old(progress: &Progress) -> Result<Vec<Driver>, String> {
        let mut old = pick_old(list()?);
        for d in &mut old {
            if let Some(dir) = std::path::Path::new(&d.original).parent() {
                d.size = size_of(dir, progress).bytes;
            }
        }
        Ok(old)
    }

    pub fn remove_old(progress: &Progress, log: &DeletionLog) -> Result<Tally, String> {
        let mut t = Tally::default();
        for d in find_old(progress)? {
            match sys::run_tool("pnputil.exe", &["/delete-driver", &d.published]) {
                Ok(_) => {
                    log.note(&format!("Removed driver package {}", d.label()));
                    t.files += 1;
                    t.bytes += d.size;
                }
                Err(e) => {
                    log.note(&format!("Kept driver package {} ({e})", d.label()));
                    t.skipped += 1;
                }
            }
        }
        Ok(t)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn drv(p: &str, ver: &str, date: &str) -> Driver {
            Driver {
                published: p.into(),
                original: format!("C:\\Windows\\System32\\DriverStore\\FileRepository\\nv_dispi.inf_{p}\\nv_dispi.inf"),
                provider: "NVIDIA".into(),
                class: "Display".into(),
                version: ver.into(),
                date: date.into(),
                size: 0,
            }
        }

        #[test]
        fn keeps_newest_driver() {
            let old = pick_old(vec![
                drv("oem1.inf", "31.0.15.1", "20230101"),
                drv("oem2.inf", "32.0.15.5", "20240601"),
                drv("oem3.inf", "31.0.15.9", "20231001"),
            ]);
            let mut names: Vec<_> = old.iter().map(|d| d.published.as_str()).collect();
            names.sort();
            assert_eq!(names, vec!["oem1.inf", "oem3.inf"]);
        }
    }
}
