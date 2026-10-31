use crate::shared::Shared;
use crate::theme::{self, card, muted, pill, semibold, Kind, Palette, ACCENTS};
use eframe::egui::{self, CursorIcon, RichText, Sense, Stroke, Vec2};
use syscleanx_core::{format_bytes, settings, sys, targets};

pub struct SettingsPage {
    exclusions: Option<String>,
}

fn section(ui: &mut egui::Ui, p: &Palette, title: &str, hint: &str, add: impl FnOnce(&mut egui::Ui)) {
    card(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).font(semibold(15.5)));
        if !hint.is_empty() {
            ui.label(muted(p, hint).size(12.5));
        }
        ui.add_space(8.0);
        add(ui);
    });
    ui.add_space(12.0);
}

/// Row of colour circles; returns the clicked index.
fn swatches(ui: &mut egui::Ui, p: &Palette, light: bool, selected: usize) -> Option<usize> {
    let mut clicked = None;
    ui.horizontal(|ui| {
        for (i, (name, dark_c, light_c)) in ACCENTS.iter().enumerate() {
            let color = if light { *light_c } else { *dark_c };
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
            let hover = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
            let sel = ui.ctx().animate_bool_with_time(resp.id.with("sel"), i == selected, 0.18);
            let c = rect.center();
            ui.painter().circle_filled(c, 10.0 + 1.5 * hover, color);
            if sel > 0.0 {
                ui.painter().circle_stroke(c, 13.5, Stroke::new(2.0, color.gamma_multiply(sel)));
            }
            if resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text(*name).clicked() {
                clicked = Some(i);
            }
            ui.add_space(4.0);
        }
        ui.label(muted(p, ACCENTS[selected.min(ACCENTS.len() - 1)].0).size(12.5));
    });
    clicked
}

impl SettingsPage {
    pub fn new() -> Self {
        SettingsPage { exclusions: None }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, shared: &mut Shared) {
        let ctx = ui.ctx().clone();
        let p = shared.pal;
        let exclusions = self.exclusions.get_or_insert_with(|| shared.settings.exclusions.join("\n"));

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            section(ui, &p, "Appearance", "", |ui| {
                let light = shared.settings.light_theme;
                ui.horizontal(|ui| {
                    if let Some(i) = theme::segmented(ui, &p, "theme", &["🌙 Dark", "☀ Light"], Some(light as usize)) {
                        if (i == 1) != light {
                            shared.set_theme(&ctx, i == 1);
                        }
                    }
                    ui.add_space(16.0);
                    if let Some(i) = swatches(ui, &p, light, shared.settings.accent) {
                        shared.set_accent(&ctx, i);
                    }
                });
            });

            section(ui, &p, "Cleaning", "", |ui| {
                let admin = shared.admin;
                let mut changed = theme::toggle_row(ui, &p, &mut shared.settings.restore_point, "Create a restore point before cleaning and debloating", admin).0;
                let (c, label) = theme::toggle_row(ui, &p, &mut shared.settings.reboot_locked, "Delete files that are in use at the next restart", admin);
                label.on_hover_text("Locked files are scheduled for deletion when Windows starts again.");
                changed |= c;
                if changed {
                    shared.save();
                }
                if !admin {
                    ui.label(muted(&p, "These options need administrator rights.").size(12.0));
                }
            });

            section(ui, &p, "Never clean", "Folders listed here are always skipped. One path per line; %ENV% variables work.", |ui| {
                ui.add(
                    egui::TextEdit::multiline(exclusions)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .margin(egui::Margin::symmetric(10, 8))
                        .hint_text("C:\\Users\\me\\AppData\\Local\\MyGame\\Cache"),
                );
                ui.add_space(4.0);
                let dirty = *exclusions != shared.settings.exclusions.join("\n");
                if pill(ui, &p, "Save", Kind::Secondary, dirty).clicked() {
                    shared.settings.exclusions = exclusions.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
                    shared.save();
                    shared.notify("Exclusions saved.");
                }
            });

            section(ui, &p, "Data", "Logs of every clean, registry backups and your own cleaning targets.", |ui| {
                for w in &shared.warnings {
                    ui.label(RichText::new(format!("⚠ {w}")).color(p.amber));
                }
                ui.horizontal(|ui| {
                    if pill(ui, &p, "Logs", Kind::Secondary, true).clicked() {
                        let _ = std::fs::create_dir_all(settings::logs_dir());
                        sys::open(&settings::logs_dir());
                    }
                    if pill(ui, &p, "Backups", Kind::Secondary, true).clicked() {
                        let _ = std::fs::create_dir_all(settings::backups_dir());
                        sys::open(&settings::backups_dir());
                    }
                    if pill(ui, &p, "Custom targets", Kind::Secondary, true).on_hover_text("Add your own folders to clean. Restart SysCleanX to load changes.").clicked() {
                        let path = settings::data_dir().join("custom-targets.toml");
                        if !path.exists() {
                            let _ = std::fs::create_dir_all(settings::data_dir());
                            let _ = std::fs::write(&path, targets::CUSTOM_TEMPLATE);
                        }
                        sys::open(&path);
                    }
                });
            });

            section(ui, &p, "About", "", |ui| {
                ui.label(format!("SysCleanX {} · by Hitman · MIT License", env!("CARGO_PKG_VERSION")));
                let stats = match shared.settings.last_clean {
                    Some(t) => format!(
                        "{} cleaning targets · last clean {} · {} freed in total",
                        shared.engine.targets.len(),
                        theme::ago(t),
                        format_bytes(shared.settings.total_freed)
                    ),
                    None => format!("{} cleaning targets", shared.engine.targets.len()),
                };
                ui.label(muted(&p, stats).size(12.5));
                ui.hyperlink_to("Join the Discord", "https://discord.gg/Ayn7uzBgbC");
            });
        });
    }
}
