use crate::job::{take, Job};
use crate::shared::Shared;
use crate::theme::{self, card, muted, pill, semibold, Kind};
use eframe::egui::{self, Id, RichText};
use std::collections::HashSet;
use std::path::PathBuf;
use syscleanx_core::duplicates::{self, DupGroup};
use syscleanx_core::util::{date_string, expand_env};
use syscleanx_core::{format_bytes, sys};

const MAX_GROUPS: usize = 500;

pub struct Duplicates {
    folders: String,
    min_mb: u32,
    groups: Option<Vec<DupGroup>>,
    job: Option<Job<Vec<DupGroup>>>,
    delete_job: Option<Job<Result<(), String>>>,
    selected: HashSet<PathBuf>,
    confirm: bool,
}

impl Duplicates {
    pub fn new() -> Self {
        let folders = ["Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music"]
            .iter()
            .filter_map(|f| expand_env(&format!("%USERPROFILE%\\{f}")))
            .collect::<Vec<_>>()
            .join("\n");
        Duplicates { folders, min_mb: 1, groups: None, job: None, delete_job: None, selected: HashSet::new(), confirm: false }
    }

    fn busy(&self) -> bool {
        self.job.is_some() || self.delete_job.is_some()
    }

    fn select_all_but(&mut self, keep_newest: bool) {
        self.selected.clear();
        for g in self.groups.iter().flatten() {
            let mut files: Vec<_> = g.files.iter().collect();
            files.sort_by_key(|f| f.modified);
            if keep_newest {
                files.pop();
            } else {
                files.remove(0);
            }
            self.selected.extend(files.into_iter().map(|f| f.path.clone()));
        }
    }

    /// Selected files, never including every copy of a group.
    fn safe_selection(&self) -> (Vec<PathBuf>, u64, usize) {
        let mut out = Vec::new();
        let mut bytes = 0;
        let mut blocked = 0;
        for g in self.groups.iter().flatten() {
            let chosen: Vec<&PathBuf> = g.files.iter().map(|f| &f.path).filter(|p| self.selected.contains(*p)).collect();
            if chosen.len() == g.files.len() {
                blocked += 1;
                continue;
            }
            bytes += g.size * chosen.len() as u64;
            out.extend(chosen.into_iter().cloned());
        }
        (out, bytes, blocked)
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut Shared) {
        let ctx = ui.ctx().clone();
        let p = shared.pal;
        if let Some(r) = take(&mut self.job) {
            match r {
                Ok(g) => {
                    self.groups = Some(g);
                    self.select_all_but(true);
                }
                Err(e) => shared.error(e),
            }
        }
        if let Some(r) = take(&mut self.delete_job) {
            match r {
                Ok(Ok(())) => {
                    shared.notify("Duplicates moved to the Recycle Bin.");
                    let gone = std::mem::take(&mut self.selected);
                    if let Some(groups) = &mut self.groups {
                        for g in groups.iter_mut() {
                            g.files.retain(|f| !gone.contains(&f.path));
                        }
                        groups.retain(|g| g.files.len() > 1);
                    }
                }
                Ok(Err(e)) | Err(e) => shared.error(e),
            }
        }

        card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(muted(&p, "Folders to search, one per line").size(12.5));
                    ui.add(egui::TextEdit::multiline(&mut self.folders).desired_rows(3).desired_width(460.0).margin(egui::Margin::symmetric(10, 8)));
                });
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(muted(&p, "Skip files smaller than").size(12.5));
                    ui.add(egui::DragValue::new(&mut self.min_mb).range(0..=100_000).suffix(" MB"));
                    ui.add_space(8.0);
                    if pill(ui, &p, "Find duplicates", Kind::Primary, !self.busy()).clicked() {
                        let roots: Vec<PathBuf> = self.folders.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect();
                        let min = self.min_mb as u64 * 1024 * 1024;
                        self.job = Some(Job::spawn(&ctx, move |pr| duplicates::find(&roots, min, pr)));
                    }
                });
            });
            let progress = self.job.as_ref().map(|j| j.progress.clone()).or_else(|| self.delete_job.as_ref().map(|j| j.progress.clone()));
            if let Some(pr) = progress {
                ui.add_space(6.0);
                theme::progress_row(ui, &p, &pr, false);
            }
        });

        let Some((group_count, wasted)) = self.groups.as_ref().map(|g| (g.len(), g.iter().map(|g| g.wasted()).sum::<u64>())) else {
            return;
        };
        let (_, sel_bytes, blocked) = self.safe_selection();
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{group_count} groups · {} can be freed", format_bytes(wasted))).font(semibold(15.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let n = self.selected.len();
                if pill(ui, &p, &format!("Recycle {}", format_bytes(sel_bytes)), Kind::Danger, n > 0 && !self.busy()).clicked() {
                    self.confirm = true;
                }
                if pill(ui, &p, "Clear", Kind::Ghost, true).clicked() {
                    self.selected.clear();
                }
                if pill(ui, &p, "Keep oldest", Kind::Ghost, true).clicked() {
                    self.select_all_but(false);
                }
                if pill(ui, &p, "Keep newest", Kind::Ghost, true).clicked() {
                    self.select_all_but(true);
                }
            });
        });
        if blocked > 0 {
            ui.label(RichText::new(format!("⚠ {blocked} group(s) have every copy selected; they are skipped so one copy always stays.")).color(p.amber).size(12.5));
        }
        if group_count == 0 {
            ui.label(RichText::new("✔ No duplicates found.").color(p.green));
            return;
        }

        let groups = self.groups.as_deref().unwrap_or_default();
        let selected = &mut self.selected;
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            for g in groups.iter().take(MAX_GROUPS) {
                card(&p).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        ui.label(RichText::new(format!("{} copies · {} each", g.files.len(), format_bytes(g.size))).font(semibold(13.5)));
                    });
                    for f in &g.files {
                        let on = selected.contains(&f.path);
                        let mut toggle = false;
                        let resp = theme::hover_row(ui, &p, 30.0, |ui| {
                            toggle |= theme::check(ui, &p, on, false, true).clicked();
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(RichText::new(f.path.display().to_string()).size(12.5)).truncate().selectable(false));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if theme::pill_sized(ui, &p, "Show", Kind::Ghost, true, 24.0, 12.0).clicked() {
                                    sys::reveal(&f.path);
                                }
                                if let Some(m) = f.modified {
                                    ui.label(muted(&p, date_string(m)).size(12.0));
                                }
                            });
                        });
                        if toggle || resp.clicked() {
                            if on {
                                selected.remove(&f.path);
                            } else {
                                selected.insert(f.path.clone());
                            }
                        }
                    }
                });
                ui.add_space(6.0);
            }
            if groups.len() > MAX_GROUPS {
                ui.label(muted(&p, format!("Showing the {MAX_GROUPS} largest of {} groups.", groups.len())));
            }
        });

        if self.confirm {
            let (files, bytes, _) = self.safe_selection();
            let mut go = false;
            let mut cancel = false;
            let resp = egui::Modal::new(Id::unique("confirm-dups")).show(&ctx, |ui| {
                ui.set_width(400.0);
                theme::modal_title(ui, &format!("Move {} files to the Recycle Bin?", files.len()));
                ui.label(muted(&p, format!("Frees about {}. At least one copy of every file is kept.", format_bytes(bytes))));
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    go = pill(ui, &p, "Recycle", Kind::Danger, true).clicked();
                    cancel = pill(ui, &p, "Cancel", Kind::Secondary, true).clicked();
                });
            });
            if go {
                self.confirm = false;
                self.delete_job = Some(Job::spawn(&ctx, move |pr| {
                    pr.set_label("Moving files to the Recycle Bin…");
                    sys::recycle(&files)
                }));
            } else if cancel || resp.should_close() {
                self.confirm = false;
            }
        }
    }
}
