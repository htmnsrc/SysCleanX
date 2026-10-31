use crate::pages::{
    cleaner::Cleaner, debloat::Debloat, disk::Disk, duplicates::Duplicates, repair::Repair, settings::SettingsPage,
    startup::StartupPage, uninstaller::Uninstaller,
};
use crate::shared::Shared;
use crate::theme::{self, ease_out, muted, semibold};
use eframe::egui::{self, Align2, Color32, FontId, Id, Margin, Pos2, Rect, RichText, Sense, Shape, Stroke, Vec2};
use std::time::Duration;
use syscleanx_core::settings::Settings;
use syscleanx_core::{format_bytes, sys, Engine};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Clean,
    Apps,
    Startup,
    Tools,
    Settings,
}

const NAV: &[(Page, &str, &str)] = &[
    (Page::Clean, "✨", "Clean"),
    (Page::Apps, "📦", "Apps"),
    (Page::Startup, "🚀", "Startup"),
    (Page::Tools, "🔧", "Tools"),
    (Page::Settings, "⚙", "Settings"),
];

const APPS_TABS: &[&str] = &["Programs", "Store apps"];
const TOOLS_TABS: &[&str] = &["Repair", "Disk usage", "Duplicates"];
const NAV_H: f32 = 40.0;

pub struct SysCleanXApp {
    shared: Shared,
    page: Page,
    apps_tab: usize,
    tools_tab: usize,
    page_changed_at: f64,
    cleaner: Cleaner,
    uninstaller: Uninstaller,
    startup: StartupPage,
    debloat: Debloat,
    repair: Repair,
    disk: Disk,
    duplicates: Duplicates,
    settings: SettingsPage,
}

impl SysCleanXApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = Settings::load();
        let (engine, warnings) = Engine::load();
        let mut shared = Shared::new(engine, settings, warnings);
        theme::install_fonts(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx, &shared.pal);
        shared.refresh_drive(true);
        if shared.admin {
            // Older builds could register a weekly task for the (now removed) CLI.
            std::thread::spawn(sys::remove_legacy_schedule);
        }
        #[allow(unused_mut)]
        let mut app = SysCleanXApp {
            cleaner: Cleaner::new(&shared),
            shared,
            page: Page::Clean,
            apps_tab: 0,
            tools_tab: 0,
            page_changed_at: -10.0,
            uninstaller: Uninstaller::new(),
            startup: StartupPage::new(),
            debloat: Debloat::new(),
            repair: Repair::new(),
            disk: Disk::new(),
            duplicates: Duplicates::new(),
            settings: SettingsPage::new(),
        };
        #[cfg(debug_assertions)]
        app.dev_overrides(&cc.egui_ctx);
        app
    }

    /// Debug builds only: SYSCLEANX_DEMO=1 fills pages with sample data and
    /// SYSCLEANX_PAGE=<name> opens a page (used for UI screenshots on non-Windows).
    #[cfg(debug_assertions)]
    fn dev_overrides(&mut self, ctx: &egui::Context) {
        if std::env::var("SYSCLEANX_DEMO").is_ok() {
            self.shared.admin = true;
            self.shared.drive = Some(crate::shared::Drive { label: "C:".into(), free: 412 << 30, total: 953 << 30 });
            self.shared.settings.last_clean = Some(
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) - 3 * 86_400,
            );
            self.shared.settings.total_freed = 14_200 << 20;
            self.cleaner.demo(&self.shared, ctx);
            self.uninstaller.demo();
            self.startup.demo();
            self.debloat.demo();
        }
        if let Ok(name) = std::env::var("SYSCLEANX_PAGE") {
            let (page, tab) = match name.to_lowercase().as_str() {
                "apps" | "programs" => (Page::Apps, 0),
                "store" | "debloat" => (Page::Apps, 1),
                "startup" => (Page::Startup, 0),
                "tools" | "repair" => (Page::Tools, 0),
                "disk" => (Page::Tools, 1),
                "duplicates" => (Page::Tools, 2),
                "settings" => (Page::Settings, 0),
                _ => (Page::Clean, 0),
            };
            self.page = page;
            self.apps_tab = if page == Page::Apps { tab } else { 0 };
            self.tools_tab = if page == Page::Tools { tab } else { 0 };
        }
    }

    fn go(&mut self, ctx: &egui::Context, page: Page) {
        if self.page != page {
            self.page = page;
            self.page_changed_at = ctx.input(|i| i.time);
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let p = self.shared.pal;
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.label(RichText::new("SysCleanX").font(semibold(21.0)));
        });
        ui.add_space(22.0);

        // The selection highlight is painted behind the items and slides to the active page.
        let highlight = ui.painter().add(Shape::Noop);
        let mut selected_rect = None;
        let mut clicked = None;
        for (page, icon, label) in NAV {
            let selected = self.page == *page;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), NAV_H), Sense::click());
            let hover = ctx.animate_bool_with_time(resp.id, resp.hovered() && !selected, 0.12);
            if hover > 0.0 {
                ui.painter().rect_filled(rect, 10, p.hover.gamma_multiply(hover));
            }
            let t = ctx.animate_bool_with_time(resp.id.with("sel"), selected, 0.2);
            let color = p.muted.lerp_to_gamma(p.text, t.max(hover));
            let icon_color = p.muted.lerp_to_gamma(p.accent, t);
            let nudge = 2.0 * hover;
            ui.painter().text(rect.left_center() + Vec2::new(16.0 + nudge, 0.0), Align2::LEFT_CENTER, icon, FontId::proportional(16.0), icon_color);
            ui.painter().text(rect.left_center() + Vec2::new(44.0 + nudge, 0.0), Align2::LEFT_CENTER, label, semibold(14.5), color);
            if selected {
                selected_rect = Some(rect);
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                clicked = Some(*page);
            }
            ui.add_space(2.0);
        }
        if let Some(page) = clicked {
            self.go(&ctx, page);
        }
        if let Some(r) = selected_rect {
            let y = ctx.animate_value_with_time(Id::unique("nav-highlight-y"), r.top(), 0.22);
            let rect = Rect::from_min_size(Pos2::new(r.left(), y), r.size());
            let bar = Rect::from_min_size(rect.min + Vec2::new(0.0, 11.0), Vec2::new(3.0, rect.height() - 22.0));
            ui.painter().set(
                highlight,
                Shape::Vec(vec![Shape::rect_filled(rect, 10, p.accent.gamma_multiply(0.12)), Shape::rect_filled(bar, 2, p.accent)]),
            );
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            if self.shared.admin {
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    theme::dot(ui, p.green);
                    ui.label(muted(&p, "Administrator").size(12.0));
                });
            } else {
                if theme::pill_sized(ui, &p, "Restart as admin", theme::Kind::Secondary, true, 30.0, 13.0).clicked()
                    && sys::relaunch_elevated("--elevated")
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                ui.label(muted(&p, "Limited mode: system items need admin rights.").size(11.5));
            }
            if let Some(d) = self.shared.drive.clone() {
                ui.add_space(14.0);
                let used = d.total.saturating_sub(d.free) as f32 / d.total.max(1) as f32;
                egui::Frame::new().inner_margin(Margin { left: 8, right: 8, top: 0, bottom: 0 }).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&d.label).font(semibold(12.5)));
                        ui.label(RichText::new(format!("{} free of {}", format_bytes(d.free), format_bytes(d.total))).size(12.0).color(p.muted));
                    });
                    let w = ui.available_width();
                    theme::mini_bar(ui, &p, used, if used > 0.9 { p.amber } else { p.accent }, w);
                });
            }
        });
    }

    fn tabs(&mut self, ui: &mut egui::Ui, id: &str, tabs: &[&str], current: usize) -> usize {
        let p = self.shared.pal;
        match theme::segmented(ui, &p, id, tabs, Some(current)) {
            Some(i) if i != current => {
                self.page_changed_at = ui.ctx().input(|inp| inp.time);
                i
            }
            _ => current,
        }
    }

    fn page(&mut self, ui: &mut egui::Ui) {
        let p = self.shared.pal;
        match self.page {
            Page::Clean => self.cleaner.ui(ui, &mut self.shared),
            Page::Apps => {
                let sub = if self.apps_tab == 0 {
                    "Uninstall programs completely, including the leftovers they leave behind."
                } else {
                    "Remove preinstalled Microsoft Store apps. They can be reinstalled from the Store."
                };
                let mut tab = self.apps_tab;
                theme::page_title(ui, &p, "Apps", sub, |ui| tab = self.tabs(ui, "apps-tabs", APPS_TABS, tab));
                self.apps_tab = tab;
                if self.apps_tab == 0 {
                    self.uninstaller.ui(ui, &mut self.shared);
                } else {
                    self.debloat.ui(ui, &mut self.shared);
                }
            }
            Page::Startup => {
                theme::page_title(ui, &p, "Startup", "Choose what starts with Windows. Every change is reversible.", |_| {});
                self.startup.ui(ui, &mut self.shared);
            }
            Page::Tools => {
                let sub = match self.tools_tab {
                    0 => "Fix broken shortcuts, dead startup entries and leftovers of removed programs.",
                    1 => "See what takes up space on your drives.",
                    _ => "Find identical files and keep just one copy.",
                };
                let mut tab = self.tools_tab;
                theme::page_title(ui, &p, "Tools", sub, |ui| tab = self.tabs(ui, "tools-tabs", TOOLS_TABS, tab));
                self.tools_tab = tab;
                match self.tools_tab {
                    0 => self.repair.ui(ui, &mut self.shared),
                    1 => self.disk.ui(ui, &mut self.shared),
                    _ => self.duplicates.ui(ui, &mut self.shared),
                }
            }
            Page::Settings => {
                theme::page_title(ui, &p, "Settings", "Changes are saved automatically.", |_| {});
                self.settings.ui(ui, &mut self.shared);
            }
        }
    }

    fn toast(&mut self, ctx: &egui::Context) {
        let p = self.shared.pal;
        let mut expired = false;
        if let Some(t) = &self.shared.toast {
            let ttl = if t.error { 9.0 } else { 4.5 };
            let age = t.at.elapsed().as_secs_f32();
            if age > ttl {
                expired = true;
            } else {
                let appear = ease_out(age / 0.25);
                let fade = ((ttl - age) / 0.3).clamp(0.0, 1.0);
                let offset = (1.0 - appear) * 24.0;
                egui::Area::new(Id::unique("toast"))
                    .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-22.0, -22.0 + offset))
                    .interactable(false)
                    .show(ctx, |ui| {
                        ui.set_opacity(appear * fade);
                        let accent = if t.error { p.red } else { p.green };
                        egui::Frame::new()
                            .fill(p.card)
                            .stroke(Stroke::new(1.0, p.border))
                            .corner_radius(12)
                            .inner_margin(Margin::symmetric(16, 12))
                            .shadow(egui::epaint::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(60) })
                            .show(ui, |ui| {
                                ui.set_max_width(420.0);
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(if t.error { "⚠" } else { "✔" }).color(accent).size(16.0));
                                    ui.add(egui::Label::new(&t.text).wrap());
                                });
                            });
                    });
                ctx.request_repaint_after(Duration::from_millis(16));
            }
        }
        if expired {
            self.shared.toast = None;
        }
    }
}

impl eframe::App for SysCleanXApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let p = self.shared.pal;
        self.cleaner.tick(&ctx, &mut self.shared);
        self.shared.refresh_drive(false);

        egui::Panel::left("nav")
            .resizable(false)
            .exact_size(212.0)
            .frame(egui::Frame::new().fill(p.bg).inner_margin(Margin { left: 12, right: 12, top: 22, bottom: 18 }).stroke(Stroke::new(1.0, p.border)))
            .show(ui, |ui| self.sidebar(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg).inner_margin(Margin::symmetric(30, 24)))
            .show(ui, |ui| {
                // Gentle fade + rise when switching pages or tabs.
                let since = (ctx.input(|i| i.time) - self.page_changed_at) as f32;
                let t = ease_out(since / 0.22);
                if t < 1.0 {
                    ui.multiply_opacity(t);
                    ui.add_space((1.0 - t) * 10.0);
                    ctx.request_repaint();
                }
                self.page(ui);
            });

        self.toast(&ctx);
    }
}
