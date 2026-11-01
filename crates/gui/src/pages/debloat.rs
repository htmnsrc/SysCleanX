use crate::job::{take, Job};
use crate::shared::Shared;
use crate::theme::{self, badge, card, muted, pill, semibold, Kind};
use eframe::egui::{self, collapsing_header::CollapsingState, Id, RichText};
use std::collections::HashSet;
use syscleanx_core::bloat::{self, AppxPackage, Category};
use syscleanx_core::sys::{self, Stopped};

type RemoveResult = Result<Vec<(String, String)>, String>;

pub struct Debloat {
    pkgs: Vec<AppxPackage>,
    selected: HashSet<String>,
    load_job: Option<Job<Result<Vec<AppxPackage>, String>>>,
    remove_job: Option<Job<RemoveResult>>,
    loaded: bool,
    deprovision: bool,
    confirm: bool,
}

impl Debloat {
    pub fn new() -> Self {
        Debloat {
            pkgs: Vec::new(),
            selected: HashSet::new(),
            load_job: None,
            remove_job: None,
            loaded: false,
            deprovision: true,
            confirm: false,
        }
    }

    #[cfg(debug_assertions)]
    pub fn demo(&mut self) {
        self.loaded = true;
        for n in [
            "Microsoft.BingWeather",
            "Clipchamp.Clipchamp",
            "king.com.CandyCrushSaga",
            "Microsoft.GetHelp",
            "Microsoft.GamingApp",
            "Microsoft.YourPhone",
            "Microsoft.WindowsStore",
            "SpotifyAB.SpotifyMusic",
        ] {
            let (category, protected, friendly) = bloat::classify(n, false);
            self.pkgs.push(AppxPackage { name: n.into(), full_name: format!("{n}_1.0_x64"), version: "1.0.0.0".into(), category, protected, friendly });
        }
        self.pkgs.sort_by_key(|p| p.category);
        self.selected = self.pkgs.iter().filter(|p| p.category == Category::Bloat).map(|p| p.full_name.clone()).collect();
    }

    fn reload(&mut self, ctx: &egui::Context) {
        self.load_job = Some(Job::spawn(ctx, |p| {
            p.set_label("Reading installed Store apps…");
            bloat::list()
        }));
    }

    fn busy(&self) -> bool {
        self.load_job.is_some() || self.remove_job.is_some()
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
                Ok(Ok(pkgs)) => {
                    self.selected = pkgs.iter().filter(|p| p.category == Category::Bloat && !p.protected).map(|p| p.full_name.clone()).collect();
                    self.pkgs = pkgs;
                }
                Ok(Err(e)) | Err(e) => shared.error(e),
            }
        }
        if let Some(r) = take(&mut self.remove_job) {
            match r {
                Ok(Ok(errors)) if errors.is_empty() => shared.notify("Selected apps removed."),
                Ok(Ok(errors)) => shared.error(errors.iter().map(|(n, e)| format!("{n}: {e}")).collect::<Vec<_>>().join("\n")),
                Ok(Err(e)) | Err(e) => shared.error(e),
            }
            self.reload(&ctx);
        }

        let can_change = shared.admin && !self.busy();
        ui.horizontal(|ui| {
            theme::toggle_row(ui, &p, &mut self.deprovision, "Also for new user accounts", can_change);
            ui.add_space(12.0);
            let (changed, _) = theme::toggle_row(ui, &p, &mut shared.settings.restore_point, "Restore point first", can_change);
            if changed {
                shared.save();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let n = self.selected.len();
                if pill(ui, &p, &format!("Remove ({n})"), Kind::Primary, n > 0 && !self.busy()).clicked() {
                    self.confirm = true;
                }
                if pill(ui, &p, "Refresh", Kind::Ghost, !self.busy()).clicked() {
                    self.reload(&ctx);
                }
            });
        });
        ui.add_space(6.0);
        let progress = self.load_job.as_ref().map(|j| j.progress.clone()).or_else(|| self.remove_job.as_ref().map(|j| j.progress.clone()));
        if let Some(pr) = progress {
            theme::progress_row(ui, &p, &pr, false);
        }

        let busy = self.busy();
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            card(&p).inner_margin(egui::Margin::symmetric(8, 8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                for cat in [Category::Bloat, Category::Optional, Category::Other] {
                    let list: Vec<usize> = (0..self.pkgs.len()).filter(|i| self.pkgs[*i].category == cat).collect();
                    if list.is_empty() {
                        continue;
                    }
                    let mut state = CollapsingState::load_with_default_open(&ctx, Id::unique(("debloat", cat.label())), cat != Category::Other);
                    let openness = state.openness(&ctx);
                    let picked = list.iter().filter(|i| self.selected.contains(&self.pkgs[**i].full_name)).count();
                    let header = theme::hover_row(ui, &p, 44.0, |ui| {
                        ui.label(RichText::new(cat.label()).font(semibold(15.0)));
                        ui.label(muted(&p, format!("{picked} of {}", list.len())).size(12.5));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| theme::chevron(ui, p.muted, openness));
                    });
                    if header.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        state.toggle(ui);
                    }
                    state.show_body_unindented(ui, |ui| {
                        for i in list {
                            let pkg = &self.pkgs[i];
                            let on = self.selected.contains(&pkg.full_name);
                            let enabled = !busy && !pkg.protected;
                            let mut toggle = false;
                            let resp = theme::hover_row(ui, &p, 36.0, |ui| {
                                ui.add_space(8.0);
                                toggle |= theme::check(ui, &p, on, false, enabled).clicked();
                                ui.add_space(4.0);
                                ui.label(RichText::new(&pkg.friendly).size(14.0));
                                if pkg.protected {
                                    badge(ui, "🔒 Windows component", p.muted);
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(muted(&p, &pkg.version).size(12.0));
                                });
                            });
                            let resp = resp.on_hover_text(&pkg.name);
                            if enabled && (toggle || resp.clicked()) {
                                let key = pkg.full_name.clone();
                                if on {
                                    self.selected.remove(&key);
                                } else {
                                    self.selected.insert(key);
                                }
                            }
                        }
                        ui.add_space(4.0);
                    });
                }
                if self.pkgs.is_empty() && !busy {
                    ui.label(muted(&p, "No removable Store apps were found."));
                }
            });
            ui.add_space(12.0);
        });

        if self.confirm {
            let names: Vec<String> = self.pkgs.iter().filter(|x| self.selected.contains(&x.full_name)).map(|x| x.friendly.clone()).collect();
            let mut go = false;
            let mut cancel = false;
            let resp = egui::Modal::new(Id::unique("confirm-debloat")).show(&ctx, |ui| {
                ui.set_width(420.0);
                theme::modal_title(ui, &format!("Remove {} app(s)?", names.len()));
                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    for n in &names {
                        ui.label(format!("• {n}"));
                    }
                });
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    go = pill(ui, &p, "Remove", Kind::Danger, true).clicked();
                    cancel = pill(ui, &p, "Cancel", Kind::Secondary, true).clicked();
                });
            });
            if go {
                self.confirm = false;
                let pkgs: Vec<AppxPackage> = self.pkgs.iter().filter(|x| self.selected.contains(&x.full_name)).cloned().collect();
                let deprovision = self.deprovision && shared.admin;
                let restore = shared.settings.restore_point && shared.admin;
                self.remove_job = Some(Job::spawn(&ctx, move |pr| {
                    if restore {
                        let start = std::time::Instant::now();
                        let result = sys::create_restore_point("SysCleanX debloat", std::time::Duration::from_secs(300), || {
                            let s = start.elapsed().as_secs();
                            pr.set_label(format!("Creating a restore point… {}:{:02}  ·  Windows can take a few minutes", s / 60, s % 60));
                            !pr.is_cancelled()
                        });
                        match result {
                            Ok(()) => {}
                            Err(Stopped::Cancelled) => return Err("Cancelled. Nothing was removed.".into()),
                            Err(Stopped::TimedOut) => {
                                return Err("Windows didn't finish creating the restore point within 5 minutes, so nothing was removed. Turn off \"Restore point first\" to continue without one.".into())
                            }
                            Err(Stopped::Failed(e)) => {
                                return Err(format!(
                                    "A restore point could not be created, so nothing was removed. Turn off \"Restore point first\" to continue.\n{e}"
                                ))
                            }
                        }
                    }
                    // One app at a time so Cancel takes effect between apps.
                    let mut errors = Vec::new();
                    for (i, pkg) in pkgs.iter().enumerate() {
                        if pr.is_cancelled() {
                            errors.push(("Stopped".to_string(), format!("{} app(s) were not removed", pkgs.len() - i)));
                            break;
                        }
                        pr.set_label(format!("Removing {}…", pkg.friendly));
                        errors.extend(bloat::remove(std::slice::from_ref(pkg), deprovision));
                    }
                    Ok(errors)
                }));
            } else if cancel || resp.should_close() {
                self.confirm = false;
            }
        }
    }
}
