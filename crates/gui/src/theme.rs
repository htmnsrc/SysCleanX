//! Palette, egui style and the small animated widgets every page uses.

use eframe::egui::{
    self, Align, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Id, Layout, Margin, Pos2, Rect, Response,
    RichText, Sense, Stroke, StrokeKind, TextStyle, Ui, UiBuilder, Vec2,
};
use syscleanx_core::targets::Level;
use syscleanx_core::Progress;

#[derive(Clone, Copy)]
pub struct Palette {
    pub light: bool,
    pub bg: Color32,
    pub card: Color32,
    pub hover: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    /// Text drawn on top of the accent colour.
    pub on_accent: Color32,
    pub green: Color32,
    pub blue: Color32,
    pub amber: Color32,
    pub red: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// (name, dark-mode colour, light-mode colour)
pub const ACCENTS: &[(&str, Color32, Color32)] = &[
    ("Violet", rgb(0x8b, 0x7c, 0xf6), rgb(0x6d, 0x5a, 0xe6)),
    ("Teal", rgb(0x2d, 0xd4, 0xbf), rgb(0x0f, 0x9f, 0x8f)),
    ("Blue", rgb(0x5b, 0x8c, 0xff), rgb(0x2f, 0x6f, 0xed)),
    ("Rose", rgb(0xf4, 0x72, 0xb6), rgb(0xdb, 0x27, 0x77)),
    ("Amber", rgb(0xf5, 0xb5, 0x47), rgb(0xc2, 0x7c, 0x0e)),
];

fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| (v as f32 / 255.0).powf(2.2);
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

pub fn palette(light: bool, accent: usize) -> Palette {
    let (_, dark_accent, light_accent) = ACCENTS[accent.min(ACCENTS.len() - 1)];
    let accent = if light { light_accent } else { dark_accent };
    let on_accent = if luminance(accent) > 0.4 { rgb(0x0b, 0x0d, 0x10) } else { Color32::WHITE };
    if light {
        Palette {
            light,
            bg: rgb(0xf6, 0xf7, 0xf9),
            card: rgb(0xff, 0xff, 0xff),
            hover: rgb(0xf0, 0xf2, 0xf5),
            border: rgb(0xe4, 0xe7, 0xec),
            border_strong: rgb(0xc5, 0xcc, 0xd6),
            text: rgb(0x15, 0x18, 0x1d),
            muted: rgb(0x5f, 0x67, 0x75),
            faint: rgb(0x99, 0xa0, 0xac),
            accent,
            on_accent,
            green: rgb(0x16, 0xa3, 0x4a),
            blue: rgb(0x25, 0x63, 0xeb),
            amber: rgb(0xb4, 0x53, 0x09),
            red: rgb(0xdc, 0x26, 0x26),
        }
    } else {
        Palette {
            light,
            bg: rgb(0x0c, 0x0e, 0x12),
            card: rgb(0x14, 0x17, 0x1c),
            hover: rgb(0x1c, 0x20, 0x27),
            border: rgb(0x22, 0x27, 0x2f),
            border_strong: rgb(0x39, 0x40, 0x4b),
            text: rgb(0xe8, 0xea, 0xed),
            muted: rgb(0x8c, 0x94, 0xa2),
            faint: rgb(0x5c, 0x63, 0x70),
            accent,
            on_accent,
            green: rgb(0x3d, 0xd6, 0x8c),
            blue: rgb(0x6c, 0xa8, 0xff),
            amber: rgb(0xf5, 0xb5, 0x47),
            red: rgb(0xf8, 0x71, 0x71),
        }
    }
}

/// Apply our style. The theme is pinned so a Windows light/dark switch while the app is
/// open cannot swap in egui's default look.
pub fn apply(ctx: &egui::Context, p: &Palette) {
    let theme = if p.light { egui::Theme::Light } else { egui::Theme::Dark };
    ctx.set_theme(if p.light { egui::ThemePreference::Light } else { egui::ThemePreference::Dark });
    let mut v = if p.light { egui::Visuals::light() } else { egui::Visuals::dark() };
    v.panel_fill = p.bg;
    v.window_fill = p.card;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(16);
    v.menu_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = if p.light { rgb(0xf8, 0xf9, 0xfb) } else { rgb(0x10, 0x12, 0x16) };
    v.faint_bg_color = p.hover;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent.gamma_multiply(0.28);
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.warn_fg_color = p.amber;
    v.error_fg_color = p.red;
    let r = CornerRadius::same(8);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = r;
        w.expansion = 0.0;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.inactive.weak_bg_fill = p.hover;
    v.widgets.inactive.bg_fill = p.hover;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.hovered.weak_bg_fill = p.border;
    v.widgets.hovered.bg_fill = p.hover;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.border_strong);
    v.widgets.active.bg_fill = p.accent;
    v.widgets.active.weak_bg_fill = p.accent;
    ctx.set_visuals_of(theme, v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 6.0);
        s.spacing.button_padding = Vec2::new(12.0, 6.0);
        s.spacing.interact_size.y = 26.0;
        s.interaction.tooltip_delay = 0.35;
        // Text is never selectable, so clicks on a row's label reach the row itself.
        s.interaction.selectable_labels = false;
        s.text_styles = [
            (TextStyle::Heading, FontId::new(22.0, FontFamily::Name("semibold".into()))),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
        ]
        .into();
    });
}

/// Use Segoe UI on Windows (nothing extra embedded), keep egui's fonts as fallback.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let proportional = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    fonts.families.insert(FontFamily::Name("semibold".into()), proportional);
    if let Ok(root) = std::env::var("SystemRoot") {
        let dir = std::path::Path::new(&root).join("Fonts");
        let mut add = |key: &str, file: &str, family: FontFamily, front: bool| {
            if let Ok(bytes) = std::fs::read(dir.join(file)) {
                fonts.font_data.insert(key.into(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
                let list = fonts.families.entry(family).or_default();
                if front {
                    list.insert(0, key.into());
                } else {
                    list.push(key.into());
                }
            }
        };
        add("segoe", "segoeui.ttf", FontFamily::Proportional, true);
        add("segoe-sb", "seguisb.ttf", FontFamily::Name("semibold".into()), true);
        add("segoe-sym", "seguisym.ttf", FontFamily::Proportional, false);
        add("segoe-sym2", "seguisym.ttf", FontFamily::Name("semibold".into()), false);
        // Monochrome outlines of Segoe UI Emoji cover the icons used in the UI.
        add("segoe-emoji", "seguiemj.ttf", FontFamily::Proportional, false);
        add("segoe-emoji2", "seguiemj.ttf", FontFamily::Name("semibold".into()), false);
        add("consolas", "consola.ttf", FontFamily::Monospace, true);
    }
    ctx.set_fonts(fonts);
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}

pub fn muted(p: &Palette, text: impl Into<String>) -> RichText {
    RichText::new(text.into()).color(p.muted)
}

pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Flat surface. Dark mode relies on the fill alone; light mode adds a hairline.
pub fn card(p: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, if p.light { p.border } else { Color32::TRANSPARENT }))
        .corner_radius(14)
        .inner_margin(Margin::symmetric(18, 16))
}

/// Page title + subtitle, with optional controls (tabs, actions) on the right.
pub fn page_title(ui: &mut Ui, p: &Palette, title: &str, subtitle: &str, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).font(semibold(26.0)));
            if !subtitle.is_empty() {
                ui.label(RichText::new(subtitle).size(13.0).color(p.muted));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
    ui.add_space(14.0);
}

pub fn badge(ui: &mut Ui, text: &str, color: Color32) -> Response {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.14))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.5).color(color));
        })
        .response
}

pub fn level_color(p: &Palette, level: Level) -> Color32 {
    match level {
        Level::Safe => p.green,
        Level::Recommended => p.blue,
        Level::Expert => p.amber,
    }
}

/// Small coloured dot (safety level, status).
pub fn dot(ui: &mut Ui, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
    resp
}

/// "● Safe  ● Recommended  ● Expert" legend.
pub fn level_legend(ui: &mut Ui, p: &Palette) {
    for level in [Level::Expert, Level::Recommended, Level::Safe] {
        ui.label(RichText::new(level.label()).size(12.0).color(p.muted));
        dot(ui, level_color(p, level));
        ui.add_space(6.0);
    }
}

/// Thin proportional bar (e.g. a category's share of all junk).
pub fn mini_bar(ui: &mut Ui, p: &Palette, fraction: f32, color: Color32, width: f32) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 4.0), Sense::hover());
    let f = ui.ctx().animate_value_with_time(resp.id, fraction.clamp(0.0, 1.0), 0.4);
    ui.painter().rect_filled(rect, 2, p.border);
    if f > 0.0 {
        ui.painter().rect_filled(Rect::from_min_size(rect.min, Vec2::new((rect.width() * f).max(3.0), rect.height())), 2, color);
    }
}

/// Stacked drive bar: used space, the part of it that is junk, and free space.
pub fn drive_bar(ui: &mut Ui, p: &Palette, total: u64, free: u64, junk: u64) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 8.0), Sense::hover());
    let used = total.saturating_sub(free);
    let junk = junk.min(used);
    let frac = |v: u64| if total > 0 { v as f32 / total as f32 } else { 0.0 };
    let used_f = ui.ctx().animate_value_with_time(resp.id.with("used"), frac(used), 0.5);
    let junk_f = ui.ctx().animate_value_with_time(resp.id.with("junk"), frac(junk), 0.5);
    let painter = ui.painter();
    painter.rect_filled(rect, 4, p.border);
    let used_rect = Rect::from_min_size(rect.min, Vec2::new(rect.width() * used_f, rect.height()));
    painter.rect_filled(used_rect, 4, p.muted.gamma_multiply(if p.light { 0.55 } else { 0.45 }));
    if junk_f > 0.0 {
        let w = (rect.width() * junk_f).max(4.0);
        let junk_rect = Rect::from_min_size(Pos2::new(used_rect.right() - w, rect.top()), Vec2::new(w, rect.height()));
        painter.rect_filled(junk_rect, 4, p.accent);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Danger,
    Secondary,
    Ghost,
}

/// Rounded button with hover fade and press feedback.
pub fn pill_sized(ui: &mut Ui, p: &Palette, text: &str, kind: Kind, enabled: bool, height: f32, size: f32) -> Response {
    let (bg, bg_hover, fg) = match kind {
        Kind::Primary => (p.accent, p.accent.lerp_to_gamma(Color32::WHITE, 0.14), p.on_accent),
        Kind::Danger => (p.red, p.red.lerp_to_gamma(Color32::WHITE, 0.14), Color32::WHITE),
        Kind::Secondary => (p.hover, p.border, p.text),
        Kind::Ghost => (Color32::TRANSPARENT, p.hover, p.accent),
    };
    let fg = if enabled { fg } else { fg.gamma_multiply(0.55) };
    let galley = ui.painter().layout_no_wrap(text.to_owned(), semibold(size), fg);
    let pad = if kind == Kind::Ghost { 12.0 } else { 18.0 };
    let desired = Vec2::new(galley.size().x + pad * 2.0, height);
    let (rect, resp) = ui.allocate_exact_size(desired, if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let hover = ui.ctx().animate_bool_with_time(resp.id, enabled && resp.hovered(), 0.12);
        let mut fill = bg.lerp_to_gamma(bg_hover, hover);
        if !enabled {
            fill = fill.gamma_multiply(0.4);
        }
        let r = if enabled && resp.is_pointer_button_down_on() { rect.shrink(1.0) } else { rect };
        ui.painter().rect_filled(r, 10, fill);
        ui.painter().galley(r.center() - galley.size() / 2.0, galley, fg);
    }
    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        resp
    }
}

pub fn pill(ui: &mut Ui, p: &Palette, text: &str, kind: Kind, enabled: bool) -> Response {
    pill_sized(ui, p, text, kind, enabled, 34.0, 14.0)
}

/// Animated checkbox. `mixed` draws the "some selected" dash.
pub fn check(ui: &mut Ui, p: &Palette, on: bool, mixed: bool, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(18.0), if enabled { Sense::click() } else { Sense::hover() });
    if ui.is_rect_visible(rect) {
        let t = ui.ctx().animate_bool_with_time(resp.id, on || mixed, 0.14);
        let a = if enabled { 1.0 } else { 0.35 };
        let border = if enabled && resp.hovered() { p.accent } else { p.border_strong };
        let painter = ui.painter();
        painter.rect_filled(rect, 5, p.accent.gamma_multiply(t * a));
        painter.rect_stroke(rect, 5, Stroke::new(1.5, border.lerp_to_gamma(p.accent, t).gamma_multiply(a)), StrokeKind::Inside);
        let stroke = Stroke::new(2.0, p.on_accent.gamma_multiply(t * a));
        if mixed {
            painter.line_segment([rect.left_center() + Vec2::new(4.5, 0.0), rect.right_center() - Vec2::new(4.5, 0.0)], stroke);
        } else if t > 0.01 {
            let pt = |x: f32, y: f32| rect.min + Vec2::new(x * rect.width(), y * rect.height());
            painter.line_segment([pt(0.26, 0.52), pt(0.43, 0.69)], stroke);
            painter.line_segment([pt(0.43, 0.69), pt(0.76, 0.33)], stroke);
        }
    }
    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Animated on/off switch. Returns a response that is `changed()` when flipped.
pub fn toggle(ui: &mut Ui, p: &Palette, on: &mut bool, enabled: bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(36.0, 20.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let t = ease_out(ui.ctx().animate_bool_with_time(resp.id, *on, 0.16));
        let a = if enabled { 1.0 } else { 0.4 };
        let track = p.border_strong.lerp_to_gamma(p.accent, t).gamma_multiply(a);
        ui.painter().rect_filled(rect, 10, track);
        let x = egui::lerp(rect.left() + 10.0..=rect.right() - 10.0, t);
        let knob = if t > 0.5 { p.on_accent } else { Color32::WHITE };
        ui.painter().circle_filled(Pos2::new(x, rect.center().y), 7.0, knob.gamma_multiply(a));
    }
    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Switch with a clickable label. Returns (changed, label response).
pub fn toggle_row(ui: &mut Ui, p: &Palette, on: &mut bool, label: &str, enabled: bool) -> (bool, Response) {
    ui.horizontal(|ui| {
        let mut changed = toggle(ui, p, on, enabled).changed();
        let color = if enabled { p.text } else { p.muted };
        let l = ui.add(egui::Label::new(RichText::new(label).color(color)).sense(if enabled { Sense::click() } else { Sense::hover() }));
        if enabled && l.clicked() {
            *on = !*on;
            changed = true;
        }
        (changed, l)
    })
    .inner
}

/// Full-width row that highlights on hover. Children are laid out left to right on top of
/// the row, so their own clicks still work; the returned response is the row background.
pub fn hover_row(ui: &mut Ui, p: &Palette, height: f32, add: impl FnOnce(&mut Ui)) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let t = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.contains_pointer(), 0.12);
    if t > 0.0 && ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, 10, p.hover.gamma_multiply(t));
    }
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(Vec2::new(12.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
    add(&mut child);
    resp
}

/// A chevron that rotates from › (closed) to ⌄ (open).
pub fn chevron(ui: &mut Ui, color: Color32, openness: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
    let c = rect.center();
    let rot = egui::emath::Rot2::from_angle(openness * std::f32::consts::FRAC_PI_2);
    let pts = [Vec2::new(-2.5, -5.0), Vec2::new(2.5, 0.0), Vec2::new(-2.5, 5.0)].map(|v| c + rot * v);
    let stroke = Stroke::new(1.7, color);
    ui.painter().line_segment([pts[0], pts[1]], stroke);
    ui.painter().line_segment([pts[1], pts[2]], stroke);
}

/// Segmented control with a sliding highlight. Returns the clicked index.
pub fn segmented(ui: &mut Ui, p: &Palette, id: &str, options: &[&str], selected: Option<usize>) -> Option<usize> {
    let font = semibold(13.0);
    let max_w = options
        .iter()
        .map(|o| ui.painter().layout_no_wrap(o.to_string(), font.clone(), p.text).size().x)
        .fold(0.0, f32::max);
    let seg_w = max_w + 26.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(seg_w * options.len() as f32 + 6.0, 32.0), Sense::hover());
    ui.painter().rect_filled(rect, 10, if p.light { p.hover } else { p.card });
    if let Some(sel) = selected {
        let target = rect.left() + 3.0 + seg_w * sel as f32;
        let x = ui.ctx().animate_value_with_time(Id::unique(("segmented", id)), target, 0.18);
        let knob = Rect::from_min_size(Pos2::new(x, rect.top() + 3.0), Vec2::new(seg_w, rect.height() - 6.0));
        ui.painter().rect_filled(knob, 8, if p.light { p.card } else { p.border });
    }
    let mut clicked = None;
    for (i, opt) in options.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(rect.left() + 3.0 + seg_w * i as f32, rect.top()), Vec2::new(seg_w, rect.height()));
        let resp = ui.interact(r, Id::unique(("segmented", id, i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let color = if Some(i) == selected || resp.hovered() { p.text } else { p.muted };
        let g = ui.painter().layout_no_wrap(opt.to_string(), font.clone(), color);
        ui.painter().galley(r.center() - g.size() / 2.0, g, color);
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// Thin progress bar: animated fill, or a moving segment when the total is unknown.
pub fn slim_progress(ui: &mut Ui, p: &Palette, fraction: Option<f32>) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 4.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2, p.border);
    match fraction {
        Some(f) => {
            let f = ui.ctx().animate_value_with_time(resp.id, f.clamp(0.0, 1.0), 0.35);
            painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(rect.width() * f, rect.height())), 2, p.accent);
        }
        None => {
            let w = rect.width() * 0.28;
            let x = ((ui.input(|i| i.time) as f32 * 0.8).fract() * (rect.width() + w)) - w;
            let seg = Rect::from_min_max(
                Pos2::new((rect.left() + x).max(rect.left()), rect.top()),
                Pos2::new((rect.left() + x + w).min(rect.right()), rect.bottom()),
            );
            painter.rect_filled(seg, 2, p.accent);
        }
    }
    ui.ctx().request_repaint();
}

/// Bytes that count up/down smoothly when the value changes.
pub fn animated_bytes(ui: &Ui, id: Id, bytes: u64) -> String {
    let target = (bytes as f64 / 1_048_576.0) as f32;
    let v = ui.ctx().animate_value_with_time(id, target, 0.6) as f64;
    syscleanx_core::format_bytes((v * 1_048_576.0).round().max(0.0) as u64)
}

/// Compact "working" indicator: spinner, label, cancel link and a slim progress bar.
/// Returns true if cancel was clicked.
pub fn progress_row(ui: &mut Ui, p: &Palette, progress: &Progress, show_bytes: bool) -> bool {
    progress_row_ext(ui, p, progress, show_bytes, None).0
}

/// Like `progress_row`, with an optional extra action next to Cancel.
/// Returns (cancel clicked, extra clicked).
pub fn progress_row_ext(ui: &mut Ui, p: &Palette, progress: &Progress, show_bytes: bool, extra: Option<&str>) -> (bool, bool) {
    let mut cancel = false;
    let mut extra_clicked = false;
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(14.0).color(p.accent));
        let mut label = progress.label();
        if show_bytes && progress.files() > 0 {
            label = format!("{label}  ·  {} files, {}", syscleanx_core::format_count(progress.files()), syscleanx_core::format_bytes(progress.bytes()));
        }
        ui.add(egui::Label::new(RichText::new(label).color(p.muted).size(13.0)).truncate());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if progress.is_cancelled() {
                ui.label(RichText::new("Stopping…").color(p.muted).size(13.0));
            } else {
                if pill_sized(ui, p, "Cancel", Kind::Ghost, true, 26.0, 13.0).clicked() {
                    progress.cancel();
                    cancel = true;
                }
                if let Some(label) = extra {
                    extra_clicked = pill_sized(ui, p, label, Kind::Ghost, true, 26.0, 13.0).clicked();
                }
            }
        });
    });
    slim_progress(ui, p, progress.fraction());
    ui.add_space(6.0);
    (cancel, extra_clicked)
}

pub fn modal_title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(semibold(19.0)));
    ui.add_space(4.0);
}

/// "3 days ago" style relative time for a unix timestamp.
pub fn ago(unix: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(unix);
    let s = now.saturating_sub(unix);
    match s {
        0..=89 => "just now".into(),
        90..=3_599 => format!("{} min ago", s / 60),
        3_600..=86_399 => format!("{} h ago", s / 3_600),
        86_400..=172_799 => "yesterday".into(),
        _ => format!("{} days ago", s / 86_400),
    }
}
