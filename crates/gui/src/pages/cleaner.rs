use crate::job::{take, Job};
use crate::shared::Shared;
use crate::theme::{self, badge, card, muted, pill, pill_sized, semibold, Kind};
use eframe::egui::{self, collapsing_header::CollapsingState, Align, Id, Layout, RichText, Vec2};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use syscleanx_core::engine::Options;
use syscleanx_core::sys::Stopped;
use syscleanx_core::log::DeletionLog;
use syscleanx_core::targets::{groups, Level, Special, Target};
use syscleanx_core::{format_bytes, format_count, settings, sys, CleanReport, TargetScan};

/// Specials that are slow to measure; only scanned once the user selects them.
fn is_slow(t: &Target) -> bool {
    matches!(t.special, Some(Special::OldDrivers | Special::InstallerOrphans | Special::RestorePoints))
}

/// One-line summary under each category name.
fn group_blurb(name: &str, rows: &[&Target]) -> String {
    match name {
        "System" => "Temp files, update caches, error reports and logs".into(),
        "Browsers" => "Caches of every browser profile".into(),
        "Applications" => "Caches and logs of apps like Discord, Spotify and Steam".into(),
        "Developer" => "Package manager and build caches".into(),
        "Deep Clean" => "Auto-detected app caches, stray temp and old log files".into(),
        "Privacy" => "Browsing history, cookies and recent-file lists".into(),
        "Advanced" => "Powerful system cleanups for experienced users".into(),
        _ => rows.iter().take(3).map(|t| t.name.as_str()).collect::<Vec<_>>().join(", "),
    }
}

/// Windows normally needs well under a minute; give up (and offer to continue) after this.
const RESTORE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

enum CleanOutcome {
    Done(CleanReport),
    RestoreFailed(String),
    /// Cancelled before anything was deleted.
    Cancelled,
}

/// Lets the UI skip a slow restore point without cancelling the clean.
#[derive(Default)]
struct RestoreCtl {
    active: AtomicBool,
    skip: AtomicBool,
}

pub struct Cleaner {
    scan: Option<HashMap<String, TargetScan>>,
    scan_job: Option<Job<(bool, Vec<TargetScan>)>>,
    clean_job: Option<Job<CleanOutcome>>,
    pending: Vec<String>,
    selected: HashSet<String>,
    /// Targets whose app is running but that the user chose to clean anyway.
    overrides: HashSet<String>,
    confirm: bool,
    report: Option<CleanReport>,
    show_report: bool,
    restore_error: Option<String>,
    scan_stopped: bool,
    started: bool,
    restore_ctl: Arc<RestoreCtl>,
    /// The user picked "Custom" (kept until they pick Safe or Recommended).
    custom_mode: bool,
}

impl Cleaner {
    pub fn new(shared: &Shared) -> Self {
        let selected = match &shared.settings.selection {
            Some(s) => s.iter().cloned().collect(),
            None => shared.engine.default_selection().into_iter().collect(),
        };
        Cleaner {
            scan: None,
            scan_job: None,
            clean_job: None,
            pending: Vec::new(),
            selected,
            overrides: HashSet::new(),
            confirm: false,
            report: None,
            show_report: false,
            restore_error: None,
            scan_stopped: false,
            started: false,
            restore_ctl: Arc::new(RestoreCtl::default()),
            custom_mode: false,
        }
    }

    /// Debug builds only: fill with sample results for UI screenshots.
    #[cfg(debug_assertions)]
    pub fn demo(&mut self, shared: &Shared, ctx: &egui::Context) {
        use syscleanx_core::engine::ItemScan;
        self.started = true;
        let mut map = HashMap::new();
        for (i, t) in shared.engine.targets.iter().enumerate() {
            let present = i % 3 != 2;
            let bytes = if present { ((i as u64 * 7919) % 900 + 3) * 1_048_576 / (1 + i as u64 % 4) } else { 0 };
            let blocked_by = if t.id == "chrome" { vec!["chrome.exe".to_string()] } else { vec![] };
            let items = vec![ItemScan { path: format!("C:\\Users\\demo\\AppData\\Local\\{}\\Cache", t.name), files: 12, bytes }];
            map.insert(t.id.clone(), TargetScan { id: t.id.clone(), files: bytes / 40_000 + 1, bytes, size_known: true, present, blocked_by, items, ..Default::default() });
        }
        self.scan = Some(map);
        if std::env::var("SYSCLEANX_DEMO_OPEN").is_ok() {
            for g in ["System", "Browsers"] {
                let mut s = CollapsingState::load_with_default_open(ctx, Id::unique(("cleaner-group", g)), false);
                s.set_open(true);
                s.store(ctx);
            }
        }
    }

    fn busy(&self) -> bool {
        self.scan_job.is_some() || self.clean_job.is_some()
    }

    fn scan_of(&self, id: &str) -> Option<&TargetScan> {
        self.scan.as_ref()?.get(id)
    }

    /// Can the user tick this target right now?
    fn selectable(&self, t: &Target, shared: &Shared) -> bool {
        !(t.admin && !shared.admin) && self.scan_of(&t.id).map(|s| s.present && !s.needs_admin).unwrap_or(true)
    }

    fn is_checked(&self, t: &Target) -> bool {
        if !self.selected.contains(&t.id) {
            return false;
        }
        match self.scan_of(&t.id) {
            None => true,
            Some(s) => s.present && !s.needs_admin && (s.blocked_by.is_empty() || self.overrides.contains(&t.id)),
        }
    }

    fn persist(&self, shared: &mut Shared) {
        let mut v: Vec<String> = self.selected.iter().cloned().collect();
        v.sort();
        shared.settings.selection = Some(v);
        shared.save();
    }

    fn set_checked(&mut self, t: &Target, on: bool) {
        let blocked = self.scan_of(&t.id).map(|s| !s.blocked_by.is_empty()).unwrap_or(false);
        if on {
            self.selected.insert(t.id.clone());
            if blocked {
                self.overrides.insert(t.id.clone());
            }
            if self.scan.is_some() && self.scan_of(&t.id).is_none() && !self.pending.contains(&t.id) {
                self.pending.push(t.id.clone());
            }
        } else if !(blocked && self.overrides.remove(&t.id)) {
            self.selected.remove(&t.id);
        }
    }

    fn preset(&mut self, shared: &mut Shared, pick: impl Fn(&Target) -> bool) {
        self.selected = shared.engine.targets.iter().filter(|t| pick(t)).map(|t| t.id.clone()).collect();
        self.overrides.clear();
        self.custom_mode = false;
        self.persist(shared);
    }

    /// 0 = Safe, 1 = Recommended, 2 = Custom.
    fn current_preset(&self, shared: &Shared) -> usize {
        if self.custom_mode {
            return 2;
        }
        let pick = |f: &dyn Fn(&Target) -> bool| -> HashSet<String> {
            shared.engine.targets.iter().filter(|t| f(t)).map(|t| t.id.clone()).collect()
        };
        if self.selected == pick(&|t| t.level == Level::Safe) {
            0
        } else if self.selected == pick(&|t| t.is_default_on()) {
            1
        } else {
            2
        }
    }

    fn start_scan(&mut self, ctx: &egui::Context, shared: &Shared, ids: Option<Vec<String>>) {
        let full = ids.is_none();
        let ids = ids.unwrap_or_else(|| {
            shared
                .engine
                .targets
                .iter()
                .filter(|t| !is_slow(t) || self.selected.contains(&t.id))
                .map(|t| t.id.clone())
                .collect()
        });
        let engine = shared.engine.clone();
        let opts = Options { exclusions: shared.settings.exclusions.clone(), ..Default::default() };
        self.scan_stopped = false;
        self.scan_job = Some(Job::spawn(ctx, move |p| (full, engine.scan(&ids, &opts, p))));
    }

    fn checked_ids(&self, shared: &Shared) -> Vec<String> {
        shared.engine.targets.iter().filter(|t| self.is_checked(t)).map(|t| t.id.clone()).collect()
    }

    fn start_clean(&mut self, ctx: &egui::Context, shared: &Shared, with_restore_point: bool) {
        let ids = self.checked_ids(shared);
        let engine = shared.engine.clone();
        let restore = with_restore_point && shared.settings.restore_point && shared.admin;
        let opts = Options {
            exclusions: shared.settings.exclusions.clone(),
            reboot_locked: shared.settings.reboot_locked,
            skip_running: true,
            force: self.overrides.iter().cloned().collect(),
        };
        let ctl = self.restore_ctl.clone();
        ctl.skip.store(false, Ordering::Relaxed);
        self.clean_job = Some(Job::spawn(ctx, move |p| {
            if restore {
                ctl.active.store(true, Ordering::Relaxed);
                let start = Instant::now();
                let result = sys::create_restore_point("SysCleanX", RESTORE_TIMEOUT, || {
                    let s = start.elapsed().as_secs();
                    p.set_label(format!("Creating a restore point… {}:{:02}  ·  Windows can take a few minutes", s / 60, s % 60));
                    !p.is_cancelled() && !ctl.skip.load(Ordering::Relaxed)
                });
                ctl.active.store(false, Ordering::Relaxed);
                match result {
                    Ok(()) => {}
                    // Stopped by "Skip restore point": carry on cleaning without one.
                    Err(Stopped::Cancelled) if !p.is_cancelled() => {}
                    Err(Stopped::Cancelled) => return CleanOutcome::Cancelled,
                    Err(Stopped::TimedOut) => {
                        return CleanOutcome::RestoreFailed(format!(
                            "Windows didn't finish creating the restore point within {} minutes. It may be busy, for example installing updates.",
                            RESTORE_TIMEOUT.as_secs() / 60
                        ))
                    }
                    Err(Stopped::Failed(e)) => return CleanOutcome::RestoreFailed(e),
                }
            }
            let log = DeletionLog::create(&settings::logs_dir(), "clean");
            CleanOutcome::Done(engine.clean(&ids, &opts, p, &log))
        }));
    }

    /// Runs every frame, whichever page is visible, so scans and cleans finish in the
    /// background (and their toasts appear) even while the user is elsewhere.
    pub fn tick(&mut self, ctx: &egui::Context, shared: &mut Shared) {
        if !self.started {
            self.started = true;
            self.start_scan(ctx, shared, None);
        }
        if let Some(job) = &self.scan_job {
            if job.progress.is_cancelled() {
                self.scan_stopped = true;
            }
        }
        if let Some(r) = take(&mut self.scan_job) {
            match r {
                Ok((full, results)) => {
                    let map = self.scan.get_or_insert_with(HashMap::new);
                    if full {
                        map.clear();
                    }
                    for s in results {
                        map.insert(s.id.clone(), s);
                    }
                }
                Err(e) => shared.error(e),
            }
        }
        if let Some(r) = take(&mut self.clean_job) {
            match r {
                Ok(CleanOutcome::Done(report)) => {
                    let skipped = report.results.iter().any(|r| r.error.is_some()) || report.total().skipped > 0;
                    let freed = report.total().bytes;
                    if freed > 0 || !report.cancelled {
                        shared.record_clean(freed);
                    }
                    if report.cancelled {
                        shared.notify(format!("Stopped. Freed {} before you cancelled.", format_bytes(freed)));
                    } else if skipped {
                        shared.notify(format!("Freed {}. Some items were skipped — see Details.", format_bytes(freed)));
                    } else {
                        shared.notify(format!("Freed {}.", format_bytes(freed)));
                    }
                    self.report = Some(report);
                    self.overrides.clear();
                    self.start_scan(ctx, shared, None);
                }
                Ok(CleanOutcome::RestoreFailed(e)) => self.restore_error = Some(e),
                Ok(CleanOutcome::Cancelled) => shared.notify("Cancelled. Nothing was deleted."),
                Err(e) => shared.error(e),
            }
        }
        if !self.busy() && !self.pending.is_empty() {
            let ids = std::mem::take(&mut self.pending);
            self.start_scan(ctx, shared, Some(ids));
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut Shared) {
        let ctx = ui.ctx().clone();
        let p = shared.pal;
        let engine = shared.engine.clone();

        let busy = self.busy();
        theme::page_title(ui, &p, "Clean", "Remove junk files, caches and leftovers safely.", |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                let current = self.current_preset(shared);
                if let Some(i) = theme::segmented(ui, &p, "preset", &["Safe", "Recommended", "Custom"], Some(current)) {
                    match i {
                        0 => self.preset(shared, |t| t.level == Level::Safe),
                        1 => self.preset(shared, |t| t.is_default_on()),
                        _ => {
                            // "Custom" stays selected and opens every category so items can be
                            // picked by hand.
                            self.custom_mode = true;
                            for g in groups(&engine.targets) {
                                let mut s = CollapsingState::load_with_default_open(&ctx, Id::unique(("cleaner-group", g.as_str())), false);
                                s.set_open(true);
                                s.store(&ctx);
                            }
                        }
                    }
                }
            });
        });

        let checked: Vec<&Target> = engine.targets.iter().filter(|t| self.is_checked(t)).collect();
        let selected_bytes: u64 = checked.iter().filter_map(|t| self.scan_of(&t.id)).map(|s| s.bytes).sum();
        let total_bytes: u64 = self.scan.as_ref().map(|m| m.values().map(|s| s.bytes).sum()).unwrap_or(0);
        let can_clean = !busy && self.scan.is_some() && !checked.is_empty();
        let n_checked = checked.len();

        self.hero(ui, shared, selected_bytes, total_bytes, n_checked, can_clean);
        ui.add_space(18.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new("What to clean").font(semibold(14.0)).color(p.muted));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| theme::level_legend(ui, &p));
        });
        ui.add_space(4.0);

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            card(&p).inner_margin(egui::Margin::symmetric(6, 6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                for group in groups(&engine.targets) {
                    let rows: Vec<&Target> = engine.targets.iter().filter(|t| t.group == group && self.visible(t, shared)).collect();
                    if !rows.is_empty() {
                        self.group(ui, &group, &rows, total_bytes, shared);
                    }
                }
                if self.scan.is_some() {
                    let hidden = engine.targets.iter().filter(|t| !self.visible(t, shared)).count();
                    let label = if hidden > 0 {
                        Some(format!("Show {hidden} items with nothing to clean"))
                    } else if shared.settings.show_empty {
                        Some("Hide items with nothing to clean".to_string())
                    } else {
                        None
                    };
                    if let Some(label) = label {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.add_space(6.0);
                            if pill_sized(ui, &p, &label, Kind::Ghost, true, 26.0, 12.5).clicked() {
                                shared.settings.show_empty = hidden > 0;
                                shared.save();
                            }
                        });
                    }
                }
            });
            ui.add_space(12.0);
        });

        self.confirm_modal(&ctx, shared);
        self.restore_modal(&ctx, shared);
        self.report_modal(&ctx, shared);
    }

    fn hero(&mut self, ui: &mut egui::Ui, shared: &mut Shared, selected_bytes: u64, total_bytes: u64, n_checked: usize, can_clean: bool) {
        let p = shared.pal;
        let ctx = ui.ctx().clone();
        card(&p).inner_margin(egui::Margin::symmetric(24, 20)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    if self.clean_job.is_some() && self.restore_ctl.active.load(Ordering::Relaxed) {
                        ui.label(RichText::new("Creating restore point…").font(semibold(32.0)).color(p.accent));
                        ui.label(muted(&p, "Nothing is deleted until it's ready. You can skip it or cancel."));
                    } else if let Some(job) = &self.clean_job {
                        ui.label(RichText::new("Cleaning…").font(semibold(32.0)).color(p.accent));
                        ui.label(muted(&p, format!("{} removed so far", format_bytes(job.progress.bytes()))));
                    } else if self.scan.is_none() {
                        let found = self.scan_job.as_ref().map(|j| j.progress.bytes()).unwrap_or(0);
                        ui.label(RichText::new("Scanning…").font(semibold(32.0)).color(p.accent));
                        ui.label(muted(&p, format!("{} of junk found so far", format_bytes(found))));
                    } else {
                        ui.horizontal(|ui| {
                            let big = theme::animated_bytes(ui, Id::unique("hero-bytes"), selected_bytes);
                            ui.label(RichText::new(big).font(semibold(36.0)).color(p.accent));
                            ui.label(RichText::new("ready to clean").size(15.0).color(p.muted));
                        });
                        let mut sub = format!("{} found in total · {} of {} items selected", format_bytes(total_bytes), n_checked, self.visible_count(shared));
                        if self.scan_stopped {
                            sub.push_str(" · scan stopped early");
                        }
                        ui.label(muted(&p, sub).size(13.0));
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if pill_sized(ui, &p, "Clean now", Kind::Primary, can_clean, 46.0, 16.0).clicked() {
                        self.confirm = true;
                    }
                    if pill_sized(ui, &p, "Rescan", Kind::Ghost, !self.busy(), 46.0, 14.0).clicked() {
                        self.start_scan(&ctx, shared, None);
                    }
                });
            });

            let progress = self.clean_job.as_ref().map(|j| j.progress.clone()).or_else(|| self.scan_job.as_ref().map(|j| j.progress.clone()));
            ui.add_space(14.0);
            if let Some(pr) = progress {
                let restoring = self.clean_job.is_some() && self.restore_ctl.active.load(Ordering::Relaxed);
                let (_, skip) = theme::progress_row_ext(ui, &p, &pr, false, restoring.then_some("Skip restore point"));
                if skip {
                    self.restore_ctl.skip.store(true, Ordering::Relaxed);
                }
            } else if let Some(d) = shared.drive.clone() {
                theme::drive_bar(ui, &p, d.total, d.free, selected_bytes);
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Drive {}", d.label)).font(semibold(12.5)));
                    ui.label(muted(&p, format!("{} free of {}", format_bytes(d.free), format_bytes(d.total))).size(12.5));
                    if selected_bytes > 0 {
                        ui.label(muted(&p, "·").size(12.5));
                        ui.label(RichText::new(format!("+{} after cleaning", format_bytes(selected_bytes))).size(12.5).color(p.accent));
                    }
                });
                ui.add_space(4.0);
            }
            ui.separator();
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let (changed, label) =
                    theme::toggle_row(ui, &p, &mut shared.settings.restore_point, "Create a restore point first", shared.admin && !self.busy());
                if changed {
                    shared.save();
                }
                label.on_hover_text(if shared.admin {
                    "A safety net for system changes. Takes about a minute and needs System Protection turned on."
                } else {
                    "Needs administrator rights."
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.report.is_some() && pill_sized(ui, &p, "Details", Kind::Ghost, true, 24.0, 12.5).clicked() {
                        self.show_report = true;
                    }
                    let stats = match shared.settings.last_clean {
                        Some(t) => format!("Last clean {} · {} freed in total", theme::ago(t), format_bytes(shared.settings.total_freed)),
                        None => "Not cleaned yet".to_string(),
                    };
                    ui.label(muted(&p, stats).size(12.5));
                });
            });
        });
    }

    fn visible(&self, t: &Target, shared: &Shared) -> bool {
        if self.scan.is_none() || shared.settings.show_empty {
            return true;
        }
        match self.scan_of(&t.id) {
            Some(s) => s.present || s.needs_admin,
            None => is_slow(t),
        }
    }

    fn visible_count(&self, shared: &Shared) -> usize {
        shared.engine.targets.iter().filter(|t| self.visible(t, shared)).count()
    }

    fn group(&mut self, ui: &mut egui::Ui, name: &str, rows: &[&Target], total_bytes: u64, shared: &mut Shared) {
        let p = shared.pal;
        let ctx = ui.ctx().clone();
        let mut state = CollapsingState::load_with_default_open(&ctx, Id::unique(("cleaner-group", name)), false);
        let openness = state.openness(&ctx);
        let busy = self.busy();
        let toggleable: Vec<&Target> = rows.iter().copied().filter(|t| self.selectable(t, shared)).collect();
        let on = toggleable.iter().filter(|t| self.is_checked(t)).count();
        let all = on > 0 && on == toggleable.len();
        let bytes: u64 = rows.iter().filter_map(|t| self.scan_of(&t.id)).map(|s| s.bytes).sum();
        let blurb = group_blurb(name, rows);
        let mut group_click = false;

        let header = theme::hover_row(ui, &p, 56.0, |ui| {
            if theme::check(ui, &p, all, on > 0 && !all, !busy && !toggleable.is_empty()).clicked() {
                group_click = true;
            }
            ui.add_space(8.0);
            let text_w = (ui.available_width() - 190.0).max(120.0);
            ui.allocate_ui_with_layout(Vec2::new(text_w, 40.0), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(name).font(semibold(15.0)));
                ui.add(egui::Label::new(muted(&p, format!("{on} of {} selected · {blurb}", rows.len())).size(12.5)).truncate());
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                theme::chevron(ui, p.muted, openness);
                ui.add_space(2.0);
                if self.scan.is_some() {
                    let share = if total_bytes > 0 { bytes as f32 / total_bytes as f32 } else { 0.0 };
                    theme::mini_bar(ui, &p, share, if on > 0 { p.accent } else { p.faint }, 52.0);
                    ui.add_space(4.0);
                    ui.label(RichText::new(format_bytes(bytes)).font(semibold(14.0)).color(if on > 0 { p.text } else { p.muted }));
                }
            });
        });
        if header.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            state.toggle(ui);
        }
        if group_click {
            // Ticking a whole group never pulls in expert items (unless the group is all expert)
            // or apps that are currently open.
            let only_expert = toggleable.iter().all(|t| t.level == Level::Expert);
            for t in &toggleable {
                if all {
                    self.set_checked(t, false);
                } else if only_expert || t.level != Level::Expert {
                    let blocked = self.scan_of(&t.id).map(|s| !s.blocked_by.is_empty()).unwrap_or(false);
                    if !blocked {
                        self.set_checked(t, true);
                    }
                }
            }
            self.persist(shared);
        }
        state.show_body_unindented(ui, |ui| {
            for t in rows {
                self.row(ui, t, shared);
            }
            ui.add_space(6.0);
        });
    }

    fn row(&mut self, ui: &mut egui::Ui, t: &Target, shared: &mut Shared) {
        let p = shared.pal;
        let s = self.scan_of(&t.id).cloned();
        let needs_admin = t.admin && !shared.admin;
        let enabled = !self.busy() && self.selectable(t, shared);
        let on = self.is_checked(t);
        let blocked = s.as_ref().map(|s| s.blocked_by.join(", ")).filter(|b| !b.is_empty());
        let mut toggled = false;
        let resp = theme::hover_row(ui, &p, 48.0, |ui| {
            ui.add_space(26.0);
            if theme::check(ui, &p, on, false, enabled).clicked() {
                toggled = true;
            }
            ui.add_space(8.0);
            let text_w = (ui.available_width() - 120.0).max(120.0);
            ui.allocate_ui_with_layout(Vec2::new(text_w, 38.0), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.horizontal(|ui| {
                    theme::dot(ui, theme::level_color(&p, t.level)).on_hover_text(t.level.label());
                    let name = RichText::new(&t.name).size(14.0);
                    ui.label(if enabled || on { name } else { name.color(p.muted) });
                    if t.level == Level::Expert {
                        badge(ui, "Expert", p.amber);
                    }
                    if needs_admin {
                        badge(ui, "Admin", p.muted);
                    }
                    if blocked.is_some() {
                        badge(ui, "In use", p.amber);
                    }
                });
                let caption = match (&blocked, &s) {
                    (Some(apps), _) => format!("{apps} is running — tick to clean anyway (files in use are skipped)"),
                    (None, Some(s)) if s.note.is_some() && t.description.is_empty() => s.note.clone().unwrap_or_default(),
                    _ => t.description.clone(),
                };
                if !caption.is_empty() {
                    ui.add(egui::Label::new(RichText::new(caption).size(12.0).color(p.faint)).truncate());
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(20.0);
                let (text, strong) = match &s {
                    None if is_slow(t) && self.scan.is_some() => ("tick to measure".to_string(), false),
                    None => ("—".to_string(), false),
                    Some(s) if s.needs_admin => ("needs admin".to_string(), false),
                    Some(s) if !s.present => ("nothing to clean".to_string(), false),
                    Some(s) if s.bytes == 0 && !s.size_known => ("size unknown".to_string(), false),
                    Some(s) if !s.size_known => (format!("~{}", format_bytes(s.bytes)), true),
                    Some(s) => (format_bytes(s.bytes), true),
                };
                let rt = RichText::new(text).size(13.5);
                ui.label(if strong && on { rt.font(semibold(13.5)) } else { rt.color(p.muted) });
            });
        });
        if enabled && resp.clicked() {
            toggled = true;
        }
        if toggled {
            self.set_checked(t, !on);
            self.persist(shared);
        }
        resp.on_hover_ui(|ui| {
            ui.set_max_width(400.0);
            if let Some(s) = &s {
                if s.files > 0 {
                    let unit = if t.special.is_some() { "items" } else { "files" };
                    ui.label(format!("{} {unit} · {}", format_count(s.files), format_bytes(s.bytes)));
                }
                if let Some(note) = &s.note {
                    ui.label(muted(&p, note));
                }
                for item in s.items.iter().take(5) {
                    ui.label(RichText::new(&item.path).monospace().size(11.5).color(p.muted));
                }
                if s.items.len() > 5 {
                    ui.label(muted(&p, format!("…and {} more locations", s.items.len() - 5)).size(11.5));
                }
            } else {
                ui.label(&t.description);
            }
        });
    }

    fn confirm_modal(&mut self, ctx: &egui::Context, shared: &mut Shared) {
        if !self.confirm {
            return;
        }
        let p = shared.pal;
        let checked: Vec<Target> = shared.engine.targets.iter().filter(|t| self.is_checked(t)).cloned().collect();
        let bytes: u64 = checked.iter().filter_map(|t| self.scan_of(&t.id)).map(|s| s.bytes).sum();
        let expert: Vec<&Target> = checked.iter().filter(|t| t.level == Level::Expert).collect();
        let forced = checked.iter().any(|t| self.overrides.contains(&t.id));
        let mut go = false;
        let mut cancel = false;
        let resp = egui::Modal::new(Id::unique("confirm-clean")).show(ctx, |ui| {
            ui.set_width(430.0);
            theme::modal_title(ui, &format!("Clean {}?", format_bytes(bytes)));
            ui.label(muted(&p, format!("{} items will be permanently deleted. Everything is logged.", checked.len())));
            if !expert.is_empty() {
                ui.add_space(6.0);
                egui::Frame::new().fill(p.amber.gamma_multiply(0.10)).corner_radius(8).inner_margin(10).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Expert items included").color(p.amber).strong());
                    for t in &expert {
                        ui.label(format!("• {}", t.name));
                    }
                });
            }
            if forced {
                ui.label(muted(&p, "Some apps are open; files they are using will be skipped."));
            }
            ui.add_space(6.0);
            if shared.admin {
                let (changed, _) = theme::toggle_row(ui, &p, &mut shared.settings.restore_point, "Create a restore point first", true);
                if changed {
                    shared.save();
                }
            }
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let kind = if expert.is_empty() { Kind::Primary } else { Kind::Danger };
                go = pill(ui, &p, "Clean now", kind, true).clicked();
                cancel = pill(ui, &p, "Cancel", Kind::Secondary, true).clicked();
            });
        });
        if go {
            self.confirm = false;
            self.start_clean(ctx, shared, true);
        } else if cancel || resp.should_close() {
            self.confirm = false;
        }
    }

    fn restore_modal(&mut self, ctx: &egui::Context, shared: &mut Shared) {
        let Some(err) = self.restore_error.clone() else { return };
        let p = shared.pal;
        let mut anyway = false;
        let mut cancel = false;
        let resp = egui::Modal::new(Id::unique("restore-failed")).show(ctx, |ui| {
            ui.set_width(440.0);
            theme::modal_title(ui, "Couldn't create a restore point");
            ui.label("Nothing has been deleted. This usually means System Protection is turned off for your system drive.");
            ui.add_space(4.0);
            let short: String = err.lines().filter(|l| !l.trim().is_empty()).take(3).collect::<Vec<_>>().join("\n");
            ui.label(RichText::new(short).size(12.0).color(p.muted));
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                anyway = pill(ui, &p, "Clean without it", Kind::Primary, true).clicked();
                cancel = pill(ui, &p, "Cancel", Kind::Secondary, true).clicked();
            });
        });
        if anyway {
            self.restore_error = None;
            self.start_clean(ctx, shared, false);
        } else if cancel || resp.should_close() {
            self.restore_error = None;
        }
    }

    fn report_modal(&mut self, ctx: &egui::Context, shared: &mut Shared) {
        if !self.show_report {
            return;
        }
        let Some(report) = &self.report else { return };
        let p = shared.pal;
        let total = report.total();
        let mut close = false;
        let resp = egui::Modal::new(Id::unique("clean-report")).show(ctx, |ui| {
            ui.set_width(480.0);
            theme::modal_title(ui, if report.cancelled { "Cleaning stopped" } else { "Cleaning complete" });
            ui.label(RichText::new(format!("{} freed", format_bytes(total.bytes))).font(semibold(28.0)).color(p.green));
            let mut line = format!("{} files removed", format_count(total.files));
            if total.skipped > 0 {
                line.push_str(&format!(" · {} in use and skipped", total.skipped));
            }
            if total.reboot > 0 {
                line.push_str(&format!(" · {} removed after restart", total.reboot));
            }
            ui.label(muted(&p, line));
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                for r in &report.results {
                    ui.horizontal(|ui| match &r.error {
                        Some(e) => {
                            theme::dot(ui, p.amber);
                            ui.label(RichText::new(format!("{}: {e}", r.name)).size(12.5).color(p.amber));
                        }
                        None => {
                            theme::dot(ui, p.green);
                            ui.label(RichText::new(&r.name).size(12.5));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(muted(&p, format_bytes(r.tally.bytes)).size(12.5));
                            });
                        }
                    });
                }
            });
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                close = pill(ui, &p, "Done", Kind::Primary, true).clicked();
                if let Some(path) = &report.log_path {
                    if pill(ui, &p, "Open log", Kind::Secondary, true).clicked() {
                        sys::open(path);
                    }
                }
            });
        });
        if close || resp.should_close() {
            self.show_report = false;
        }
    }
}
