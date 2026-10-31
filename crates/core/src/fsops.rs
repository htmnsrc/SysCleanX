//! Single-pass, parallel scan/purge of a folder tree.
//!
//! Scan and delete share one walker so the preview always matches what cleaning does.
//! Sizes come from directory enumeration (free on Windows), links are never followed,
//! sealed folders are never entered, and the root folder itself is kept unless asked.

use crate::guard::Guard;
use crate::log::DeletionLog;
use crate::paths::is_link;
use crate::progress::Progress;
use crate::util::{key_within, norm_key, wildcard_match};
use rayon::prelude::*;
use std::fs;
use std::io;
use std::ops::{Add, AddAssign};
use std::path::Path;
use std::time::{Duration, SystemTime};

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tally {
    pub files: u64,
    pub bytes: u64,
    pub dirs: u64,
    pub skipped: u64,
    pub skipped_bytes: u64,
    pub reboot: u64,
}

impl Add for Tally {
    type Output = Tally;
    fn add(self, o: Tally) -> Tally {
        Tally {
            files: self.files + o.files,
            bytes: self.bytes + o.bytes,
            dirs: self.dirs + o.dirs,
            skipped: self.skipped + o.skipped,
            skipped_bytes: self.skipped_bytes + o.skipped_bytes,
            reboot: self.reboot + o.reboot,
        }
    }
}

impl AddAssign for Tally {
    fn add_assign(&mut self, o: Tally) {
        *self = *self + o;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Scan,
    Delete { reboot_locked: bool },
}

#[derive(Default, Clone, Debug)]
pub struct Filter {
    /// File name patterns to include; empty = every file.
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub min_age: Option<Duration>,
    pub recursive: bool,
    /// Skip LevelDB / ESE / SQLite folders entirely (used by heuristic targets).
    pub skip_markers: bool,
    /// Remove empty folders only; files are never touched.
    pub empty_dirs: bool,
    /// User exclusions as normalized keys (files and folders).
    pub exclusions: Vec<String>,
    /// Folders never entered, e.g. caches another target already covers (checked per folder).
    pub skip_dirs: Vec<String>,
}

impl Filter {
    /// Filtered purges only remove matching files (or empty folders), never whole trees.
    pub fn is_filtered(&self) -> bool {
        !self.include.is_empty() || self.empty_dirs
    }

    fn old_enough(&self, md: &fs::Metadata, now: SystemTime) -> bool {
        let Some(min_age) = self.min_age else { return true };
        let newest = [md.modified().ok(), md.created().ok()].into_iter().flatten().max();
        match newest {
            Some(t) => now.duration_since(t).map(|age| age >= min_age).unwrap_or(false),
            None => false,
        }
    }
}

pub struct Ctx<'a> {
    pub guard: &'a Guard,
    pub progress: &'a Progress,
    pub log: &'a DeletionLog,
    pub now: SystemTime,
}

/// Directories whose files belong to live databases; never touched by heuristics.
const MARKER_DIRS: &[&str] = &[
    "indexeddb", "local storage", "session storage", "databases", "leveldb",
    "service worker", "blob_storage", "file system", "storage", "webstorage",
];

fn has_marker(dir: &Path, entries: &[fs::DirEntry]) -> bool {
    let own = dir.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if MARKER_DIRS.contains(&own.as_str()) || own.ends_with(".leveldb") {
        return true;
    }
    entries.iter().any(|e| {
        let n = e.file_name().to_string_lossy().to_lowercase();
        n == "current" || n == "lock" || n.ends_with(".ldb") || n.ends_with(".chk")
            || n.ends_with(".jrs") || n.ends_with("-journal") || n.ends_with("-wal") || n.ends_with("-shm")
    })
}

fn is_locked(e: &io::Error) -> bool {
    // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION
    cfg!(windows) && matches!(e.raw_os_error(), Some(32) | Some(33))
}

#[allow(clippy::permissions_set_readonly_false)]
fn remove_file_robust(path: &Path, md: &fs::Metadata) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied && md.permissions().readonly() => {
            let mut perms = md.permissions();
            perms.set_readonly(false);
            fs::set_permissions(path, perms)?;
            fs::remove_file(path)
        }
        r => r,
    }
}

fn excluded(path: &Path, exclusions: &[String]) -> bool {
    if exclusions.is_empty() {
        return false;
    }
    let key = norm_key(path);
    exclusions.iter().any(|x| key_within(&key, x))
}

/// Handle one file. Returns (tally, removed-or-would-be-removed).
fn file(path: &Path, md: &fs::Metadata, f: &Filter, mode: Mode, ctx: &Ctx) -> (Tally, bool) {
    if f.empty_dirs {
        return (Tally::default(), false);
    }
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    if !f.include.is_empty() && !f.include.iter().any(|p| wildcard_match(p, &name)) {
        return (Tally::default(), false);
    }
    if f.exclude.iter().any(|p| wildcard_match(p, &name)) || !f.old_enough(md, ctx.now) {
        return (Tally::default(), false);
    }
    let size = md.len();
    match mode {
        Mode::Scan => {
            ctx.progress.add(1, size);
            (Tally { files: 1, bytes: size, ..Default::default() }, true)
        }
        Mode::Delete { reboot_locked } => match remove_file_robust(path, md) {
            Ok(()) => {
                ctx.progress.add(1, size);
                ctx.log.deleted(path, size);
                (Tally { files: 1, bytes: size, ..Default::default() }, true)
            }
            Err(e) => {
                if reboot_locked && is_locked(&e) && crate::sys::delete_on_reboot(path) {
                    ctx.log.reboot(path);
                    (Tally { reboot: 1, ..Default::default() }, false)
                } else {
                    let reason = if is_locked(&e) { "in use".to_string() } else { e.to_string() };
                    ctx.log.skipped(path, &reason);
                    (Tally { skipped: 1, skipped_bytes: size, ..Default::default() }, false)
                }
            }
        },
    }
}

/// Walk a directory. Returns (tally, directory is/would be empty afterwards).
fn walk(dir: &Path, f: &Filter, mode: Mode, ctx: &Ctx) -> (Tally, bool) {
    if ctx.progress.is_cancelled() {
        return (Tally::default(), false);
    }
    let Ok(rd) = fs::read_dir(dir) else { return (Tally::default(), false) };
    let entries: Vec<fs::DirEntry> = rd.flatten().collect();
    if f.skip_markers && has_marker(dir, &entries) {
        return (Tally::default(), false);
    }
    let mut tally = Tally::default();
    let mut all_removed = true;
    let mut subdirs = Vec::new();
    for e in &entries {
        let Ok(md) = e.metadata() else {
            all_removed = false;
            continue;
        };
        let path = e.path();
        if is_link(&md) || excluded(&path, &f.exclusions) || (md.is_dir() && excluded(&path, &f.skip_dirs)) {
            all_removed = false;
            continue;
        }
        if md.is_dir() {
            if f.recursive || f.empty_dirs {
                subdirs.push((path, md));
            } else {
                all_removed = false;
            }
        } else {
            let (t, removed) = file(&path, &md, f, mode, ctx);
            tally += t;
            all_removed &= removed;
        }
    }
    // Folders are only removed by full purges or empty-folder mode.
    let remove_dirs = f.include.is_empty();
    let (sub, sub_all_removed) = subdirs
        .into_par_iter()
        .map(|(p, md)| {
            if ctx.guard.is_sealed(&norm_key(&p)) {
                return (Tally::default(), false);
            }
            let old = f.old_enough(&md, ctx.now); // read before purging changes mtime
            let (mut t, empty) = walk(&p, f, mode, ctx);
            let mut removed = false;
            if remove_dirs && old && empty {
                match mode {
                    Mode::Scan => removed = true,
                    Mode::Delete { .. } => {
                        if fs::remove_dir(&p).is_ok() {
                            ctx.log.deleted(&p, 0);
                            removed = true;
                        }
                    }
                }
            }
            if removed {
                t.dirs += 1;
            }
            (t, removed)
        })
        .reduce(|| (Tally::default(), true), |a, b| (a.0 + b.0, a.1 && b.1));
    (tally + sub, all_removed && sub_all_removed)
}

/// Scan or purge `root`. Folder roots are emptied (and removed only if `remove_root`);
/// file roots are handled as a single file. Callers must run the guard check first.
pub fn process_root(root: &Path, f: &Filter, mode: Mode, remove_root: bool, ctx: &Ctx) -> Tally {
    let Ok(md) = fs::symlink_metadata(root) else { return Tally::default() };
    if is_link(&md) || ctx.guard.is_sealed(&norm_key(root)) || excluded(root, &f.exclusions) {
        return Tally::default();
    }
    if !md.is_dir() {
        return file(root, &md, f, mode, ctx).0;
    }
    let old = f.old_enough(&md, ctx.now);
    let (mut t, empty) = walk(root, f, mode, ctx);
    if remove_root && old && empty && ctx.guard.check_delete(root).is_ok() {
        match mode {
            Mode::Scan => t.dirs += 1,
            Mode::Delete { .. } => {
                if fs::remove_dir(root).is_ok() {
                    ctx.log.deleted(root, 0);
                    t.dirs += 1;
                }
            }
        }
    }
    t
}

/// Read-only size of a path (no filters, no guard). Used for estimates.
pub fn size_of(path: &Path, progress: &Progress) -> Tally {
    let guard = Guard::default();
    let log = DeletionLog::disabled();
    let ctx = Ctx { guard: &guard, progress, log: &log, now: SystemTime::now() };
    let f = Filter { recursive: true, ..Default::default() };
    process_root(path, &f, Mode::Scan, false, &ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(name: &str) -> Tmp {
            let p = std::env::temp_dir().join(format!("scx-fs-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
        fn file(&self, rel: &str, len: usize) {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, vec![b'x'; len]).unwrap();
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn run(root: &Path, f: &Filter, mode: Mode, guard: &Guard) -> Tally {
        let progress = Progress::default();
        let log = DeletionLog::disabled();
        let ctx = Ctx { guard, progress: &progress, log: &log, now: SystemTime::now() };
        process_root(root, f, mode, false, &ctx)
    }

    fn all() -> Filter {
        Filter { recursive: true, ..Default::default() }
    }

    #[test]
    fn purge_empties_but_keeps_root_and_scan_matches_delete() {
        let t = Tmp::new("purge");
        t.file("a.tmp", 10);
        t.file("sub/b.tmp", 20);
        t.file("sub/deeper/c.tmp", 30);
        let g = Guard::default();
        let scan = run(&t.0, &all(), Mode::Scan, &g);
        assert_eq!((scan.files, scan.bytes, scan.dirs), (3, 60, 2));
        let del = run(&t.0, &all(), Mode::Delete { reboot_locked: false }, &g);
        assert_eq!((del.files, del.bytes, del.dirs), (3, 60, 2));
        assert!(t.0.exists(), "root must survive");
        assert_eq!(fs::read_dir(&t.0).unwrap().count(), 0);
    }

    #[test]
    fn include_patterns_and_min_age() {
        let t = Tmp::new("filter");
        t.file("keep.txt", 5);
        t.file("x/old.log", 7);
        let f = Filter { include: vec!["*.log".into()], recursive: true, ..Default::default() };
        let del = run(&t.0, &f, Mode::Delete { reboot_locked: false }, &Guard::default());
        assert_eq!((del.files, del.bytes, del.dirs), (1, 7, 0));
        assert!(t.0.join("keep.txt").exists());
        assert!(t.0.join("x").exists(), "filtered purge never removes folders");

        t.file("fresh.log", 3);
        let aged = Filter { min_age: Some(Duration::from_secs(3600)), ..f };
        let scan = run(&t.0, &aged, Mode::Scan, &Guard::default());
        assert_eq!(scan.files, 0, "files newer than min_age are kept");
    }

    #[test]
    fn non_recursive_only_top_level() {
        let t = Tmp::new("flat");
        t.file("a.lnk", 1);
        t.file("AutomaticDestinations/pinned.dat", 1);
        let f = Filter { recursive: false, ..Default::default() };
        let del = run(&t.0, &f, Mode::Delete { reboot_locked: false }, &Guard::default());
        assert_eq!(del.files, 1);
        assert!(t.0.join("AutomaticDestinations/pinned.dat").exists());
    }

    #[test]
    fn markers_protect_databases() {
        let t = Tmp::new("markers");
        t.file("app/logs/old.log", 4);
        t.file("app/leveldb-ish/000003.log", 4);
        t.file("app/leveldb-ish/CURRENT", 1);
        t.file("app/Local Storage/x.log", 4);
        let f = Filter { include: vec!["*.log".into()], recursive: true, skip_markers: true, ..Default::default() };
        let del = run(&t.0, &f, Mode::Delete { reboot_locked: false }, &Guard::default());
        assert_eq!(del.files, 1);
        assert!(t.0.join("app/leveldb-ish/000003.log").exists());
        assert!(t.0.join("app/Local Storage/x.log").exists());
    }

    #[test]
    fn sealed_folders_are_never_entered() {
        let t = Tmp::new("sealed");
        t.file("ok/a", 1);
        t.file("vault/b", 1);
        let g = Guard::new(vec![], vec![norm_key(&t.0.join("vault"))]);
        let del = run(&t.0, &all(), Mode::Delete { reboot_locked: false }, &g);
        assert_eq!(del.files, 1);
        assert!(t.0.join("vault/b").exists());
    }

    #[test]
    fn empty_dir_mode() {
        let t = Tmp::new("empty");
        fs::create_dir_all(t.0.join("a/b/c")).unwrap();
        t.file("d/keep.txt", 1);
        let f = Filter { empty_dirs: true, recursive: true, ..Default::default() };
        let scan = run(&t.0, &f, Mode::Scan, &Guard::default());
        assert_eq!((scan.files, scan.dirs), (0, 3));
        let del = run(&t.0, &f, Mode::Delete { reboot_locked: false }, &Guard::default());
        assert_eq!(del.dirs, 3);
        assert!(!t.0.join("a").exists());
        assert!(t.0.join("d/keep.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_followed() {
        let t = Tmp::new("links");
        let outside = Tmp::new("links-outside");
        outside.file("precious.txt", 9);
        t.file("junk.tmp", 1);
        std::os::unix::fs::symlink(&outside.0, t.0.join("link")).unwrap();
        let del = run(&t.0, &all(), Mode::Delete { reboot_locked: false }, &Guard::default());
        assert_eq!(del.files, 1);
        assert!(outside.0.join("precious.txt").exists());
    }

    #[test]
    fn exclusions_respected() {
        let t = Tmp::new("excl");
        t.file("keep/a", 1);
        t.file("b", 1);
        let f = Filter { exclusions: vec![norm_key(&t.0.join("keep"))], ..all() };
        let del = run(&t.0, &f, Mode::Delete { reboot_locked: false }, &Guard::default());
        assert_eq!(del.files, 1);
        assert!(t.0.join("keep/a").exists());
    }

    #[test]
    fn skip_dirs_only_affect_folders() {
        let t = Tmp::new("skipdirs");
        t.file("owned/cache/a.tmp", 1);
        t.file("free/b.tmp", 1);
        let f = Filter { include: vec!["*.tmp".into()], skip_dirs: vec![norm_key(&t.0.join("owned"))], ..all() };
        let scan = run(&t.0, &f, Mode::Scan, &Guard::default());
        assert_eq!(scan.files, 1, "files under a folder owned by another target are not counted twice");
    }

    #[test]
    fn readonly_files_are_deleted() {
        let t = Tmp::new("ro");
        t.file("ro.tmp", 2);
        let p = t.0.join("ro.tmp");
        let mut perms = fs::metadata(&p).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&p, perms).unwrap();
        let del = run(&t.0, &all(), Mode::Delete { reboot_locked: false }, &Guard::default());
        assert_eq!(del.files, 1);
    }
}
