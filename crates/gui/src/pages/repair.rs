use crate::job::{take, Job};
use crate::shared::Shared;
use crate::theme::{self, card, muted, pill, semibold, Kind};
use eframe::egui::{self, Id, RichText};
use std::collections::BTreeMap;
use syscleanx_core::broken::{self, Finding, Kind as FindingKind};
use syscleanx_core::guard::Guard;
use syscleanx_core::{format_bytes, settings, sys, uninstall};

pub struct Repair {
    findings: Option<Vec<Finding>>,
    scan_job: Option<Job<Vec<Finding>>>,
    fix_job: Option<Job<Vec<String>>>,
    hiber_job: Option<Job<Result<(), String>>>,
    /// Cached hiberfil.sys size (reading it every frame is wasteful).
    hiberfil: Option<Option<u64>>,
    /// Requested hibernation state while powercfg runs.
    hiber_pending: Option<bool>,
    confirm: bool,
}

impl Repair {
    pub fn new() -> Self {
        Repair { findings: None, scan_job: None, fix_job: None, hiber_job: None, hiberfil: None, hiber_pending: None, confirm: false }
    }

    fn busy(&self) -> bool {
        self.scan_job.is_some() || self.fix_job.is_some()
    }

    fn scan(&mut self, ctx: &egui::Context) {
        self.scan_job = Some(Job::spawn(ctx, |p| {
            p.set_label("Reading installed programs…");
            let apps = uninstall::list_apps();
            broken::scan(&apps, p)
        }));
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut Shared) {
        let ctx = ui.ctx().clone();
        let p = shared.pal;
        if let Some(r) = take(&mut self.scan_job) {
            match r {
                Ok(f) => self.findings = Some(f),
                Err(e) => shared.error(e),
            }
        }
        if let Some(r) = take(&mut self.fix_job) {
            match r {
                Ok(errors) if errors.is_empty() => shared.notify("Fixed. Backups are in the SysCleanX data folder."),
                Ok(errors) => shared.error(errors.join("\n")),
                Err(e) => shared.error(e),
            }
            self.scan(&ctx);
        }
        if let Some(r) = take(&mut self.hiber_job) {
            match r {
                Ok(Ok(())) => shared.notify("Hibernation setting changed."),
                Ok(Err(e)) | Err(e) => shared.error(e),
            }
            self.hiberfil = None;
            self.hiber_pending = None;
        }

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Broken items & leftovers").font(semibold(16.0)));
                        ui.label(muted(&p, "Every fix can be undone: registry entries are backed up, files go to the Recycle Bin.").size(12.5));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let n = self.findings.as_ref().map(|f| f.iter().filter(|x| x.checked).count()).unwrap_or(0);
                        if self.findings.is_some() && pill(ui, &p, &format!("Fix ({n})"), Kind::Primary, n > 0 && !self.busy()).clicked() {
                            self.confirm = true;
                        }
                        let (label, kind) = if self.findings.is_some() { ("Scan again", Kind::Ghost) } else { ("Scan", Kind::Primary) };
                        if pill(ui, &p, label, kind, !self.busy()).clicked() {
                            self.scan(&ctx);
                        }
                    });
                });
                let progress = self.scan_job.as_ref().map(|j| j.progress.clone()).or_else(|| self.fix_job.as_ref().map(|j| j.progress.clone()));
                if let Some(pr) = progress {
                    ui.add_space(6.0);
                    theme::progress_row(ui, &p, &pr, false);
                }
                if let Some(findings) = &mut self.findings {
                    ui.add_space(6.0);
                    if findings.is_empty() {
                        ui.label(RichText::new("✔ Nothing broken was found.").color(p.green));
                    }
                    let mut by_kind: BTreeMap<FindingKind, Vec<&mut Finding>> = BTreeMap::new();
                    for f in findings.iter_mut() {
                        by_kind.entry(f.kind).or_default().push(f);
                    }
                    for (kind, list) in by_kind {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(kind.label()).font(semibold(14.5)));
                            ui.label(muted(&p, format!("{} · {}", list.len(), kind.fix_label())).size(12.0));
                        });
                        for f in list {
                            let mut toggle = false;
                            let resp = theme::hover_row(ui, &p, 34.0, |ui| {
                                toggle |= theme::check(ui, &p, f.checked, false, true).clicked();
                                ui.add_space(4.0);
                                ui.label(RichText::new(&f.title).size(13.5));
                                if f.size > 0 {
                                    ui.label(muted(&p, format_bytes(f.size)).size(12.5));
                                }
                                ui.add(egui::Label::new(muted(&p, &f.detail).size(12.0)).truncate().selectable(false));
                            });
                            if toggle || resp.on_hover_text(&f.target).clicked() {
                                f.checked = !f.checked;
                            }
                        }
                    }
                }
            });

            ui.add_space(12.0);
            let hiberfil = *self.hiberfil.get_or_insert_with(sys::hiberfil_size);
            card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Hibernation file").font(semibold(16.0)));
                        let text = match hiberfil {
                            Some(size) => format!("hiberfil.sys uses {}. Turning hibernation off frees it but also disables Fast Startup.", format_bytes(size)),
                            None => "Hibernation is off.".to_string(),
                        };
                        ui.label(muted(&p, text).size(12.5));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can = shared.admin && self.hiber_job.is_none() && cfg!(windows);
                        let mut on = self.hiber_pending.unwrap_or(hiberfil.is_some());
                        if theme::toggle(ui, &p, &mut on, can).changed() {
                            self.hiber_pending = Some(on);
                            self.hiber_job = Some(Job::spawn(&ctx, move |_| sys::set_hibernation(on)));
                        }
                    });
                });
                if !shared.admin {
                    ui.label(muted(&p, "Requires administrator.").size(12.0));
                }
            });
        });

        if self.confirm {
            let n = self.findings.as_ref().map(|f| f.iter().filter(|x| x.checked).count()).unwrap_or(0);
            let mut go = false;
            let mut cancel = false;
            let resp = egui::Modal::new(Id::unique("confirm-repair")).show(&ctx, |ui| {
                ui.set_width(400.0);
                theme::modal_title(ui, &format!("Fix {n} item(s)?"));
                ui.label(muted(&p, "Registry entries are exported to .reg backups first; files and folders go to the Recycle Bin."));
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    go = pill(ui, &p, "Fix", Kind::Primary, true).clicked();
                    cancel = pill(ui, &p, "Cancel", Kind::Secondary, true).clicked();
                });
            });
            if go {
                self.confirm = false;
                let chosen: Vec<Finding> = self.findings.as_ref().map(|f| f.iter().filter(|x| x.checked).cloned().collect()).unwrap_or_default();
                self.fix_job = Some(Job::spawn(&ctx, move |pr| {
                    pr.set_label("Fixing…");
                    broken::fix(&chosen, &Guard::system(), &settings::backups_dir())
                }));
            } else if cancel || resp.should_close() {
                self.confirm = false;
            }
        }
    }
}
