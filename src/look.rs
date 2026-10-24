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
        (TextStyle::Heading, FontId::new(15.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(12.5, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(11.5, FontFamily::Monospace)),
        (TextStyle::Button, FontId::new(12.5, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(10.5, FontFamily::Proportional)),
    ]);
    style.spacing.item_spacing = vec2(6.0, 4.0);
    style.spacing.button_padding = vec2(8.0, 2.0);
    style.spacing.interact_size = vec2(36.0, 21.0);
    style.spacing.icon_width = 13.0;
    style.spacing.icon_width_inner = 7.0;
    style.spacing.slider_width = 140.0;
    style.spacing.combo_height = 220.0;
    style.visuals = visuals();
    ctx.set_style(style);
}

/// A rounded panel with an optional blue title.
pub fn card<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let r = egui::Frame::none()
        .fill(CARD)
        .rounding(6.0)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).strong().size(12.0).color(ACCENT));
                ui.add_space(2.0);
            }
            add(ui)
        })
        .inner;
    ui.add_space(6.0);
    r
}

/// A darker box inside a card (one IP output, one value tile).
pub fn inset<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::none().fill(INSET).rounding(5.0).stroke(Stroke::new(1.0_f32, BORDER)).inner_margin(7.0).show(ui, add).inner
}

/// A value tile: small caption, large number, unit.
pub fn tile(ui: &mut Ui, caption: &str, value: &str, unit: &str, color: Color32, width: f32) {
    egui::Frame::none().fill(INSET).rounding(5.0).stroke(Stroke::new(1.0_f32, BORDER)).inner_margin(egui::Margin::symmetric(8.0, 5.0)).show(ui, |ui| {
        ui.set_width(width * 0.8);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(RichText::new(caption).small().color(DIM));
            ui.horizontal(|ui| {
                ui.label(RichText::new(value).size(16.0).strong().color(color));
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
        .rounding(10.0)
        .inner_margin(egui::Margin::symmetric(8.0, 1.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).strong().size(10.5).color(color));
        });
}

/// A small round state light.
pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

pub fn primary_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE).strong().size(12.5)).fill(Color32::from_rgb(0x1f, 0x8f, 0x55)).rounding(5.0).min_size(vec2(80.0, 24.0)))
}

pub fn danger_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(text).color(Color32::WHITE).strong().size(12.5)).fill(Color32::from_rgb(160, 45, 50)).rounding(5.0).min_size(vec2(80.0, 24.0)))
}

/// Deviation bar: 0..125%, green below 100%, red above.
pub fn deviation_bar(ui: &mut Ui, peak: f32) {
    let w = ui.available_width().max(120.0);
    let (rect, _) = ui.allocate_exact_size(vec2(w, 16.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, INSET);
    let x_of = |v: f32| rect.left() + (v.clamp(0.0, 1.25) / 1.25) * rect.width();
    let fill = Rect::from_min_max(rect.min, pos2(x_of(peak), rect.max.y));
    let c = if peak > 1.0 { BAD } else if peak > 0.95 { WARN } else { GOOD };
    p.rect_filled(fill, 4.0, c.gamma_multiply(0.85));
    let x100 = x_of(1.0);
    p.line_segment([pos2(x100, rect.top()), pos2(x100, rect.bottom())], Stroke::new(1.5_f32, BAD));
    p.text(rect.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, format!("{:.1} kHz  ({:.0}%)", peak * 75.0, peak * 100.0), FontId::proportional(10.5), Color32::WHITE);
    p.text(pos2(x100 - 4.0, rect.center().y), Align2::RIGHT_CENTER, "75 kHz", FontId::proportional(9.5), Color32::from_rgb(255, 200, 200));
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

pub const TRANSMITTER_ICON: &[u8] = include_bytes!("../packaging/transmitter.png");
pub const RECEIVER_ICON: &[u8] = include_bytes!("../packaging/receiver.png");

/// Program icon for the window (PNG bytes, e.g. `TRANSMITTER_ICON`).
pub fn icon(png: &[u8]) -> egui::IconData {
    match image::load_from_memory(png) {
        Ok(i) => {
            let i = i.to_rgba8();
            egui::IconData { width: i.width(), height: i.height(), rgba: i.into_raw() }
        }
        Err(_) => egui::IconData { rgba: vec![0; 4], width: 1, height: 1 },
    }
}

// ---------------------------------------------------------------- About window

/// What the About window of a program shows.
pub struct About {
    pub name: &'static str,
    pub tagline: &'static str,
    pub features: &'static [&'static str],
    /// The program icon (PNG).
    pub png: &'static [u8],
    pub version: &'static str,
}

/// Shows the About window while `open` is true. `tex` keeps the loaded logo between frames.
pub fn about_window(ctx: &egui::Context, open: &mut bool, tex: &mut Option<egui::TextureHandle>, a: &About) {
    if !*open {
        return;
    }
    if tex.is_none() {
        if let Ok(img) = image::load_from_memory(a.png) {
            let img = img.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            *tex = Some(ctx.load_texture("about-logo", egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR));
        }
    }
    let mut close = false;
    let mut shown = true;
    egui::Window::new(RichText::new("About").strong())
        .open(&mut shown)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .fixed_size(vec2(380.0, 0.0))
        .frame(egui::Frame::window(&ctx.style()).fill(CARD).stroke(Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.5))).inner_margin(egui::Margin::same(18.0)))
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                if let Some(t) = tex.as_ref() {
                    ui.add(egui::Image::new((t.id(), vec2(72.0, 72.0))));
                }
                ui.add_space(4.0);
                ui.label(RichText::new(a.name).size(19.0).strong().color(TEXT));
                ui.label(RichText::new(format!("Version {}", a.version)).monospace().color(GOOD));
                ui.add_space(2.0);
                ui.label(RichText::new(a.tagline).color(DIM));
            });
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(RichText::new("CREATED BY").small().strong().color(ACCENT));
            egui::Grid::new("about_credits").num_columns(2).spacing([14.0, 4.0]).show(ui, |ui| {
                ui.label(RichText::new("Nikos Batsikas").strong().color(TEXT));
                ui.label(RichText::new("Author - idea, design, testing").color(DIM));
                ui.end_row();
                ui.label(RichText::new("Claude").strong().color(TEXT));
                ui.label(RichText::new("AI pair programmer by Anthropic").color(DIM));
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.label(RichText::new("FEATURES").small().strong().color(ACCENT));
            for line in a.features {
                ui.label(RichText::new(format!("\u{2022}  {line}")).color(TEXT));
            }
            ui.add_space(8.0);
            ui.label(RichText::new("PROJECT").small().strong().color(ACCENT));
            ui.hyperlink_to("github.com/nikosbatsikas/radiomoon-mpx", "https://github.com/nikosbatsikas/radiomoon-mpx");
            ui.label(RichText::new("Part of RadioMoon - github.com/nikosbatsikas/radiomoon").small().color(DIM));
            ui.label(RichText::new("Written in Rust - egui / eframe - rodio").small().color(DIM));
            ui.label(RichText::new("\u{A9} 2026 Nikos Batsikas").small().color(DIM));
            ui.add_space(10.0);
            egui::Frame::none().fill(INSET).stroke(Stroke::new(1.0_f32, BORDER)).rounding(5.0).inner_margin(egui::Margin::same(9.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Like RadioMoon MPX?").strong().color(TEXT));
                        ui.label(RichText::new("Send a gift to the author.").small().color(DIM));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let btn = egui::Button::new(RichText::new("Gift with PayPal").strong().size(12.0).color(Color32::from_rgb(0x00, 0x30, 0x87)))
                            .fill(Color32::from_rgb(0xFF, 0xC4, 0x39))
                            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x00, 0x70, 0xBA)))
                            .rounding(12.0)
                            .min_size(vec2(130.0, 24.0));
                        if ui.add(btn).on_hover_text("paypal.me/nikosbatsikas").clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab("https://paypal.me/nikosbatsikas"));
                        }
                    });
                });
            });
            ui.add_space(10.0);
            ui.vertical_centered(|ui| {
                if ui.add(egui::Button::new(RichText::new("Close").strong().color(Color32::WHITE)).fill(Color32::from_rgb(0x1f, 0x7f, 0xc4)).min_size(vec2(90.0, 24.0))).clicked() {
                    close = true;
                }
            });
        });
    if close || !shown {
        *open = false;
    }
}

// ---------------------------------------------------------------- window that fits its content

/// Makes the window as high as its content (`want`, in points), within the screen. Call once per frame after the
/// layout; `last` remembers the last request so the size is only sent when it changes. Not while maximised.
pub fn fit_height(ctx: &egui::Context, want: f32, last: &mut f32) {
    let (cur, monitor, maximized, fullscreen) = ctx.input(|i| (i.screen_rect().size(), i.viewport().monitor_size, i.viewport().maximized, i.viewport().fullscreen));
    if maximized == Some(true) || fullscreen == Some(true) {
        return;
    }
    let max_h = monitor.map(|m| m.y * 0.9).unwrap_or(1200.0).max(320.0);
    let target = want.clamp(320.0, max_h).round();
    if (target - cur.y).abs() > 4.0 && (target - *last).abs() > 0.5 {
        *last = target;
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(cur.x, target)));
    }
}
