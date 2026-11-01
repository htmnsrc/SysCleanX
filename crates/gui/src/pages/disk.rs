use crate::job::{take, Job};
use crate::shared::Shared;
use crate::theme::{self, card, muted, pill, pill_sized, semibold, Kind};
use eframe::egui::{self, Id, Layout, RichText, Vec2};
use std::path::PathBuf;
use syscleanx_core::diskusage::{self, DiskReport};
use syscleanx_core::guard::Guard;
use syscleanx_core::{format_bytes, format_count, sys};

fn age(days: u64) -> String {
    match days {
        0 => "today".into(),
        1 => "1 day old".into(),
        d => format!("{d} days old"),
    }
}

/// File name and containing folder. Splits the text on either separator so the
/// Windows sample paths of preview mode split the same way on any system.
fn name_and_folder(path: &std::path::Path) -> (String, String) {
    let s = path.display().to_string();
    match s.rfind(['\\', '/']) {
        // Keep the separator of a drive root ("C:\").
        Some(i) => {
            let folder = if i == 0 || s[..i].ends_with(':') { &s[..=i] } else { &s[..i] };
            (s[i + 1..].to_string(), folder.to_string())
        }
        None => (s, String::new()),
    }
}

/// A file or folder the user asked to delete, waiting for confirmation.
struct PendingDelete {
    path: PathBuf,
    size: u64,
    files: u64,
    is_dir: bool,
    /// Allowed, but worth a warning (e.g. inside Program Files).
    warning: Option<&'static str>,
    /// Refused by the safety rules, with the reason.
    blocked: Option<String>,
}

type DeleteResult = (PathBuf, u64, u64, Result<(), String>);

pub struct Disk {
    path: String,
    report: Option<DiskReport>,
    job: Option<Job<DiskReport>>,
    nav: Vec<String>,
    drives: Vec<PathBuf>,
    guard: Guard,
    pending: Option<PendingDelete>,
    delete_job: Option<Job<DeleteResult>>,
}

impl Disk {
    pub fn new() -> Self {
        let drives = sys::fixed_drives();
        let path = drives.first().map(|d| d.display().to_string()).unwrap_or_default();
        Disk { path, report: None, job: None, nav: Vec::new(), drives, guard: Guard::system(), pending: None, delete_job: None }
    }

    /// Debug builds only: a sample drive for UI screenshots.
    #[cfg(debug_assertions)]
    pub fn demo(&mut self) {
        use syscleanx_core::diskusage::{BigFile, DirNode};
        const GB: u64 = 1 << 30;
        self.drives = vec![PathBuf::from("C:\\"), PathBuf::from("D:\\")];
        self.path = "C:\\".into();
        let leaf =|name: &str, size: u64, files: u64| DirNode { name: name.into(), size, file_count: files, own_files: size, children: vec![] };
        let mut root = DirNode {
            name: "C:\\".into(),
            children: vec![
                leaf("Users", 212 * GB, 412_000),
                leaf("Games", 148 * GB, 96_000),
                leaf("Windows", 31 * GB, 268_000),
                leaf("Program Files", 27 * GB, 118_000),
                leaf("ProgramData", 9 * GB, 41_000),
                leaf("Program Files (x86)", 6 * GB, 33_000),
            ],
            ..Default::default()
        };
        root.own_files = 18 * GB;
        root.size = root.own_files + root.children.iter().map(|c| c.size).sum::<u64>();
        root.file_count = root.children.iter().map(|c| c.file_count).sum::<u64>() + 3;
        let big = |p: &str, gb: u64, age: u64| BigFile { path: PathBuf::from(p), size: gb * GB / 10, age_days: age };
        self.report = Some(DiskReport {
            root_path: PathBuf::from("C:\\"),
            root,
            largest: vec![
                big("C:\\hiberfil.sys", 128, 1),
                big("C:\\Users\\demo\\Videos\\Holiday 2025.mp4", 74, 312),
                big("C:\\Games\\Cyberpunk 2077\\archive\\pc\\content\\basegame_4_gamedata.archive", 61, 120),
                big("C:\\Users\\demo\\Downloads\\Windows11_23H2.iso", 58, 401),
                big("C:\\Users\\demo\\Downloads\\Setup-backup.zip", 33, 95),
                big("C:\\Users\\demo\\Documents\\VM\\ubuntu.vdi", 29, 64),
            ],
            unreadable: 4,
        });
    }

    fn analyze(&mut self, ctx: &egui::Context) {
        let root = PathBuf::from(self.path.trim());
        self.nav.clear();
        self.job = Some(Job::spawn(ctx, move |p| diskusage::analyze(&root, 100, p)));
    }

    fn ask_delete(&mut self, path: PathBuf, size: u64, files: u64, is_dir: bool) {
        let (warning, blocked) = match self.guard.check_manual(&path) {
            Ok(w) => (w, None),
            Err(reason) => (None, Some(reason)),
        };
        self.pending = Some(PendingDelete { path, size, files, is_dir, warning, blocked });
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
        if let Some(r) = take(&mut self.delete_job) {
            match r {
                Ok((path, size, files, Ok(()))) => {
                    if let Some(report) = &mut self.report {
                        report.forget(&path, size, files);
                    }
                    shared.refresh_drive(true);
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
                    shared.notify(format!("Moved {name} ({}) to the Recycle Bin.", format_bytes(size)));
                }
                Ok((_, _, _, Err(e))) | Err(e) => shared.error(e),
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
        if let Some(job) = &self.delete_job {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0).color(p.accent));
                ui.label(muted(&p, job.progress.label()).size(13.0));
            });
        }
        let Some(report) = &self.report else {
            if self.job.is_none() {
                ui.add_space(20.0);
                ui.label(muted(&p, "Pick a drive or folder and press Analyze."));
            }
            return;
        };

        let busy = self.delete_job.is_some();
        let mut go_to: Option<Vec<String>> = None;
        let mut delete: Option<(PathBuf, u64, u64, bool)> = None;
        let nav = self.nav.clone();
        let current = report.root.descend(&nav).unwrap_or(&report.root);
        let folder_path = nav.iter().fold(report.root_path.clone(), |acc, n| acc.join(n));

        egui::Panel::right("largest-files")
            .resizable(true)
            .default_size(370.0)
            .frame(egui::Frame::new().inner_margin(egui::Margin { left: 12, ..Default::default() }))
            .show(ui, |ui| {
                card(&p).inner_margin(egui::Margin::symmetric(8, 12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.label(RichText::new("Largest files").font(semibold(15.0)));
                    });
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().id_salt("largest").auto_shrink([false; 2]).show(ui, |ui| {
                        for f in &report.largest {
                            let (name, folder) = name_and_folder(&f.path);
                            theme::hover_row(ui, &p, 44.0, |ui| {
                                let hovered = ui.rect_contains_pointer(ui.max_rect());
                                let text_w = (ui.available_width() - if hovered { 130.0 } else { 70.0 }).max(60.0);
                                ui.allocate_ui_with_layout(Vec2::new(text_w, 38.0), Layout::top_down(egui::Align::Min), |ui| {
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    ui.add(egui::Label::new(RichText::new(&name).size(13.5)).truncate());
                                    ui.add(egui::Label::new(RichText::new(format!("{} · {folder}", age(f.age_days))).size(11.5).color(p.faint)).truncate());
                                });
                                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                                    if hovered && !busy {
                                        if pill_sized(ui, &p, "Delete", Kind::Ghost, true, 24.0, 12.0).clicked() {
                                            delete = Some((f.path.clone(), f.size, 1, false));
                                        }
                                        if pill_sized(ui, &p, "Show", Kind::Ghost, true, 24.0, 12.0).clicked() {
                                            sys::reveal(&f.path);
                                        }
                                    } else {
                                        ui.label(RichText::new(format_bytes(f.size)).font(semibold(13.0)));
                                    }
                                });
                            })
                            .on_hover_text(f.path.display().to_string());
                        }
                        if report.largest.is_empty() {
                            ui.label(muted(&p, "No files here."));
                        }
                    });
                });
            });

        card(&p).inner_margin(egui::Margin::symmetric(8, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.add_space(8.0);
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
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                ui.label(muted(&p, format!("{} in {} files", format_bytes(current.size), format_count(current.file_count))).size(12.5));
                if report.unreadable > 0 && nav.is_empty() {
                    ui.label(muted(&p, format!("· {} folders could not be read", report.unreadable)).size(12.5));
                }
            });
            ui.add_space(4.0);
            let max = current.size.max(1) as f32;
            egui::ScrollArea::vertical().id_salt("tree").auto_shrink([false; 2]).show(ui, |ui| {
                let mut rows: Vec<(String, u64, u64, bool)> = current
                    .children
                    .iter()
                    .take(300)
                    .map(|c| (c.name.clone(), c.size, c.file_count, true))
                    .collect();
                if current.own_files > 0 {
                    rows.push(("(files in this folder)".into(), current.own_files, 0, false));
                    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
                }
                for (name, size, files, is_dir) in rows {
                    let resp = theme::hover_row(ui, &p, 40.0, |ui| {
                        let hovered = ui.rect_contains_pointer(ui.max_rect());
                        let label = if is_dir { format!("📁  {name}") } else { name.clone() };
                        let text_w = (ui.available_width() - if hovered && is_dir { 240.0 } else { 180.0 }).max(80.0);
                        ui.allocate_ui_with_layout(Vec2::new(text_w, 30.0), Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(RichText::new(label).size(13.5).color(if is_dir { p.text } else { p.muted })).truncate());
                        });
                        ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                            if hovered && is_dir && !busy {
                                if pill_sized(ui, &p, "Delete", Kind::Ghost, true, 24.0, 12.0).clicked() {
                                    delete = Some((folder_path.join(&name), size, files, true));
                                }
                                if pill_sized(ui, &p, "Show", Kind::Ghost, true, 24.0, 12.0).clicked() {
                                    sys::reveal(&folder_path.join(&name));
                                }
                            }
                            ui.label(RichText::new(format_bytes(size)).font(semibold(13.0)));
                            if files > 0 {
                                ui.label(muted(&p, format!("{} files", format_count(files))).size(12.0));
                            }
                        });
                    });
                    // Share of this folder, drawn as a thin bar along the bottom of the row.
                    let r = resp.rect;
                    let bar = egui::Rect::from_min_size(egui::pos2(r.left() + 12.0, r.bottom() - 5.0), Vec2::new((r.width() - 24.0) * (size as f32 / max), 3.0));
                    ui.painter().rect_filled(bar, 2, p.accent.gamma_multiply(0.75));
                    if is_dir && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        let mut n = nav.clone();
                        n.push(name);
                        go_to = Some(n);
                    }
                }
                if current.children.is_empty() && current.own_files == 0 {
                    ui.label(muted(&p, "This folder is empty."));
                }
            });
        });

        if let Some(n) = go_to {
            self.nav = n;
        }
        if let Some((path, size, files, is_dir)) = delete {
            self.ask_delete(path, size, files, is_dir);
        }
        self.confirm_modal(&ctx, shared);
    }

    fn confirm_modal(&mut self, ctx: &egui::Context, shared: &mut Shared) {
        let Some(pd) = &self.pending else { return };
        let p = shared.pal;
        let name = pd.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| pd.path.display().to_string());
        let mut go = false;
        let mut close = false;
        let resp = egui::Modal::new(Id::unique("confirm-disk-delete")).show(ctx, |ui| {
            ui.set_width(460.0);
            if let Some(reason) = &pd.blocked {
                theme::modal_title(ui, &format!("{name} can't be deleted"));
                ui.label(reason.as_str());
                ui.add_space(4.0);
                ui.label(RichText::new(pd.path.display().to_string()).size(12.0).color(p.muted));
                ui.add_space(10.0);
                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    close = pill(ui, &p, "OK", Kind::Secondary, true).clicked();
                });
                return;
            }
            theme::modal_title(ui, &format!("Delete {name}?"));
            ui.label(RichText::new(pd.path.display().to_string()).size(12.0).color(p.muted));
            ui.add_space(4.0);
            let what = if pd.is_dir { format!("{} in {} files", format_bytes(pd.size), format_count(pd.files)) } else { format_bytes(pd.size) };
            ui.label(format!("{what} will be moved to the Recycle Bin, so you can restore it if needed."));
            if let Some(w) = pd.warning {
                ui.add_space(6.0);
                egui::Frame::new().fill(p.amber.gamma_multiply(0.10)).corner_radius(8).inner_margin(10).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(w).color(p.amber));
                });
            }
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                go = pill(ui, &p, "Delete", Kind::Danger, true).clicked();
                close = pill(ui, &p, "Cancel", Kind::Secondary, true).clicked();
            });
        });
        if go {
            if let Some(pd) = self.pending.take() {
                let (path, size, files) = (pd.path, pd.size, pd.files);
                self.delete_job = Some(Job::spawn(ctx, move |pr| {
                    pr.set_label(format!("Moving {} to the Recycle Bin…", path.display()));
                    let r = sys::recycle(std::slice::from_ref(&path));
                    (path, size, files, r)
                }));
            }
        } else if close || resp.should_close() {
            self.pending = None;
        }
    }
}
