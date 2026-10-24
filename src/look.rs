//! The look of both programs: the dark RadioMoon palette, cards, value tiles, status pills and bandwidth graphs.
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Response, RichText, Sense, Stroke, Ui};
use std::collections::VecDeque;

pub const PANEL: Color32 = Color32::from_rgb(0x11, 0x15, 0x1b);
pub const CARD: Color32 = Color32::from_rgb(0x17, 0x1c, 0x24);
pub const BORDER: Color32 = Color32::from_rgb(0x26, 0x2d, 0x38);
pub const INSET: Color32 = Color32::from_rgb(0x0c, 0x0f, 0x14);
pub const CONTROL: Color32 = Color32::from_rgb(0x20, 0x27, 0x31);
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(0x29, 0x32, 0x3e);
pub const CONTROL_ACTIVE: Color32 = Color32::from_rgb(0x31, 0x3c, 0x4a);
pub const TEXT: Color32 = Color32::from_rgb(0xd6, 0xdc, 0xe4);
pub const DIM: Color32 = Color32::from_rgb(0x7a, 0x85, 0x93);
pub const ACCENT: Color32 = Color32::from_rgb(0x38, 0xb6, 0xff);
pub const GOOD: Color32 = Color32::from_rgb(0x34, 0xd3, 0x86);
pub const WARN: Color32 = Color32::from_rgb(0xf5, 0xb0, 0x3a);
pub const BAD: Color32 = Color32::from_rgb(0xff, 0x52, 0x5a);

fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.panel_fill = PANEL;
    v.window_fill = CARD;
    v.extreme_bg_color = INSET;
    v.faint_bg_color = Color32::from_rgb(0x1b, 0x21, 0x2a);
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARN;
    v.error_fg_color = BAD;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.window_rounding = 6.0.into();
    v.menu_rounding = 6.0.into();
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    let r: egui::Rounding = 4.0.into();
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = CARD;
    w.noninteractive.weak_bg_fill = CARD;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.noninteractive.rounding = r;
    w.inactive.bg_fill = CONTROL;
    w.inactive.weak_bg_fill = CONTROL;
    w.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.inactive.rounding = r;
    w.hovered.bg_fill = CONTROL_HOVER;
    w.hovered.weak_bg_fill = CONTROL_HOVER;
    w.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.6));
    w.hovered.fg_stroke = Stroke::new(1.5_f32, TEXT);
    w.hovered.rounding = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = CONTROL_ACTIVE;
    w.active.weak_bg_fill = CONTROL_ACTIVE;
    w.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    w.active.fg_stroke = Stroke::new(1.5_f32, TEXT);
    w.active.rounding = r;
    w.active.expansion = 0.0;
    w.open = w.active;
    v
}

/// Fonts, sizes, spacing and colours (call once at start).
pub fn apply_style(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions, FontFamily, TextStyle};
    ctx.set_theme(egui::Theme::Dark);
    let mut fd = FontDefinitions::default();
    fd.font_data.insert("dejavu".into(), FontData::from_static(include_bytes!("../assets/fonts/DejaVuSans.ttf")));
    fd.families.entry(FontFamily::Proportional).or_default().push("dejavu".into());
    fd.families.entry(FontFamily::Monospace).or_default().push("dejavu".into());
    ctx.set_fonts(fd);
    let mut style = (*ctx.style()).clone();
    style.text_styles = std::collections::BTreeMap::from([
        (TextStyle::Heading, FontId::new(19.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
        (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
    ]);
    style.spacing.item_spacing = vec2(8.0, 6.0);
    style.spacing.button_padding = vec2(10.0, 4.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.slider_width = 170.0;
    style.visuals = visuals();
    ctx.set_style(style);
}

/// A rounded panel with an optional blue title.
pub fn card<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let r = egui::Frame::none()
        .fill(CARD)
        .rounding(8.0)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .inner_margin(14.0)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).strong().size(13.5).color(ACCENT));
                ui.add_space(4.0);
            }
            add(ui)
        })
        .inner;
    ui.add_space(8.0);
    r
}

/// A darker box inside a card (one IP output, one value tile).
pub fn inset<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::none().fill(INSET).rounding(6.0).stroke(Stroke::new(1.0_f32, BORDER)).inner_margin(10.0).show(ui, add).inner
}

/// A value tile: small caption, large number, unit.
pub fn tile(ui: &mut Ui, caption: &str, value: &str, unit: &str, color: Color32, width: f32) {
    egui::Frame::none().fill(INSET).rounding(6.0).stroke(Stroke::new(1.0_f32, BORDER)).inner_margin(egui::Margin::symmetric(12.0, 8.0)).show(ui, |ui| {
        ui.set_width(width);
        ui.vertical(|ui| {
            ui.label(RichText::new(caption).small().color(DIM));
            ui.horizontal(|ui| {
                ui.label(RichText::new(value).size(22.0).strong().color(color));
                if !unit.is_empty() {
                    ui.label(RichText::new(unit).color(DIM));
                }
            });
        });
    });
}

/// A coloured rounded label such as "ON AIR".
pub fn pill(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(color.gamma_multiply(0.18))
        .stroke(Stroke::new(1.0_f32, color))
        .rounding(12.0)
        .inner_margin(egui::Margin::symmetric(10.0, 3.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).strong().size(12.5).color(color));
        });
}

/// A small round state light.
pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 5.0, color);
}

pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE).strong().size(15.0)).fill(Color32::from_rgb(0x1f, 0x8f, 0x55)).rounding(6.0).min_size(vec2(110.0, 32.0)))
}

pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE).strong().size(15.0)).fill(Color32::from_rgb(160, 45, 50)).rounding(6.0).min_size(vec2(110.0, 32.0)))
}

/// Deviation bar: 0..125%, green below 100%, red above.
pub fn deviation_bar(ui: &mut Ui, peak: f32) {
    let w = ui.available_width().max(120.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, 22.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, INSET);
    let x_of = |v: f32| rect.left() + (v.clamp(0.0, 1.25) / 1.25) * rect.width();
    let fill = Rect::from_min_max(rect.min, pos2(x_of(peak), rect.max.y));
    let c = if peak > 1.0 { BAD } else if peak > 0.95 { WARN } else { GOOD };
    p.rect_filled(fill, 4.0, c.gamma_multiply(0.85));
    let x100 = x_of(1.0);
    p.line_segment([pos2(x100, rect.top()), pos2(x100, rect.bottom())], Stroke::new(1.5_f32, BAD));
    p.text(rect.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, format!("{:.1} kHz  ({:.0}%)", peak * 75.0, peak * 100.0), FontId::proportional(12.5), Color32::WHITE);
    p.text(pos2(x100 - 4.0, rect.center().y), Align2::RIGHT_CENTER, "75 kHz", FontId::proportional(10.5), Color32::from_rgb(255, 200, 200));
}

/// Bandwidth history graph (Mbit/s) with its scale.
pub fn sparkline(ui: &mut Ui, hist: &VecDeque<f32>, height: f32, color: Color32) {
    let w = ui.available_width().max(120.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, height), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, INSET);
    let top = hist.iter().fold(1.0f32, |m, &v| m.max(v));
    let scale = nice_ceiling(top * 1.15);
    let plot = rect.shrink2(vec2(4.0, 4.0));
    let y_of = |v: f32| plot.bottom() - (v / scale).clamp(0.0, 1.0) * plot.height();
    for k in [0.5f32, 1.0] {
        let y = y_of(scale * k);
        p.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.0_f32, Color32::from_rgb(0x22, 0x29, 0x33)));
    }
    p.text(pos2(plot.left() + 2.0, y_of(scale) + 1.0), Align2::LEFT_TOP, format!("{scale} Mbit/s"), FontId::proportional(10.0), DIM);
    if hist.len() >= 2 {
        let n = crate::mpx::HIST as f32;
        let start = n - hist.len() as f32;
        let pts: Vec<egui::Pos2> = hist.iter().enumerate().map(|(i, &v)| pos2(plot.left() + (start + i as f32) / (n - 1.0) * plot.width(), y_of(v))).collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.6_f32, color)));
    } else {
        p.text(rect.center(), Align2::CENTER_CENTER, "no data yet", FontId::proportional(11.0), DIM);
    }
    p.text(pos2(plot.right() - 2.0, plot.bottom()), Align2::RIGHT_BOTTOM, "2 min", FontId::proportional(10.0), DIM);
}

fn nice_ceiling(v: f32) -> f32 {
    for s in [1.0f32, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0] {
        if v <= s {
            return s;
        }
    }
    v.ceil()
}

pub fn mbps(v: f32) -> String {
    format!("{v:.2}")
}

/// Program icon for the window.
pub fn icon() -> egui::IconData {
    let rgba = image::load_from_memory(include_bytes!("../packaging/icon.png")).map(|i| i.to_rgba8().into_raw()).unwrap_or_else(|_| vec![0; 256 * 256 * 4]);
    egui::IconData { rgba, width: 256, height: 256 }
}
