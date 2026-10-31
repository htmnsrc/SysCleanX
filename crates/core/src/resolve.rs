//! Turn a target into concrete folders/files on this machine.

use crate::paths::{dedupe, drop_nested, is_link, join_rel, resolve};
use crate::targets::{Target, CHROMIUM_CACHE, CHROMIUM_ROOT_CACHE};
use crate::util::{expand_env, key_within, norm_key};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

fn exists(p: &Path) -> bool {
    fs::symlink_metadata(p).map(|m| !is_link(&m)).unwrap_or(false)
}

/// Profile folders inside a Chromium "User Data" folder (`Default`, `Profile 1`, ...).
/// Browsers like Opera keep the profile directly in the root.
pub fn chromium_profiles(user_data: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(user_data) {
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() && !is_link(&md) && e.path().join("Preferences").is_file() {
                out.push(e.path());
            }
        }
    }
    let root_is_profile = user_data.join("Preferences").is_file()
        || (out.is_empty() && CHROMIUM_CACHE.iter().any(|s| join_rel(user_data, s).is_dir()));
    if root_is_profile {
        out.push(user_data.to_path_buf());
    }
    out
}

/// Concrete paths of a path-based target (no resolver, no special).
pub fn target_items(t: &Target) -> Vec<PathBuf> {
    let mut items = Vec::new();
    for spec in &t.paths {
        for base in resolve(spec) {
            if !t.profile_subpaths.is_empty() {
                for profile in chromium_profiles(&base) {
                    items.extend(t.profile_subpaths.iter().map(|s| join_rel(&profile, s)).filter(|p| exists(p)));
                }
                items.extend(t.subpaths.iter().map(|s| join_rel(&base, s)).filter(|p| exists(p)));
            } else if !t.subpaths.is_empty() {
                items.extend(t.subpaths.iter().map(|s| join_rel(&base, s)).filter(|p| exists(p)));
            } else {
                items.push(base);
            }
        }
    }
    drop_nested(dedupe(items))
}

/// Normalized keys of a target's `exclude_paths`.
pub fn exclusion_keys(t: &Target) -> Vec<String> {
    t.exclude_paths
        .iter()
        .flat_map(|s| resolve(s))
        .map(|p| norm_key(&p))
        .collect()
}

const APP_CACHE: &[&str] = &[
    "Cache", "Code Cache", "GPUCache", "DawnCache", "DawnGraphiteCache", "DawnWebGPUCache",
    "Service Worker\\CacheStorage", "Service Worker\\ScriptCache",
];
const SKIP_TOP: &[&str] = &["packages", "temp", "programs", "crashdumps"];

/// A `Cache` folder is only treated as a Chromium HTTP cache if it looks like one.
fn is_http_cache(p: &Path) -> bool {
    p.join("Cache_Data").is_dir() || p.join("index").is_file() || p.join("data_0").is_file()
}

fn app_cache_items(dir: &Path, out: &mut Vec<PathBuf>) {
    for sub in APP_CACHE {
        let p = join_rel(dir, sub);
        if !exists(&p) || (*sub == "Cache" && !is_http_cache(&p)) {
            continue;
        }
        out.push(p);
    }
}

fn scan_app_dir(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut names = HashSet::new();
    let mut subdirs = Vec::new();
    for e in rd.flatten() {
        let Ok(md) = e.metadata() else { continue };
        if is_link(&md) {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_lowercase();
        if md.is_dir() {
            subdirs.push((name.clone(), e.path()));
        }
        names.insert(name);
    }
    if depth > 0 {
        let own = dir.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let has_profiles = subdirs.iter().any(|(n, _)| n == "default" || n.starts_with("profile "));
        if own == "ebwebview" || (names.contains("local state") && has_profiles) {
            for profile in chromium_profiles(dir) {
                app_cache_items(&profile, out);
            }
            out.extend(CHROMIUM_ROOT_CACHE.iter().map(|s| join_rel(dir, s)).filter(|p| exists(p)));
            return;
        }
        if names.contains("code cache") || names.contains("gpucache") {
            app_cache_items(dir, out);
            return;
        }
    }
    if depth >= 3 {
        return;
    }
    for (name, path) in subdirs {
        if depth == 0 && SKIP_TOP.contains(&name.as_str()) {
            continue;
        }
        scan_app_dir(&path, depth + 1, out);
    }
}

/// Chromium/Electron/WebView2 caches anywhere in AppData that no other target covers.
pub fn discover_app_caches(roots: &[PathBuf], claimed: &[String]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for r in roots {
        scan_app_dir(r, 0, &mut found);
    }
    let found = drop_nested(dedupe(found));
    found
        .into_iter()
        .filter(|p| {
            let k = norm_key(p);
            !claimed.iter().any(|c| key_within(&k, c) || key_within(c, &k))
        })
        .collect()
}

pub fn app_cache_roots() -> Vec<PathBuf> {
    ["%APPDATA%", "%LOCALAPPDATA%", "%USERPROFILE%\\AppData\\LocalLow"]
        .iter()
        .filter_map(|s| expand_env(s))
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_electron_and_webview_caches() {
        let root = std::env::temp_dir().join(format!("scx-apps-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let mk = |rel: &str| fs::create_dir_all(join_rel(&root, rel)).unwrap();
        mk("SomeApp/Cache/Cache_Data");
        mk("SomeApp/Code Cache/js");
        mk("SomeApp/GPUCache");
        mk("SomeApp/IndexedDB");
        mk("NotCache/Cache"); // no Chromium markers -> ignored
        mk("Vendor/Tool/EBWebView/Default/Cache/Cache_Data");
        fs::write(join_rel(&root, "Vendor/Tool/EBWebView/Default/Preferences"), "{}").unwrap();
        mk("Claimed/Code Cache");

        let claimed = vec![norm_key(&root.join("Claimed").join("Code Cache"))];
        let found = discover_app_caches(&[root.clone()], &claimed);
        let rel: Vec<String> = found
            .iter()
            .map(|p| p.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        assert!(rel.contains(&"SomeApp/Cache".to_string()), "{rel:?}");
        assert!(rel.contains(&"SomeApp/Code Cache".to_string()));
        assert!(rel.contains(&"SomeApp/GPUCache".to_string()));
        assert!(rel.contains(&"Vendor/Tool/EBWebView/Default/Cache".to_string()), "{rel:?}");
        assert!(!rel.iter().any(|r| r.contains("IndexedDB") || r.starts_with("NotCache") || r.starts_with("Claimed")));
        let _ = fs::remove_dir_all(&root);
    }
}
