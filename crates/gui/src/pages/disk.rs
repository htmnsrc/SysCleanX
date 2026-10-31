use crate::job::{take, Job};
use crate::shared::Shared;
use crate::theme::{self, card, muted, pill, pill_sized, semibold, Kind};
use eframe::egui::{self, RichText, Sense, Vec2};
use std::collections::HashSet;
use std::path::PathBuf;
use syscleanx_core::diskusage::{self, DiskReport};
use syscleanx_core::{format_bytes, sys};

pub struct Disk {
    path: String,
    report: Option<DiskReport>,
    job: Option<Job<DiskReport>>,
    nav: Vec<String>,
    recycled: HashSet<PathBuf>,
    drives: Vec<PathBuf>,
}

impl Disk {
    pub fn new() -> Self {
        let drives = sys::fixed_drives();
        let path = drives.first().map(|d| d.display().to_string()).unwrap_or_default();
        Disk { path, report: None, job: None, nav: Vec::new(), recycled: HashSet::new(), drives }
    }

    fn analyze(&mut self, ctx: &egui::Context) {
        let root = PathBuf::from(self.path.trim());
        self.nav.clear();
        self.recycled.clear();
        self.job = Some(Job::spawn(ctx, move |p| diskusage::analyze(&root, 100, p)));
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut Shared) {
        let ctx = ui.ctx().clone();
        let p = shared.pal;
        if let Some(r) = take(&mut self.job) {
            match r {
                Ok(rep) => self.report = Some(rep),
                Err(e) => shared.error(e),
            }
        }
        ui.horizontal(|ui| {
            let labels: Vec<String> = self.drives.iter().map(|d| d.display().to_string()).collect();
            if !labels.is_empty() {
                let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                let current = labels.iter().position(|l| l.eq_ignore_ascii_case(self.path.trim()));
                if let Some(i) = theme::segmented(ui, &p, "drives", &refs, current) {
                    self.path = labels[i].clone();
                }
            }
            ui.add(egui::TextEdit::singleline(&mut self.path).desired_width(300.0).hint_text("Or type a folder path").margin(egui::Margin::symmetric(10, 7)));
            if pill(ui, &p, "Analyze", Kind::Primary, self.job.is_none() && !self.path.trim().is_empty()).clicked() {
                if std::path::Path::new(self.path.trim()).is_dir() {
                    self.analyze(&ctx);
                } else {
                    shared.error(format!("Folder not found: {}", self.path.trim()));
                }
            }
        });
        if let Some(job) = &self.job {
            theme::progress_row(ui, &p, &job.progress, true);
        }
        let Some(report) = &self.report else {
            if self.job.is_none() {
                ui.add_space(20.0);
                ui.label(muted(&p, "Pick a drive or folder and press Analyze."));
            }
            return;
        };

        let mut go_to: Option<Vec<String>> = None;
        let mut recycle: Option<PathBuf> = None;
        let nav = self.nav.clone();
        let current = report.root.descend(&nav).unwrap_or(&report.root);

        egui::Panel::right("largest-files").resizable(true).default_size(360.0).frame(egui::Frame::new().inner_margin(egui::Margin { left: 10, ..Default::default() })).show(ui, |ui| {
            card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Largest files").font(semibold(15.0)));
                egui::ScrollArea::vertical().id_salt("largest").auto_shrink([false; 2]).show(ui, |ui| {
                    for f in &report.largest {
                        let gone = self.recycled.contains(&f.path);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format_bytes(f.size)).font(semibold(13.0)));
                            let name = f.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                            let text = if gone { RichText::new(name).strikethrough().color(p.muted) } else { RichText::new(name) };
                            ui.add(egui::Label::new(text).truncate()).on_hover_text(format!("{}\nLast changed {} days ago", f.path.display(), f.age_days));
                        });
                        if !gone {
                            ui.horizontal(|ui| {
                                ui.add_space(4.0);
                                if pill_sized(ui, &p, "Show", Kind::Ghost, true, 24.0, 12.0).clicked() {
                                    sys::reveal(&f.path);
                                }
                                if pill_sized(ui, &p, "Recycle", Kind::Ghost, true, 24.0, 12.0).clicked() {
                                    recycle = Some(f.path.clone());
                                }
                                ui.label(muted(&p, format!("{} days old", f.age_days)).size(11.5));
                            });
                        }
                        ui.separator();
                    }
                });
            });
        });

        card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                if ui.link(report.root_path.display().to_string()).clicked() {
                    go_to = Some(Vec::new());
                }
                for (i, n) in nav.iter().enumerate() {
                    ui.label(muted(&p, "›"));
                    if ui.link(n).clicked() {
                        go_to = Some(nav[..=i].to_vec());
                    }
                }
            });
            ui.label(muted(&p, format!("{} in {} files", format_bytes(current.size), current.file_count)));
            if report.unreadable > 0 && nav.is_empty() {
                ui.label(muted(&p, format!("{} folders could not be read (permissions)", report.unreadable)).size(12.0));
            }
            ui.add_space(4.0);
            let max = current.size.max(1) as f32;
            egui::ScrollArea::vertical().id_salt("tree").auto_shrink([false; 2]).show(ui, |ui| {
                let mut rows: Vec<(String, u64, u64, bool)> =
                    current.children.iter().take(300).map(|c| (c.name.clone(), c.size, c.file_count, !c.children.is_empty() || c.file_count > 0)).collect();
                if current.own_files > 0 {
                    rows.push(("(files in this folder)".into(), current.own_files, 0, false));
                    rows.sort_by(|a, b| b.1.cmp(&a.1));
                }
                for (name, size, files, can_open) in rows {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), if can_open { Sense::click() } else { Sense::hover() });
                    let resp = if can_open { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
                    let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
                    if hover > 0.0 {
                        ui.painter().rect_filled(rect, 6, p.hover.gamma_multiply(hover));
                    }
                    let frac = size as f32 / max;
                    let bar = egui::Rect::from_min_size(rect.left_top() + Vec2::new(6.0, 24.0), Vec2::new((rect.width() - 12.0) * frac, 3.0));
                    ui.painter().rect_filled(bar, 2, p.accent.gamma_multiply(0.8));
                    let font = egui::FontId::proportional(13.5);
                    ui.painter().text(rect.left_center() + Vec2::new(8.0, -3.0), egui::Align2::LEFT_CENTER, format!("{}{name}", if can_open { "📁 " } else { "" }), font.clone(), p.text);
                    let right = if files > 0 { format!("{}   ·   {files} files", format_bytes(size)) } else { format_bytes(size) };
                    ui.painter().text(rect.right_center() + Vec2::new(-8.0, -3.0), egui::Align2::RIGHT_CENTER, right, font, p.muted);
                    if resp.clicked() {
                        let mut n = nav.clone();
                        n.push(name);
                        go_to = Some(n);
                    }
                }
            });
        });

        if let Some(n) = go_to {
            self.nav = n;
        }
        if let Some(path) = recycle {
            match sys::recycle(std::slice::from_ref(&path)) {
                Ok(()) => {
                    shared.notify(format!("Moved {} to the Recycle Bin.", path.display()));
                    self.recycled.insert(path);
                }
                Err(e) => shared.error(e),
            }
        }
    }
}
