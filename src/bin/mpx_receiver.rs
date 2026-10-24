//! RadioMoon MPX Receiver - a small separate program for the transmitter site.
//! Receives the MPX stream sent by the RadioMoon MPX Transmitter over IP (UDP/RTP or TCP; L24 or FLAC) and plays it on a 192 kHz sound card.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use eframe::egui::{self, RichText};
use radiomoon_mpx::look::{self, card, pill, tile};
use radiomoon_mpx::mpxrx::{RxCfg, RxHandle, RxView};
use radiomoon_mpx::mpxscope;
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Saved {
    proto: String,
    port: u16,
    device: String,
    buffer_ms: u32,
    autostart: bool,
}

impl Default for Saved {
    fn default() -> Self {
        Saved { proto: "udp".into(), port: 5004, device: String::new(), buffer_ms: 250, autostart: true }
    }
}

fn cfg_path() -> PathBuf {
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    if exe.join("portable.flag").exists() {
        return exe.join("mpx-receiver.json");
    }
    let old = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).filter(|o| o.join("RadioMoon").join("mpx-receiver.json").exists());
    let base = old
        .or_else(|| std::env::var_os("PROGRAMDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or(exe);
    let dir = base.join("RadioMoon");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("mpx-receiver.json")
}

struct App {
    s: Saved,
    rx: Option<RxHandle>,
    devs: Vec<String>,
    first: bool,
    about: bool,
    about_tex: Option<egui::TextureHandle>,
    logo_tex: Option<egui::TextureHandle>,
}

impl App {
    fn start(&mut self) {
        self.stop();
        self.rx = Some(RxHandle::start(&RxCfg { proto: self.s.proto.clone(), port: self.s.port, device: self.s.device.clone(), buffer_ms: self.s.buffer_ms }));
    }
    fn stop(&mut self) {
        if let Some(r) = self.rx.take() {
            r.stop();
        }
    }
    /// Small program icon for the header.
    fn logo(&mut self, ctx: &egui::Context) -> egui::TextureId {
        if self.logo_tex.is_none() {
            if let Ok(img) = image::load_from_memory(look::RECEIVER_ICON) {
                let img = img.resize(64, 64, image::imageops::FilterType::Lanczos3).to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                self.logo_tex = Some(ctx.load_texture("logo", egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR));
            }
        }
        self.logo_tex.as_ref().map(|t| t.id()).unwrap_or_default()
    }

    fn persist(&self) {
        if let Ok(js) = serde_json::to_string_pretty(&self.s) {
            let _ = std::fs::write(cfg_path(), js);
        }
    }
}

fn cap(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(look::DIM));
}

impl App {
    fn header(&mut self, ui: &mut egui::Ui, v: &RxView) {
        ui.horizontal(|ui| {
            ui.add(egui::Image::new((self.logo(ui.ctx()), egui::vec2(20.0, 20.0))));
            ui.label(RichText::new("RadioMoon MPX Receiver").size(15.0).strong());
            ui.label(RichText::new(format!("v{VERSION}")).small().color(look::DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("About").clicked() {
                    self.about = true;
                }
                if self.rx.is_none() {
                    if look::primary_button(ui, "▶  Start").clicked() {
                        self.start();
                    }
                    pill(ui, "STOPPED", look::DIM);
                } else {
                    if look::danger_button(ui, "■  Stop").clicked() {
                        self.stop();
                    }
                    if v.receiving {
                        pill(ui, "RECEIVING", look::GOOD);
                    } else {
                        pill(ui, "NO SIGNAL", look::WARN);
                    }
                }
            });
        });
    }

    /// Live numbers of the link: bandwidth, format, buffer, losses and the history graph.
    fn link_card(&self, ui: &mut egui::Ui, v: &RxView) {
        let Some(r) = &self.rx else { return };
        card(ui, "Link", |ui| {
            ui.horizontal_wrapped(|ui| {
                tile(ui, "Bandwidth", &look::mbps(v.mbps), "Mbit/s", look::ACCENT, 120.0);
                let bits = if v.stats.bits == 0 { 24 } else { v.stats.bits };
                let fmt = if !v.receiving { "-".to_string() } else if v.flac { "FLAC".to_string() } else { format!("L{bits}") };
                tile(ui, "Format", &fmt, &if v.receiving { format!("{bits}-bit, 192 kHz") } else { String::new() }, look::TEXT, 130.0);
                let low = v.buffer_ms < self.s.buffer_ms / 3;
                tile(ui, "Buffer", &format!("{}", v.buffer_ms), "ms", if v.receiving && low { look::WARN } else { look::TEXT }, 90.0);
                let lost = v.stats.lost + v.stats.bad;
                tile(ui, "Lost packets", &format!("{lost}"), &format!("{:.0} ms hidden", v.stats.concealed as f64 / 192.0), if lost > 0 { look::WARN } else { look::GOOD }, 140.0);
                tile(ui, "Received", &format!("{:.1}", v.total_mb), "MB", look::TEXT, 100.0);
            });
            ui.add_space(4.0);
            look::sparkline(ui, &v.hist, 52.0, look::ACCENT);
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                look::dot(ui, if v.receiving { look::GOOD } else { look::WARN });
                ui.label(if v.receiving { format!("From {}", v.from) } else { r.status_text() });
            });
            if !v.card.is_empty() {
                ui.label(RichText::new(&v.card).small().color(look::DIM));
            }
        });
    }

    fn settings_cards(&mut self, ui: &mut egui::Ui) -> bool {
        let mut ch = false;
        card(ui, "Input (from the transmitter)", |ui| {
            egui::Grid::new("in").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                cap(ui, "Protocol");
                ui.horizontal(|ui| {
                    ch |= ui.selectable_value(&mut self.s.proto, "udp".to_string(), "UDP (RTP)").changed();
                    ch |= ui.selectable_value(&mut self.s.proto, "tcp".to_string(), "TCP").changed();
                });
                ui.end_row();
                cap(ui, "Port");
                ch |= ui.add(egui::DragValue::new(&mut self.s.port).range(1..=65535)).changed();
                ui.end_row();
            });
            ui.label(RichText::new("The format (L16 / L20 / L24 or FLAC 16 / 20 / 24-bit) is detected automatically. Lost packets are concealed without a jump of the pilot or RDS phase.").small().color(look::DIM));
        });
        card(ui, "Output (exciter MPX input)", |ui| {
            egui::Grid::new("out").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                cap(ui, "Sound device");
                let shown = if self.s.device.is_empty() { "Default".to_string() } else { self.s.device.clone() };
                let w = (ui.available_width() - 10.0).clamp(150.0, 360.0);
                egui::ComboBox::from_id_salt("dev").width(w).selected_text(shown).show_ui(ui, |ui| {
                    ch |= ui.selectable_value(&mut self.s.device, String::new(), "Default").changed();
                    for d in &self.devs {
                        ch |= ui.selectable_value(&mut self.s.device, d.clone(), d).changed();
                    }
                });
                ui.end_row();
                cap(ui, "Buffer");
                ch |= ui.add(egui::DragValue::new(&mut self.s.buffer_ms).range(20..=2000).suffix(" ms")).changed();
                ui.end_row();
            });
            ch |= ui.checkbox(&mut self.s.autostart, "Start receiving when the program starts").changed();
            if self.rx.is_some() {
                ui.label(RichText::new("Changes are applied when you press Stop and Start.").small().color(look::DIM));
            }
        });
        ch
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _f: &mut eframe::Frame) {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        if self.first {
            self.first = false;
            if self.s.autostart {
                self.start();
            }
        }
        let v = self.rx.as_ref().map(|r| r.view()).unwrap_or_default();
        egui::TopBottomPanel::top("header").frame(egui::Frame::none().fill(look::CARD).inner_margin(egui::Margin::symmetric(12.0, 6.0))).show(ctx, |ui| {
            self.header(ui, &v);
        });
        look::about_window(
            ctx,
            &mut self.about,
            &mut self.about_tex,
            &look::About {
                name: "RadioMoon MPX Receiver",
                tagline: "Receives the MPX over IP and plays it into the exciter",
                features: &[
                    "UDP (RTP) or TCP, format detected automatically",
                    "L16 / L20 / L24 and FLAC lossless 16 / 20 / 24-bit",
                    "Lost packets hidden without a pilot or RDS phase jump",
                    "Live bandwidth, buffer and loss readouts with history",
                    "192 kHz sound card output and signal analyzer",
                ],
                png: look::RECEIVER_ICON,
                version: VERSION,
            },
        );
        egui::CentralPanel::default().frame(egui::Frame::none().fill(look::PANEL).inner_margin(10.0)).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let mut ch = false;
                if ui.available_width() >= 740.0 {
                    ui.columns(2, |cols| {
                        ch |= self.settings_cards(&mut cols[0]);
                        self.link_card(&mut cols[1], &v);
                    });
                } else {
                    self.link_card(ui, &v);
                    ch |= self.settings_cards(ui);
                }
                if let Some(r) = &self.rx {
                    card(ui, "Signal analysis", |ui| {
                        look::deviation_bar(ui, r.peak());
                        ui.add_space(6.0);
                        mpxscope::draw(ui, &r.scope);
                    });
                }
                if ch {
                    self.persist();
                }
            });
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop();
    }
}

fn main() -> eframe::Result<()> {
    let s: Saved = std::fs::read_to_string(cfg_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let devs: Vec<String> = rodio::cpal::default_host().output_devices().map(|d| d.filter_map(|x| x.name().ok()).collect()).unwrap_or_default();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([820.0, 700.0])
            .with_min_inner_size([460.0, 380.0])
            .with_title("RadioMoon MPX Receiver")
            .with_icon(std::sync::Arc::new(look::icon(look::RECEIVER_ICON))),
        ..Default::default()
    };
    eframe::run_native(
        "RadioMoon MPX Receiver",
        options,
        Box::new(|cc| {
            look::apply_style(&cc.egui_ctx);
            Ok(Box::new(App { s, rx: None, devs, first: true, about: false, about_tex: None, logo_tex: None }))
        }),
    )
}
