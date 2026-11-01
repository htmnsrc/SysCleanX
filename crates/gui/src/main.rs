// No console window in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod job;
mod pages;
mod shared;
mod theme;

use eframe::egui;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let skip_elevation = args.iter().any(|a| a == "--elevated" || a == "--no-elevate");
    // The embedded manifest (assets/app.manifest) makes Windows start SysCleanX as
    // administrator. This relaunch is only a fallback for builds without that manifest.
    if cfg!(windows) && !skip_elevation && !syscleanx_core::sys::is_admin() && syscleanx_core::sys::relaunch_elevated("--elevated") {
        return Ok(());
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("SysCleanX")
        .with_inner_size([1060.0, 720.0])
        .with_min_inner_size([880.0, 560.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("SysCleanX", options, Box::new(|cc| Ok(Box::new(app::SysCleanXApp::new(cc)))))
}
