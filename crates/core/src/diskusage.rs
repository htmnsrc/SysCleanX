//! Parallel folder-size tree + largest files, for the disk usage view.

use crate::paths::is_link;
use crate::progress::Progress;
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Default)]
pub struct DirNode {
    pub name: String,
    /// Total size including subfolders.
    pub size: u64,
    pub file_count: u64,
    /// Size of files directly in this folder.
    pub own_files: u64,
    /// Sorted largest first.
    pub children: Vec<DirNode>,
}

#[derive(Debug, Clone)]
pub struct BigFile {
    pub path: PathBuf,
    pub size: u64,
    pub age_days: u64,
}

#[derive(Debug, Clone, Default)]
pub struct DiskReport {
    pub root_path: PathBuf,
    pub root: DirNode,
    pub largest: Vec<BigFile>,
    pub unreadable: u64,
}

fn merge_top(mut a: Vec<BigFile>, b: Vec<BigFile>, n: usize) -> Vec<BigFile> {
    a.extend(b);
    if a.len() > n {
        a.sort_unstable_by(|x, y| y.size.cmp(&x.size));
        a.truncate(n);
    }
    a
}

fn walk(path: &Path, name: String, top_n: usize, progress: &Progress, now: SystemTime) -> (DirNode, Vec<BigFile>, u64) {
    let mut node = DirNode { name, ..Default::default() };
    if progress.is_cancelled() {
        return (node, Vec::new(), 0);
    }
    let Ok(rd) = fs::read_dir(path) else { return (node, Vec::new(), 1) };
    let mut top = Vec::new();
    let mut subdirs = Vec::new();
    let mut files = 0u64;
    for e in rd.flatten() {
        let Ok(md) = e.metadata() else { continue };
        if is_link(&md) {
            continue;
        }
        if md.is_dir() {
            subdirs.push((e.path(), e.file_name().to_string_lossy().into_owned()));
        } else {
            let size = md.len();
            node.own_files += size;
            files += 1;
            let age_days = md.modified().ok().and_then(|t| now.duration_since(t).ok()).map(|d| d.as_secs() / 86_400).unwrap_or(0);
            top.push(BigFile { path: e.path(), size, age_days });
            if top.len() > top_n * 2 {
                top = merge_top(top, Vec::new(), top_n);
            }
        }
    }
    progress.add(files, node.own_files);
    let (children, child_top, unreadable) = subdirs
        .into_par_iter()
        .map(|(p, n)| walk(&p, n, top_n, progress, now))
        .fold(
            || (Vec::new(), Vec::new(), 0u64),
            |(mut nodes, t, u), (n, nt, nu)| {
                nodes.push(n);
                (nodes, merge_top(t, nt, top_n), u + nu)
            },
        )
        .reduce(
            || (Vec::new(), Vec::new(), 0u64),
            |(mut a, at, au), (b, bt, bu)| {
                a.extend(b);
                (a, merge_top(at, bt, top_n), au + bu)
            },
        );
    node.children = children;
    node.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
    node.size = node.own_files + node.children.iter().map(|c| c.size).sum::<u64>();
    node.file_count = files + node.children.iter().map(|c| c.file_count).sum::<u64>();
    (node, merge_top(top, child_top, top_n), unreadable)
}

pub fn analyze(root: &Path, top_n: usize, progress: &Progress) -> DiskReport {
    progress.set_label(format!("Measuring {}", root.display()));
    let (node, mut largest, unreadable) = walk(root, root.display().to_string(), top_n, progress, SystemTime::now());
    largest.sort_unstable_by(|a, b| b.size.cmp(&a.size));
    largest.truncate(top_n);
    DiskReport { root_path: root.to_path_buf(), root: node, largest, unreadable }
}

impl DirNode {
    /// Follow a path of child names from this node.
    pub fn descend(&self, names: &[String]) -> Option<&DirNode> {
        let mut cur = self;
        for n in names {
            cur = cur.children.iter().find(|c| &c.name == n)?;
        }
        Some(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_largest_files() {
        let root = std::env::temp_dir().join(format!("scx-du-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("top.bin"), vec![0u8; 100]).unwrap();
        fs::write(root.join("a/mid.bin"), vec![0u8; 300]).unwrap();
        fs::write(root.join("a/b/big.bin"), vec![0u8; 1000]).unwrap();
        let r = analyze(&root, 2, &Progress::default());
        assert_eq!(r.root.size, 1400);
        assert_eq!(r.root.file_count, 3);
        assert_eq!(r.root.children[0].name, "a");
        assert_eq!(r.root.children[0].size, 1300);
        assert_eq!(r.largest.len(), 2);
        assert_eq!(r.largest[0].size, 1000);
        assert!(r.root.descend(&["a".into(), "b".into()]).is_some());
        let _ = fs::remove_dir_all(&root);
    }
}
