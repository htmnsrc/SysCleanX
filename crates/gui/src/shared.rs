//! State shared by all pages.

use crate::theme::{self, Palette};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use syscleanx_core::settings::Settings;
use syscleanx_core::{sys, Engine};

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

/// Free/total space of the system drive, refreshed every few seconds.
#[derive(Clone)]
pub struct Drive {
    pub label: String,
    pub free: u64,
    pub total: u64,
}

pub struct Shared {
    pub engine: Arc<Engine>,
    pub settings: Settings,
    pub admin: bool,
    pub pal: Palette,
    pub toast: Option<Toast>,
    pub warnings: Vec<String>,
    pub drive: Option<Drive>,
    drive_checked: Option<Instant>,
}

impl Shared {
    pub fn new(engine: Engine, settings: Settings, warnings: Vec<String>) -> Self {
        Shared {
            admin: engine.admin,
            engine: Arc::new(engine),
            pal: theme::palette(settings.light_theme, settings.accent),
            settings,
            toast: None,
            warnings,
            drive: None,
            drive_checked: None,
        }
    }

    pub fn notify(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast { text: text.into(), error: false, at: Instant::now() });
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast { text: text.into(), error: true, at: Instant::now() });
    }

    pub fn save(&mut self) {
        if let Err(e) = self.settings.save() {
            self.error(format!("Could not save settings: {e}"));
        }
    }

    fn restyle(&mut self, ctx: &eframe::egui::Context) {
        self.pal = theme::palette(self.settings.light_theme, self.settings.accent);
        theme::apply(ctx, &self.pal);
        self.save();
    }

    pub fn set_theme(&mut self, ctx: &eframe::egui::Context, light: bool) {
        self.settings.light_theme = light;
        self.restyle(ctx);
    }

    pub fn set_accent(&mut self, ctx: &eframe::egui::Context, accent: usize) {
        self.settings.accent = accent;
        self.restyle(ctx);
    }

    /// Refresh drive info at most every 10 s (or right away when `force`).
    pub fn refresh_drive(&mut self, force: bool) {
        if !force && self.drive_checked.map(|t| t.elapsed() < Duration::from_secs(10)).unwrap_or(false) {
            return;
        }
        self.drive_checked = Some(Instant::now());
        let root = format!("{}\\", std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()));
        if let Some((free, total)) = sys::disk_space(&PathBuf::from(&root)) {
            self.drive = Some(Drive { label: root.trim_end_matches('\\').to_string(), free, total });
        }
    }

    /// Remember a finished clean for the stats shown on the Clean page.
    pub fn record_clean(&mut self, freed: u64) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        self.settings.last_clean = Some(now);
        self.settings.total_freed = self.settings.total_freed.saturating_add(freed);
        self.save();
        self.refresh_drive(true);
    }
}
