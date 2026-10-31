//! Append-only log of everything the engine deletes or skips.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const KEEP_LOGS: usize = 30;

/// Delete all but the newest `keep - 1` log files (a new one is about to be created).
fn prune(dir: &Path, keep: usize) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut logs: Vec<_> = rd
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "log").unwrap_or(false))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    if logs.len() < keep {
        return;
    }
    logs.sort();
    for (_, p) in logs.iter().take(logs.len() + 1 - keep) {
        let _ = fs::remove_file(p);
    }
}

pub struct DeletionLog {
    out: Mutex<Option<BufWriter<File>>>,
    pub path: Option<PathBuf>,
}

impl DeletionLog {
    /// Create `<dir>/<prefix>-<timestamp>.log`. Falls back to a disabled log on error.
    pub fn create(dir: &Path, prefix: &str) -> Self {
        prune(dir, KEEP_LOGS);
        let path = dir.join(format!("{prefix}-{}.log", crate::sys::file_stamp()));
        let file = fs::create_dir_all(dir).and_then(|_| File::create(&path));
        match file {
            Ok(f) => {
                let log = DeletionLog { out: Mutex::new(Some(BufWriter::new(f))), path: Some(path) };
                log.note(&format!("SysCleanX {} log started {}", env!("CARGO_PKG_VERSION"), crate::sys::time_stamp()));
                log
            }
            Err(_) => Self::disabled(),
        }
    }

    pub fn disabled() -> Self {
        DeletionLog { out: Mutex::new(None), path: None }
    }

    fn write(&self, line: &str) {
        if let Ok(mut g) = self.out.lock() {
            if let Some(w) = g.as_mut() {
                let _ = writeln!(w, "{line}");
            }
        }
    }

    pub fn deleted(&self, path: &Path, bytes: u64) {
        self.write(&format!("DEL\t{bytes}\t{}", path.display()));
    }
    pub fn skipped(&self, path: &Path, reason: &str) {
        self.write(&format!("SKIP\t{reason}\t{}", path.display()));
    }
    pub fn reboot(&self, path: &Path) {
        self.write(&format!("REBOOT\t{}", path.display()));
    }
    pub fn note(&self, text: &str) {
        self.write(&format!("#\t{text}"));
    }
    pub fn flush(&self) {
        if let Ok(mut g) = self.out.lock() {
            if let Some(w) = g.as_mut() {
                let _ = w.flush();
            }
        }
    }
}

impl Drop for DeletionLog {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn prune_keeps_newest() {
        let dir = std::env::temp_dir().join(format!("scx-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..5 {
            std::fs::write(dir.join(format!("clean-{i}.log")), "x").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        std::fs::write(dir.join("keep.txt"), "x").unwrap();
        super::prune(&dir, 3);
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, vec!["clean-3.log", "clean-4.log", "keep.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
