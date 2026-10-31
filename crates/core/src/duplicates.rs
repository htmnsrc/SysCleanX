//! Duplicate file finder: size -> head/tail hash -> full hash (xxh3-128), all parallel.

use crate::paths::is_link;
use crate::progress::Progress;
use crate::util::norm_key;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use xxhash_rust::xxh3::{xxh3_128, Xxh3};

#[derive(Debug, Clone)]
pub struct DupFile {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone)]
pub struct DupGroup {
    pub size: u64,
    pub files: Vec<DupFile>,
}

impl DupGroup {
    /// Space recovered by keeping one copy.
    pub fn wasted(&self) -> u64 {
        self.size * (self.files.len() as u64).saturating_sub(1)
    }
}

const SAMPLE: usize = 16 * 1024;

fn collect(dir: &Path, min_size: u64, out: &mut Vec<(PathBuf, u64, Option<SystemTime>)>, progress: &Progress) {
    if progress.is_cancelled() {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(md) = e.metadata() else { continue };
        if is_link(&md) {
            continue;
        }
        if md.is_dir() {
            collect(&e.path(), min_size, out, progress);
        } else if md.len() >= min_size {
            progress.add(1, 0);
            out.push((e.path(), md.len(), md.modified().ok()));
        }
    }
}

fn sample_hash(path: &Path, size: u64) -> Option<u128> {
    let mut f = File::open(path).ok()?;
    // Files up to 2×SAMPLE are hashed completely here (the full pass skips them).
    let head = if size as usize <= SAMPLE * 2 { size as usize } else { SAMPLE };
    let mut buf = vec![0u8; head];
    f.read_exact(&mut buf).ok()?;
    if size as usize > SAMPLE * 2 {
        let mut tail = vec![0u8; SAMPLE];
        f.seek(SeekFrom::End(-(SAMPLE as i64))).ok()?;
        f.read_exact(&mut tail).ok()?;
        buf.extend_from_slice(&tail);
    }
    Some(xxh3_128(&buf))
}

fn full_hash(path: &Path) -> Option<u128> {
    let mut f = File::open(path).ok()?;
    let mut h = Xxh3::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.digest128())
}

/// Regroup each group by `key`, dropping singletons.
fn refine<K, F>(groups: Vec<Vec<(PathBuf, u64, Option<SystemTime>)>>, progress: &Progress, key: F) -> Vec<Vec<(PathBuf, u64, Option<SystemTime>)>>
where
    K: std::hash::Hash + Eq + Send,
    F: Fn(&Path, u64) -> Option<K> + Sync,
{
    groups
        .into_par_iter()
        .flat_map(|g| {
            if progress.is_cancelled() {
                return Vec::new();
            }
            let mut by: HashMap<K, Vec<_>> = HashMap::new();
            for f in g {
                if let Some(k) = key(&f.0, f.1) {
                    by.entry(k).or_default().push(f);
                }
            }
            by.into_values().filter(|v| v.len() > 1).collect::<Vec<_>>()
        })
        .collect()
}

pub fn find(roots: &[PathBuf], min_size: u64, progress: &Progress) -> Vec<DupGroup> {
    progress.set_label("Listing files…");
    let mut files = Vec::new();
    for r in roots {
        collect(r, min_size.max(1), &mut files, progress);
    }
    let mut seen = HashSet::new();
    files.retain(|f| seen.insert(norm_key(&f.0)));

    let mut by_size: HashMap<u64, Vec<_>> = HashMap::new();
    for f in files {
        by_size.entry(f.1).or_default().push(f);
    }
    let groups: Vec<_> = by_size.into_values().filter(|v| v.len() > 1).collect();
    progress.set_label("Comparing file samples…");
    let groups = refine(groups, progress, sample_hash);
    progress.set_label("Verifying full contents…");
    let groups = refine(groups, progress, |p, size| if size as usize <= SAMPLE * 2 { Some(0u128) } else { full_hash(p) });

    let mut out: Vec<DupGroup> = groups
        .into_iter()
        .map(|g| DupGroup {
            size: g[0].1,
            files: g.into_iter().map(|(path, _, modified)| DupFile { path, modified }).collect(),
        })
        .collect();
    out.sort_by(|a, b| b.wasted().cmp(&a.wasted()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_true_duplicates_only() {
        let root = std::env::temp_dir().join(format!("scx-dup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("x")).unwrap();
        let big: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let mut almost = big.clone();
        almost[50_000] ^= 1; // same size, same head/tail, different middle
        fs::write(root.join("a.bin"), &big).unwrap();
        fs::write(root.join("x/b.bin"), &big).unwrap();
        fs::write(root.join("c.bin"), &almost).unwrap();
        fs::write(root.join("small1"), b"hello").unwrap();
        fs::write(root.join("small2"), b"hello").unwrap();
        let groups = find(&[root.clone()], 10, &Progress::default());
        assert_eq!(groups.len(), 1, "{groups:?}");
        assert_eq!(groups[0].files.len(), 2);
        assert_eq!(groups[0].wasted(), 100_000);
        let small = find(&[root.clone(), root.join("x")], 1, &Progress::default());
        assert_eq!(small.len(), 2, "overlapping roots must not double count");
        let _ = fs::remove_dir_all(&root);
    }
}
