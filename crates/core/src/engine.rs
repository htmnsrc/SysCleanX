//! Orchestrates scanning and cleaning of targets.

use crate::fsops::{process_root, Ctx, Filter, Mode, Tally};
use crate::guard::Guard;
use crate::log::DeletionLog;
use crate::progress::Progress;
use crate::resolve::{app_cache_roots, discover_app_caches, exclusion_keys, target_items};
use crate::targets::{self, Resolver, Target};
use crate::util::norm_key;
use crate::{settings, special, sys};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Default)]
pub struct ItemScan {
    pub path: String,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct TargetScan {
    pub id: String,
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub size_known: bool,
    /// There is something to clean.
    pub present: bool,
    pub items: Vec<ItemScan>,
    /// Required apps that are currently running.
    pub blocked_by: Vec<String>,
    pub needs_admin: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TargetResult {
    pub id: String,
    pub name: String,
    pub tally: Tally,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CleanReport {
    pub results: Vec<TargetResult>,
    pub cancelled: bool,
    pub log_path: Option<PathBuf>,
}

impl CleanReport {
    pub fn total(&self) -> Tally {
        self.results.iter().fold(Tally::default(), |a, r| a + r.tally)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// User exclusions (raw paths).
    pub exclusions: Vec<String>,
    /// Schedule locked files for deletion at next boot.
    pub reboot_locked: bool,
    /// Skip targets whose app is running at clean time (re-checked, not taken from the scan).
    pub skip_running: bool,
    /// Target ids the user explicitly chose to clean even while their app is running.
    pub force: Vec<String>,
}

fn user_exclusions(raw: &[String]) -> Vec<String> {
    raw.iter()
        .map(|e| crate::util::expand_env(e.trim()).unwrap_or_else(|| e.trim().to_string()))
        .filter(|e| !e.is_empty())
        .map(|e| norm_key(Path::new(&e)))
        .collect()
}

pub struct Engine {
    pub targets: Vec<Target>,
    pub guard: Guard,
    pub admin: bool,
}

impl Engine {
    pub fn new(targets: Vec<Target>) -> Self {
        Engine { targets, guard: Guard::system(), admin: sys::is_admin() }
    }

    /// Built-in targets plus the user's custom-targets.toml.
    pub fn load() -> (Self, Vec<String>) {
        let custom = std::fs::read_to_string(settings::data_dir().join("custom-targets.toml")).ok();
        let (targets, warnings) = targets::load(custom.as_deref());
        (Self::new(targets), warnings)
    }

    pub fn target(&self, id: &str) -> Option<&Target> {
        self.targets.iter().find(|t| t.id == id)
    }

    pub fn default_selection(&self) -> Vec<String> {
        self.targets.iter().filter(|t| t.is_default_on()).map(|t| t.id.clone()).collect()
    }

    /// `claimed`: folders owned by specific targets; broad sweeps (pattern/empty-folder
    /// targets) skip them so nothing is counted or processed twice.
    fn filter(&self, t: &Target, user_exclusions: &[String], claimed: &[String]) -> Filter {
        let mut exclusions = user_exclusions.to_vec();
        exclusions.extend(exclusion_keys(t));
        let broad = t.skip_markers || t.empty_dirs;
        Filter {
            include: t.include.clone(),
            exclude: t.exclude.clone(),
            min_age: t.min_age(),
            recursive: t.recursive,
            skip_markers: t.skip_markers,
            empty_dirs: t.empty_dirs,
            exclusions,
            skip_dirs: if broad { claimed.to_vec() } else { Vec::new() },
        }
    }

    /// Concrete paths for every file-based target. Auto-detected app caches skip anything
    /// a specific target already covers, so nothing is counted twice.
    pub fn resolve_all(&self) -> (HashMap<String, Vec<PathBuf>>, Vec<String>) {
        let mut map: HashMap<String, Vec<PathBuf>> = self
            .targets
            .par_iter()
            .filter(|t| t.special.is_none() && t.resolver.is_none())
            .map(|t| (t.id.clone(), target_items(t)))
            .collect();
        let mut claimed: Vec<String> = self
            .targets
            .iter()
            .filter(|t| t.include.is_empty() && !t.empty_dirs)
            .filter_map(|t| map.get(&t.id))
            .flatten()
            .map(|p| norm_key(p))
            .collect();
        for t in &self.targets {
            if t.resolver == Some(Resolver::AppCaches) {
                let found = discover_app_caches(&app_cache_roots(), &claimed);
                claimed.extend(found.iter().map(|p| norm_key(p)));
                map.insert(t.id.clone(), found);
            }
        }
        (map, claimed)
    }

    fn blocked(t: &Target, running: &HashSet<String>) -> Vec<String> {
        t.processes.iter().filter(|p| running.contains(&p.to_lowercase())).cloned().collect()
    }

    pub fn scan(&self, ids: &[String], opts: &Options, progress: &Progress) -> Vec<TargetScan> {
        progress.set_label("Preparing…");
        let running = sys::process_names();
        let (resolved, claimed) = self.resolve_all();
        let user_excl = user_exclusions(&opts.exclusions);
        let log = DeletionLog::disabled();
        let now = SystemTime::now();
        progress.set_total(ids.len());
        ids.par_iter()
            .filter_map(|id| self.target(id))
            .map(|t| {
                progress.set_label(format!("Scanning {}", t.name));
                let s = self.scan_one(t, &resolved, &claimed, &user_excl, &running, &log, now, progress);
                progress.step();
                s
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn scan_one(
        &self,
        t: &Target,
        resolved: &HashMap<String, Vec<PathBuf>>,
        claimed: &[String],
        user_excl: &[String],
        running: &HashSet<String>,
        log: &DeletionLog,
        now: SystemTime,
        progress: &Progress,
    ) -> TargetScan {
        let mut s = TargetScan { id: t.id.clone(), size_known: true, ..Default::default() };
        s.blocked_by = Self::blocked(t, running);
        if t.admin && !self.admin {
            s.needs_admin = true;
            s.note = Some("Requires administrator".into());
            return s;
        }
        if let Some(sp) = t.special {
            let r = special::scan(t, sp, progress);
            s.bytes = r.bytes;
            s.files = r.count;
            s.size_known = r.size_known;
            s.present = r.present;
            s.note = r.note;
            s.items = r.details.into_iter().map(|(path, bytes)| ItemScan { path, files: 0, bytes }).collect();
            return s;
        }
        let f = self.filter(t, user_excl, claimed);
        let ctx = Ctx { guard: &self.guard, progress, log, now };
        let items = resolved.get(&t.id).map(Vec::as_slice).unwrap_or(&[]);
        let scans: Vec<(String, Tally)> = items
            .par_iter()
            .filter(|p| self.guard.check_purge(p, f.is_filtered()).is_ok())
            .map(|p| (p.display().to_string(), process_root(p, &f, Mode::Scan, t.remove_root, &ctx)))
            .collect();
        for (path, tally) in scans {
            s.files += tally.files;
            s.dirs += tally.dirs;
            s.bytes += tally.bytes;
            if tally.files > 0 || (f.empty_dirs && tally.dirs > 0) {
                s.items.push(ItemScan { path, files: tally.files + if f.empty_dirs { tally.dirs } else { 0 }, bytes: tally.bytes });
            }
        }
        if !f.empty_dirs {
            s.dirs = 0;
        }
        s.present = s.files > 0 || s.dirs > 0;
        s.items.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(b.files.cmp(&a.files)));
        s
    }

    pub fn clean(&self, ids: &[String], opts: &Options, progress: &Progress, log: &DeletionLog) -> CleanReport {
        progress.set_label("Preparing…");
        let running = sys::process_names();
        let (resolved, claimed) = self.resolve_all();
        let user_excl = user_exclusions(&opts.exclusions);
        let mode = Mode::Delete { reboot_locked: opts.reboot_locked };
        let mut report = CleanReport::default();
        progress.set_total(ids.len());
        for id in ids {
            if progress.is_cancelled() {
                report.cancelled = true;
                log.note("Cancelled by user");
                break;
            }
            let Some(t) = self.target(id) else { continue };
            progress.set_label(format!("Cleaning {}", t.name));
            log.note(&format!("== {} ==", t.name));
            let mut res = TargetResult { id: t.id.clone(), name: t.name.clone(), ..Default::default() };
            let blocked = Self::blocked(t, &running);
            if t.admin && !self.admin {
                res.error = Some("Requires administrator".into());
            } else if opts.skip_running && !blocked.is_empty() && !opts.force.contains(&t.id) {
                res.error = Some(format!("Skipped: close {} first", blocked.join(", ")));
            } else {
                let mut restart = Vec::new();
                for svc in &t.services {
                    match sys::stop_service(svc) {
                        Ok(true) => restart.push(svc.clone()),
                        Ok(false) => {}
                        Err(e) => log.note(&format!("Could not stop {svc}: {e}")),
                    }
                }
                let r = match t.special {
                    Some(sp) => special::clean(t, sp, progress, log),
                    None => {
                        let f = self.filter(t, &user_excl, &claimed);
                        let ctx = Ctx { guard: &self.guard, progress, log, now: SystemTime::now() };
                        let mut tally = Tally::default();
                        for item in resolved.get(&t.id).map(Vec::as_slice).unwrap_or(&[]) {
                            match self.guard.check_purge(item, f.is_filtered()) {
                                Ok(()) => tally += process_root(item, &f, mode, t.remove_root, &ctx),
                                Err(e) => log.skipped(item, &e),
                            }
                        }
                        Ok(tally)
                    }
                };
                for svc in restart {
                    if let Err(e) = sys::start_service(&svc) {
                        log.note(&e);
                    }
                }
                match r {
                    Ok(tally) => res.tally = tally,
                    Err(e) => {
                        log.note(&format!("Error: {e}"));
                        res.error = Some(e);
                    }
                }
            }
            report.results.push(res);
            progress.step();
        }
        let total = report.total();
        log.note(&format!(
            "Done: {} files, {} bytes freed, {} skipped, {} scheduled for reboot",
            total.files, total.bytes, total.skipped, total.reboot
        ));
        log.flush();
        report.log_path = log.path.clone();
        report
    }
}
