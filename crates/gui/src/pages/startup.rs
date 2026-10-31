use crate::job::{take, Job};
use crate::shared::Shared;
use crate::theme::{self, badge, card, muted, pill, semibold, Kind};
use eframe::egui::{self, RichText, Sense};
use egui_extras::{Column, TableBuilder};
use std::path::PathBuf;
use syscleanx_core::startup::{self, Source, StartupItem};
use syscleanx_core::sys;
use syscleanx_core::util::{expand_env, split_command};

pub struct StartupPage {
    items: Vec<StartupItem>,
    job: Option<Job<Vec<StartupItem>>>,
    toggle_job: Option<Job<(usize, bool, Result<(), String>)>>,
    loaded: bool,
}

impl StartupPage {
    pub fn new() -> Self {
        StartupPage { items: Vec::new(), job: None, toggle_job: None, loaded: false }
    }

    #[cfg(debug_assertions)]
    pub fn demo(&mut self) {
        self.loaded = true;
        let mk = |n: &str, c: &str, s: Source, e: bool, m: bool| StartupItem {
            name: n.into(),
            command: c.into(),
            source: s,
            enabled: e,
            missing: m,
            location: String::new(),
        };
        self.items = vec![
            mk("Discord", "\"C:\\Users\\demo\\AppData\\Local\\Discord\\Update.exe\" --processStart Discord.exe", Source::RunUser, true, false),
            mk("Steam", "\"C:\\Program Files (x86)\\Steam\\steam.exe\" -silent", Source::RunUser, false, false),
            mk("SecurityHealth", "%windir%\\system32\\SecurityHealthSystray.exe", Source::RunMachine, true, false),
            mk("OldUpdater", "\"C:\\Program Files\\Gone\\updater.exe\"", Source::RunMachine32, true, true),
            mk("GoogleUpdateTaskMachineUA", "\"C:\\Program Files (x86)\\Google\\Update\\GoogleUpdate.exe\" /ua", Source::Task, true, false),
        ];
    }

    fn reload(&mut self, ctx: &egui::Context) {
        self.job = Some(Job::spawn(ctx, |p| {
            p.set_label("Reading startup programs and tasks…");
            startup::list()
        }));
    }

    fn reveal(item: &StartupItem) {
        let path = match item.source {
            Source::FolderUser | Source::FolderCommon => PathBuf::from(&item.location),
            _ => {
                let cmd = expand_env(&item.command).unwrap_or_else(|| item.command.clone());
                PathBuf::from(split_command(&cmd).0)
            }
        };
        if path.exists() {
            sys::reveal(&path);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut Shared) {
        let ctx = ui.ctx().clone();
        let p = shared.pal;
        if !self.loaded {
            self.loaded = true;
            self.reload(&ctx);
        }
        if let Some(r) = take(&mut self.job) {
            match r {
                Ok(items) => self.items = items,
                Err(e) => shared.error(e),
            }
        }
        if let Some(r) = take(&mut self.toggle_job) {
            match r {
                // `on` is the final state: the new value, or the old one if Windows refused.
                Ok((i, on, result)) => {
                    if let Some(item) = self.items.get_mut(i) {
                        item.enabled = on;
                    }
                    if let Err(e) = result {
                        shared.error(e);
                    }
                }
                Err(e) => shared.error(e),
            }
        }

        ui.horizontal(|ui| {
            let enabled = self.items.iter().filter(|i| i.enabled).count();
            let broken = self.items.iter().filter(|i| i.missing).count();
            let mut summary = format!("{} of {} start automatically", enabled, self.items.len());
            if broken > 0 {
                summary.push_str(&format!(" · {broken} point to missing programs (remove them in Tools → Repair)"));
            }
            ui.label(muted(&p, summary).size(13.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if pill(ui, &p, "Refresh", Kind::Ghost, self.job.is_none()).clicked() {
                    self.reload(&ctx);
                }
            });
        });
        if let Some(job) = &self.job {
            theme::progress_row(ui, &p, &job.progress, false);
        }
        if !shared.admin {
            ui.label(muted(&p, "Entries for all users and scheduled tasks can only be changed as administrator.").size(12.5));
        }
        ui.add_space(4.0);

        let admin = shared.admin;
        let busy = self.toggle_job.is_some();
        let mut flip: Option<(usize, bool)> = None;
        let items = &self.items;
        card(&p).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
            TableBuilder::new(ui)
                .resizable(false)
                .sense(Sense::hover())
                .auto_shrink([false; 2])
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::exact(58.0))
                .column(Column::initial(220.0).clip(true))
                .column(Column::initial(170.0).clip(true))
                .column(Column::remainder().at_least(200.0).clip(true))
                .column(Column::exact(40.0))
                .header(28.0, |mut h| {
                    for title in ["", "Name", "Where", "Command", ""] {
                        h.col(|ui| {
                            ui.label(RichText::new(title).font(semibold(12.5)).color(p.muted));
                        });
                    }
                })
                .body(|body| {
                    body.rows(38.0, items.len(), |mut row| {
                        let i = row.index();
                        let item = &items[i];
                        let machine_wide = matches!(item.source, Source::RunMachine | Source::RunMachine32 | Source::FolderCommon | Source::Task);
                        row.col(|ui| {
                            ui.add_space(6.0);
                            let mut on = item.enabled;
                            let allowed = (admin || !machine_wide) && !busy;
                            if theme::toggle(ui, &p, &mut on, allowed).changed() {
                                flip = Some((i, on));
                            }
                        });
                        row.col(|ui| {
                            let text = RichText::new(&item.name).font(semibold(13.5));
                            ui.add(egui::Label::new(if item.enabled { text } else { text.color(p.muted) }).truncate().selectable(false));
                            if item.missing {
                                badge(ui, "missing", p.amber).on_hover_text("The program it starts no longer exists. Repair can remove this entry.");
                            }
                        });
                        row.col(|ui| {
                            ui.label(muted(&p, item.source.label()).size(12.5));
                        });
                        row.col(|ui| {
                            ui.add(egui::Label::new(RichText::new(&item.command).size(12.0).monospace().color(p.muted)).truncate())
                                .on_hover_text(&item.command);
                        });
                        row.col(|ui| {
                            if theme::pill_sized(ui, &p, "📂", Kind::Ghost, true, 28.0, 13.0).on_hover_text("Show in Explorer").clicked() {
                                Self::reveal(item);
                            }
                        });
                    });
                });
        });
        if let Some((i, on)) = flip {
            // Optimistic update so the switch animates immediately; reverted if Windows refuses.
            let item = self.items[i].clone();
            self.items[i].enabled = on;
            self.toggle_job = Some(Job::spawn(&ctx, move |_| {
                let r = startup::set_enabled(&item, on);
                (i, if r.is_ok() { on } else { !on }, r)
            }));
        }
    }
}
