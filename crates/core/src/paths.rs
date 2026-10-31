//! Path specs from target data: `%ENV%` expansion plus `*`/`?` wildcards in any component.

use crate::util::{expand_env, has_wildcard, norm_key, wildcard_match};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// True for symlinks, junctions and other name-surrogate reparse points.
pub fn is_link(md: &fs::Metadata) -> bool {
    if md.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        // Any reparse-point directory (junction, mount point, cloud folder) is never entered.
        if md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 && md.is_dir() {
            return true;
        }
    }
    false
}

/// Split a path string on both separators into an absolute base and components.
fn split_spec(spec: &str) -> Option<(PathBuf, Vec<&str>)> {
    let parts: Vec<&str> = spec.split(['\\', '/']).filter(|s| !s.is_empty()).collect();
    if spec.starts_with('/') && !cfg!(windows) {
        return Some((PathBuf::from("/"), parts));
    }
    let first = *parts.first()?;
    if first.len() == 2 && first.ends_with(':') && first.as_bytes()[0].is_ascii_alphabetic() {
        return Some((PathBuf::from(format!("{first}\\")), parts[1..].to_vec()));
    }
    None
}

/// Expand env vars and wildcards. Only existing paths are returned. Wildcards never
/// match through symlinks/junctions. Results are deduplicated case-insensitively.
pub fn resolve(spec: &str) -> Vec<PathBuf> {
    let Some(expanded) = expand_env(spec) else { return Vec::new() };
    let Some((base, parts)) = split_spec(&expanded) else { return Vec::new() };
    let mut current = vec![base];
    for (i, part) in parts.iter().enumerate() {
        let last = i == parts.len() - 1;
        let mut next = Vec::new();
        for dir in &current {
            if has_wildcard(part) {
                let Ok(rd) = fs::read_dir(dir) else { continue };
                for e in rd.flatten() {
                    let name = e.file_name();
                    let Some(name) = name.to_str() else { continue };
                    if !wildcard_match(part, name) {
                        continue;
                    }
                    let Ok(md) = e.metadata() else { continue };
                    if is_link(&md) || (!last && !md.is_dir()) {
                        continue;
                    }
                    next.push(e.path());
                }
            } else {
                next.push(dir.join(part));
            }
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }
    dedupe(current.into_iter().filter(|p| fs::symlink_metadata(p).is_ok()).collect())
}

pub fn dedupe(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    paths.into_iter().filter(|p| seen.insert(norm_key(p))).collect()
}

/// Remove paths that are nested inside another path of the same list.
pub fn drop_nested(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let keys: Vec<String> = paths.iter().map(|p| norm_key(p)).collect();
    paths
        .into_iter()
        .enumerate()
        .filter(|(i, _)| {
            !keys.iter().enumerate().any(|(j, other)| {
                j != *i && keys[*i] != *other && crate::util::key_within(&keys[*i], other)
            })
        })
        .map(|(_, p)| p)
        .collect()
}

/// Join a relative sub-path written with either separator.
pub fn join_rel(base: &Path, rel: &str) -> PathBuf {
    let mut p = base.to_path_buf();
    for part in rel.split(['\\', '/']).filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}

/// Windows system tool path, e.g. `C:\Windows\System32\dism.exe`. Absolute paths avoid
/// PATH hijacking when running elevated.
pub fn system_tool(name: &str) -> PathBuf {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    PathBuf::from(root).join("System32").join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_wildcards_and_dedupes() {
        let tmp = std::env::temp_dir().join(format!("scx-paths-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        for d in ["Profiles/a1.default/cache2", "Profiles/b2.dev/cache2", "Profiles/c3/other"] {
            fs::create_dir_all(join_rel(&tmp, d)).unwrap();
        }
        std::env::set_var("SCX_PATHS_TEST", &tmp);
        let mut found = resolve("%SCX_PATHS_TEST%/Profiles/*/cache2");
        found.sort();
        assert_eq!(found.len(), 2);
        assert!(found[0].ends_with("cache2"));
        assert!(resolve("%SCX_NOT_SET_VAR%/x").is_empty());
        assert!(resolve("relative/path").is_empty());
        let nested = drop_nested(vec![tmp.join("Profiles"), tmp.join("Profiles").join("c3")]);
        assert_eq!(nested.len(), 1);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn wildcards_skip_symlinks() {
        let tmp = std::env::temp_dir().join(format!("scx-links-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("real/Cache")).unwrap();
        fs::create_dir_all(tmp.join("outside/Cache")).unwrap();
        std::os::unix::fs::symlink(tmp.join("outside"), tmp.join("link")).unwrap();
        let found = resolve(&format!("{}/*/Cache", tmp.display()));
        assert_eq!(found.len(), 2, "{found:?}"); // real + outside, never via link
        assert!(found.iter().all(|p| !p.starts_with(tmp.join("link"))));
        let _ = fs::remove_dir_all(&tmp);
    }
}
