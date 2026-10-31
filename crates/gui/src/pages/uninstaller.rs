use crate::job::{take, Job};
use crate::shared::Shared;
use crate::theme::{self, card, muted, pill, semibold, Kind};
use eframe::egui::{self, Id, RichText, Sense};
use egui_extras::{Column, TableBuilder};
use std::collections::HashSet;
use syscleanx_core::guard::Guard;
use syscleanx_core::uninstall::{self, App, Leftover, LeftoverKind};
use syscleanx_core::{format_bytes, settings};

#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Name,
    Size,
    Date,
}

struct Outcome {
    app: App,
    error: Option<String>,
    removed: bool,
    leftovers: Vec<Leftover>,
}

pub struct Uninstaller {
    apps: Vec<App>,
    load_job: Option<Job<Vec<App>>>,
    run_job: Option<Job<Vec<Outcome>>>,
    leftover_job: Option<Job<Vec<String>>>,
    loaded: bool,
    search: String,
    show_system: bool,
    sort: Sort,
    quiet: bool,
    selected: HashSet<String>,
    confirm: bool,
    outcomes: Option<Vec<Outcome>>,
}

impl Uninstaller {
    pub fn new() -> Self {
        Uninstaller {
            apps: Vec::new(),
            load_job: None,
            run_job: None,
            leftover_job: None,
            loaded: false,
            search: String::new(),
            show_system: false,
            sort: Sort::Name,
            quiet: false,
            selected: HashSet::new(),
            confirm: false,
            outcomes: None,
        }
    }

    #[cfg(debug_assertions)]
    pub fn demo(&mut self) {
        self.loaded = true;
        let rows = [
            ("7-Zip 24.08 (x64)", "Igor Pavlov", "24.08", 5_800_000, "2024-08-11"),
            ("Discord", "Discord Inc.", "1.0.9163", 380_000_000, "2025-02-03"),
            ("Google Chrome", "Google LLC", "130.0.6723.92", 610_000_000, "2024-10-30"),
            ("Microsoft Visual Studio Code", "Microsoft Corporation", "1.95.1", 410_000_000, "2024-11-01"),
            ("NVIDIA Graphics Driver 566.03", "NVIDIA Corporation", "566.03", 1_200_000_000, "2024-10-22"),
            ("Steam", "Valve Corporation", "2.10.91.91", 820_000_000, "2023-12-24"),
            ("Spotify", "Spotify AB", "1.2.50.335", 290_000_000, "2024-11-12"),
            ("VLC media player", "VideoLAN", "3.0.21", 160_000_000, "2024-06-10"),
        ];
        self.apps = rows
            .iter()
            .map(|(n, p, v, s, d)| App {
                name: n.to_string(),
                publisher: p.to_string(),
                version: v.to_string(),
                size: *s,
                install_date: d.to_string(),
                reg_key: format!("HKLM\\demo\\{n}"),
                uninstall: "x".into(),
                ..Default::default()
            })
            .collect();
        self.selected.insert(self.apps[1].reg_key.clone());
    }

    fn reload(&mut self, ctx: &egui::Context) {
        self.load_job = Some(Job::spawn(ctx, |p| {
            p.set_label("Reading installed programs…");
            uninstall::list_apps()
        }));
    }

    fn busy(&self) -> bool {
        self.load_job.is_some() || self.run_job.is_some() || self.leftover_job.is_some()
    }

    fn visible(&self) -> Vec<usize> {
        let q = self.search.to_lowercase();
        let mut idx: Vec<usize> = self
            .apps
            .iter()
            .enumerate()
            .filter(|(_, a)| self.show_system || !a.system)
            .filter(|(_, a)| q.is_empty() || a.name.to_lowercase().contains(&q) || a.publisher.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
        let apps = &self.apps;
        match self.sort {
            Sort::Name => {}
            Sort::Size => idx.sort_by_key(|i| std::cmp::Reverse(apps[*i].size)),
            Sort::Date => idx.sort_by(|a, b| apps[*b].install_date.cmp(&apps[*a].install_date)),
        }
        idx
    }

    fn start_uninstall(&mut self, ctx: &egui::Context) {
        let chosen: Vec<App> = self.apps.iter().filter(|a| self.selected.contains(&a.reg_key)).cloned().collect();
        let quiet = self.quiet;
        self.run_job = Some(Job::spawn(ctx, move |p| {
            let guard = Guard::system();
            p.set_total(chosen.len());
            let mut out = Vec::new();
            for app in chosen {
                if p.is_cancelled() {
                    break;
                }
                p.set_label(format!("Uninstalling {} — follow its uninstaller window", app.name));
                let error = uninstall::uninstall(&app, quiet).err();
                let removed = !uninstall::key_exists(&app.reg_key);
                let leftovers = if removed {
                    p.set_label(format!("Looking for leftovers of {}…", app.name));
                    let remaining = uninstall::list_apps();
                    uninstall::find_leftovers(&app, &remaining, &guard)
                } else {
                    Vec::new()
                };
                out.push(Outcome { app, error, removed, leftovers });
                p.step();
            }
            out
        }));
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut Shared) {
        let ctx = ui.ctx().clone();
        let p = shared.pal;
        if !self.loaded {
            self.loaded = true;
            self.reload(&ctx);
        }
        if let Some(r) = take(&mut self.load_job) {
            match r {
                Ok(apps) => {
                    self.selected.retain(|k| apps.iter().any(|a| &a.reg_key == k));
                    self.apps = apps;
                }
                Err(e) => shared.error(e),
            }
        }
        if let Some(r) = take(&mut self.run_job) {
            match r {
                Ok(o) => {
                    self.selected.clear();
                    self.outcomes = Some(o);
                    self.reload(&ctx);
                }
                Err(e) => shared.error(e),
            }
        }
        if let Some(r) = take(&mut self.leftover_job) {
            match r {
                Ok(errors) if errors.is_empty() => shared.notify("Leftovers removed. Folders are in the Recycle Bin."),
                Ok(errors) => shared.error(errors.join("\n")),
                Err(e) => shared.error(e),
            }
        }

        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .hint_text("🔍  Search programs")
                    .desired_width(230.0)
                    .margin(egui::Margin::symmetric(10, 7)),
            );
            let sorts = [Sort::Name, Sort::Size, Sort::Date];
            let current = sorts.iter().position(|s| *s == self.sort);
            if let Some(i) = theme::segmented(ui, &p, "uninstall-sort", &["Name", "Size", "Newest"], current) {
                self.sort = sorts[i];
            }
            ui.add_space(4.0);
            theme::toggle_row(ui, &p, &mut self.show_system, "System items", true);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let n = self.selected.len();
                if pill(ui, &p, &format!("Uninstall ({n})"), Kind::Primary, n > 0 && !self.busy()).clicked() {
                    self.confirm = true;
                }
                if pill(ui, &p, "Refresh", Kind::Ghost, !self.busy()).clicked() {
                    self.reload(&ctx);
                }
            });
        });
        ui.add_space(6.0);

        let progress = self
            .run_job
            .as_ref()
            .map(|j| j.progress.clone())
            .or_else(|| self.load_job.as_ref().map(|j| j.progress.clone()))
            .or_else(|| self.leftover_job.as_ref().map(|j| j.progress.clone()));
        if let Some(pr) = progress {
            theme::progress_row(ui, &p, &pr, false);
        }

        let visible = self.visible();
        let total: u64 = visible.iter().map(|i| self.apps[*i].size).sum();
        let picked: Vec<&App> = self.apps.iter().filter(|a| self.selected.contains(&a.reg_key)).collect();
        let mut summary = format!("{} programs · {} reported size", visible.len(), format_bytes(total));
        if !picked.is_empty() {
            summary.push_str(&format!(" · {} selected ({})", picked.len(), format_bytes(picked.iter().map(|a| a.size).sum())));
        }
        ui.label(muted(&p, summary).size(12.5));
        ui.add_space(4.0);

        let busy = self.busy();
        let apps = &self.apps;
        let selected = &mut self.selected;
        card(&p).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(false)
                .resizable(false)
                .sense(Sense::click())
                .auto_shrink([false; 2])
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::exact(34.0))
                .column(Column::remainder().at_least(200.0).clip(true))
                .column(Column::initial(170.0).clip(true))
                .column(Column::initial(110.0).clip(true))
                .column(Column::initial(80.0))
                .column(Column::initial(95.0))
                .header(28.0, |mut h| {
                    h.col(|_| {});
                    for title in ["Name", "Publisher", "Version", "Size", "Installed"] {
                        h.col(|ui| {
                            ui.label(RichText::new(title).font(semibold(12.5)).color(p.muted));
                        });
                    }
                })
                .body(|body| {
                    body.rows(34.0, visible.len(), |mut row| {
                        let app = &apps[visible[row.index()]];
                        let on = selected.contains(&app.reg_key);
                        row.set_selected(on);
                        let mut toggle = false;
                        row.col(|ui| {
                            ui.add_space(6.0);
                            toggle |= theme::check(ui, &p, on, false, !busy).clicked();
                        });
                        row.col(|ui| {
                            ui.add(egui::Label::new(RichText::new(&app.name).font(semibold(13.5))).truncate().selectable(false));
                        });
                        row.col(|ui| {
                            ui.add(egui::Label::new(muted(&p, &app.publisher).size(13.0)).truncate().selectable(false));
                        });
                        row.col(|ui| {
                            ui.add(egui::Label::new(muted(&p, &app.version).size(13.0)).truncate().selectable(false));
                        });
                        row.col(|ui| {
                            if app.size > 0 {
                                ui.add(egui::Label::new(RichText::new(format_bytes(app.size)).size(13.0)).selectable(false));
                            }
                        });
                        row.col(|ui| {
                            ui.add(egui::Label::new(muted(&p, &app.install_date).size(13.0)).selectable(false));
                        });
                        if (toggle || row.response().clicked()) && !busy {
                            if on {
                                selected.remove(&app.reg_key);
                            } else {
                                selected.insert(app.reg_key.clone());
                            }
                        }
                    });
                });
        });

        self.confirm_modal(&ctx, shared);
        self.outcome_modal(&ctx, shared);
    }

    fn confirm_modal(&mut self, ctx: &egui::Context, shared: &Shared) {
        if !self.confirm {
            return;
        }
        let p = shared.pal;
        let names: Vec<String> = self.apps.iter().filter(|a| self.selected.contains(&a.reg_key)).map(|a| a.name.clone()).collect();
        let mut go = false;
        let mut cancel = false;
        let resp = egui::Modal::new(Id::unique("confirm-uninstall")).show(ctx, |ui| {
            ui.set_width(420.0);
            theme::modal_title(ui, &format!("Uninstall {} program(s)?", names.len()));
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                for n in &names {
                    ui.label(format!("• {n}"));
                }
            });
            ui.label(muted(&p, "Each program's own uninstaller runs in turn. Afterwards SysCleanX looks for leftovers."));
            ui.add_space(4.0);
            theme::toggle_row(ui, &p, &mut self.quiet, "Silent uninstall when supported", true);
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                go = pill(ui, &p, "Uninstall", Kind::Danger, true).clicked();
                cancel = pill(ui, &p, "Cancel", Kind::Secondary, true).clicked();
            });
        });
        if go {
            self.confirm = false;
            self.start_uninstall(ctx);
        } else if cancel || resp.should_close() {
            self.confirm = false;
        }
    }

    fn outcome_modal(&mut self, ctx: &egui::Context, shared: &mut Shared) {
        let Some(outcomes) = &mut self.outcomes else { return };
        let p = shared.pal;
        let mut close = false;
        let mut delete = false;
        let resp = egui::Modal::new(Id::unique("uninstall-outcome")).show(ctx, |ui| {
            ui.set_width(560.0);
            theme::modal_title(ui, "Uninstall results");
            egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                for o in outcomes.iter_mut() {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if o.removed {
                            ui.label(RichText::new("✔").color(p.green));
                        } else {
                            ui.label(RichText::new("✖").color(p.amber));
                        }
                        ui.label(RichText::new(&o.app.name).strong());
                    });
                    if !o.removed {
                        let why = o.error.clone().unwrap_or_else(|| "Still installed — the uninstaller was cancelled or failed.".into());
                        ui.label(muted(&p, why).size(12.5));
                    } else if o.leftovers.is_empty() {
                        ui.label(muted(&p, "No leftovers found.").size(12.5));
                    } else {
                        for l in o.leftovers.iter_mut() {
                            let resp = theme::hover_row(ui, &p, 30.0, |ui| {
                                if theme::check(ui, &p, l.checked, false, true).clicked() {
                                    l.checked = !l.checked;
                                }
                                let icon = if l.kind == LeftoverKind::Folder { "📁" } else { "🔑" };
                                ui.add(egui::Label::new(RichText::new(format!("{icon} {}", l.path)).size(12.5)).truncate().selectable(false));
                                if l.size > 0 {
                                    ui.label(muted(&p, format_bytes(l.size)).size(12.0));
                                }
                            });
                            if resp.on_hover_text(&l.reason).clicked() {
                                l.checked = !l.checked;
                            }
                        }
                    }
                }
            });
            let any = outcomes.iter().flat_map(|o| &o.leftovers).any(|l| l.checked);
            ui.add_space(6.0);
            ui.label(muted(&p, "Folders go to the Recycle Bin; registry keys are backed up before deletion.").size(12.5));
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                delete = pill(ui, &p, "Remove selected leftovers", Kind::Primary, any).clicked();
                close = pill(ui, &p, "Close", Kind::Secondary, true).clicked();
            });
        });
        if delete {
            let items: Vec<Leftover> = outcomes.iter().flat_map(|o| o.leftovers.iter()).filter(|l| l.checked).cloned().collect();
            self.leftover_job = Some(Job::spawn(ctx, move |pr| {
                pr.set_label("Removing leftovers…");
                uninstall::remove_leftovers(&items, &Guard::system(), &settings::backups_dir())
            }));
            self.outcomes = None;
        } else if close || resp.should_close() {
            self.outcomes = None;
        }
    }
}
