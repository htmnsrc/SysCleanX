//! Data-driven cleaning targets. Built-ins come from `targets.toml` (embedded at build
//! time); users can add more in `%APPDATA%\SysCleanX\custom-targets.toml`.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Safe,
    Recommended,
    Expert,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Safe => "Safe",
            Level::Recommended => "Recommended",
            Level::Expert => "Expert",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Special {
    RecycleBin,
    ExplorerMru,
    Cleanmgr,
    Dism,
    EventLogs,
    RestorePoints,
    InstallerOrphans,
    OldDrivers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Resolver {
    /// Auto-detect Chromium/Electron/WebView2 caches of apps under AppData.
    AppCaches,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub group: String,
    #[serde(default)]
    pub description: String,
    pub level: Level,
    /// Needs administrator rights.
    #[serde(default)]
    pub admin: bool,
    /// Path specs (`%ENV%` + wildcards). Folders are emptied, files deleted.
    #[serde(default)]
    pub paths: Vec<String>,
    /// For Chromium "User Data" roots: sub-paths inside every profile folder.
    #[serde(default)]
    pub profile_subpaths: Vec<String>,
    /// Sub-paths relative to each resolved path (used instead of the path itself).
    #[serde(default)]
    pub subpaths: Vec<String>,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Path specs never entered (e.g. folders already covered by another target).
    #[serde(default)]
    pub exclude_paths: Vec<String>,
    #[serde(default)]
    pub min_age_hours: u64,
    #[serde(default = "yes")]
    pub recursive: bool,
    #[serde(default)]
    pub remove_root: bool,
    #[serde(default)]
    pub skip_markers: bool,
    #[serde(default)]
    pub empty_dirs: bool,
    /// Process names that must not be running (e.g. `chrome.exe`).
    #[serde(default)]
    pub processes: Vec<String>,
    /// Services stopped during cleaning and restarted afterwards.
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub special: Option<Special>,
    /// Disk Cleanup handler names for `special = "cleanmgr"`.
    #[serde(default)]
    pub cleanmgr: Vec<String>,
    #[serde(default)]
    pub resolver: Option<Resolver>,
    /// Override the default selection (default: on unless Expert).
    #[serde(default)]
    pub default_on: Option<bool>,
}

impl Target {
    pub fn is_default_on(&self) -> bool {
        self.default_on.unwrap_or(self.level != Level::Expert)
    }
    pub fn min_age(&self) -> Option<std::time::Duration> {
        (self.min_age_hours > 0).then(|| std::time::Duration::from_secs(self.min_age_hours * 3600))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Chromium {
    id: String,
    name: String,
    user_data: Vec<String>,
    process: String,
}

#[derive(Debug, Default, Deserialize)]
struct TargetFile {
    #[serde(default)]
    target: Vec<Target>,
    #[serde(default)]
    chromium: Vec<Chromium>,
}

pub const BUILTIN: &str = include_str!("targets.toml");

/// Per-profile cache folders of Chromium-based browsers and apps.
pub const CHROMIUM_CACHE: &[&str] = &[
    "Cache", "Code Cache", "GPUCache", "DawnCache", "DawnGraphiteCache", "DawnWebGPUCache",
    "Service Worker\\CacheStorage", "Service Worker\\ScriptCache", "Application Cache", "Media Cache",
];
/// Caches that live directly in a Chromium "User Data" folder.
pub const CHROMIUM_ROOT_CACHE: &[&str] = &[
    "ShaderCache", "GrShaderCache", "GraphiteDawnCache", "Crashpad\\reports", "component_crx_cache",
];
const CHROMIUM_HISTORY: &[&str] = &[
    "History", "History-journal", "Visited Links", "Top Sites", "Top Sites-journal", "Shortcuts",
    "Shortcuts-journal", "Network Action Predictor", "Network Action Predictor-journal",
];
const CHROMIUM_COOKIES: &[&str] = &["Network\\Cookies", "Network\\Cookies-journal", "Cookies", "Cookies-journal"];

fn strings(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn chromium_targets(c: &Chromium) -> (Target, Vec<Target>) {
    let base = Target {
        id: c.id.clone(),
        name: c.name.clone(),
        group: "Browsers".into(),
        description: format!("{} cache, code cache, GPU and shader caches for every profile.", c.name),
        level: Level::Safe,
        admin: false,
        paths: c.user_data.clone(),
        profile_subpaths: strings(CHROMIUM_CACHE),
        subpaths: strings(CHROMIUM_ROOT_CACHE),
        include: vec![],
        exclude: vec![],
        exclude_paths: vec![],
        min_age_hours: 0,
        recursive: true,
        remove_root: false,
        skip_markers: false,
        empty_dirs: false,
        processes: vec![c.process.clone()],
        services: vec![],
        special: None,
        cleanmgr: vec![],
        resolver: None,
        default_on: None,
    };
    let history = Target {
        id: format!("{}-history", c.id),
        name: format!("{} browsing history", c.name),
        group: "Privacy".into(),
        description: "Visited pages, typed URLs and top sites. Bookmarks and passwords are kept.".into(),
        level: Level::Recommended,
        profile_subpaths: strings(CHROMIUM_HISTORY),
        subpaths: vec![],
        default_on: Some(false),
        ..base.clone()
    };
    let cookies = Target {
        id: format!("{}-cookies", c.id),
        name: format!("{} cookies", c.name),
        group: "Privacy".into(),
        description: "Signs you out of websites.".into(),
        level: Level::Expert,
        profile_subpaths: strings(CHROMIUM_COOKIES),
        subpaths: vec![],
        ..base.clone()
    };
    (base, vec![history, cookies])
}

/// Parse a targets file. Chromium entries expand into cache/history/cookie targets.
pub fn parse(text: &str) -> Result<Vec<Target>, String> {
    let file: TargetFile = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut targets = file.target;
    let mut caches = Vec::new();
    let mut privacy = Vec::new();
    for c in &file.chromium {
        let (cache, extra) = chromium_targets(c);
        caches.push(cache);
        privacy.extend(extra);
    }
    let browsers_at = targets.iter().position(|t| t.group == "Browsers").unwrap_or(targets.len());
    targets.splice(browsers_at..browsers_at, caches);
    let privacy_at = targets.iter().position(|t| t.group == "Privacy").unwrap_or(targets.len());
    targets.splice(privacy_at..privacy_at, privacy);
    Ok(targets)
}

/// Built-in targets plus optional user targets. Duplicate ids in user data are skipped.
pub fn load(custom: Option<&str>) -> (Vec<Target>, Vec<String>) {
    let mut warnings = Vec::new();
    let mut targets = parse(BUILTIN).expect("built-in targets.toml must parse");
    if let Some(text) = custom {
        match parse(text) {
            Ok(extra) => {
                let mut ids: HashSet<String> = targets.iter().map(|t| t.id.clone()).collect();
                for t in extra {
                    if ids.insert(t.id.clone()) {
                        targets.push(t);
                    } else {
                        warnings.push(format!("custom target '{}' ignored: duplicate id", t.id));
                    }
                }
            }
            Err(e) => warnings.push(format!("custom-targets.toml: {e}")),
        }
    }
    (targets, warnings)
}

/// Groups in first-appearance order.
pub fn groups(targets: &[Target]) -> Vec<String> {
    let mut seen = HashSet::new();
    targets.iter().filter(|t| seen.insert(t.group.clone())).map(|t| t.group.clone()).collect()
}

pub const CUSTOM_TEMPLATE: &str = r#"# SysCleanX custom targets. Same format as the built-in list.
# Paths support %ENV% variables and * / ? wildcards. Folders are emptied, files deleted.
# Everything still goes through the safety guard (system and user folders are protected).
#
# [[target]]
# id = "my-app-cache"
# name = "My App cache"
# group = "Applications"
# level = "safe"            # safe | recommended | expert
# description = "Cache of My App"
# paths = ['%LOCALAPPDATA%\MyApp\Cache']
# processes = ["myapp.exe"] # skipped while running
# min_age_hours = 0
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_targets_are_valid() {
        let targets = parse(BUILTIN).expect("parse");
        assert!(targets.len() > 50, "only {} targets", targets.len());
        let mut ids = HashSet::new();
        for t in &targets {
            assert!(ids.insert(t.id.clone()), "duplicate id {}", t.id);
            assert!(!t.name.is_empty() && !t.group.is_empty());
            let has_work = !t.paths.is_empty() || t.special.is_some() || t.resolver.is_some();
            assert!(has_work, "target {} does nothing", t.id);
            if t.special == Some(Special::Cleanmgr) {
                assert!(!t.cleanmgr.is_empty(), "{} needs cleanmgr handlers", t.id);
            }
            for p in &t.paths {
                assert!(p.starts_with('%'), "{}: path must start with an env var: {p}", t.id);
            }
        }
        assert!(ids.contains("chrome") && ids.contains("chrome-history") && ids.contains("edge-cookies"));
        let g = groups(&targets);
        assert_eq!(g.first().map(String::as_str), Some("System"));
    }

    #[test]
    fn custom_duplicates_are_rejected() {
        let custom = "[[target]]\nid='user-temp'\nname='x'\ngroup='y'\nlevel='safe'\npaths=['%TEMP%']\n";
        let (_, warnings) = load(Some(custom));
        assert_eq!(warnings.len(), 1);
        let (_, warnings) = load(Some("not toml ["));
        assert_eq!(warnings.len(), 1);
    }
}
