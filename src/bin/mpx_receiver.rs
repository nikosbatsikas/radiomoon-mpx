//! RadioMoon MPX Receiver - a small separate program for the transmitter site.
//! Receives the MPX stream sent by the RadioMoon MPX Transmitter over IP (UDP/RTP or TCP; L24 or FLAC) and plays it on a 192 kHz sound card.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use radiomoon_mpx::{mpxscope, mpxrx};
use eframe::egui::{self, Color32, RichText};
use mpxrx::{RxCfg, RxHandle};
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    fn persist(&self) {
        if let Ok(js) = serde_json::to_string_pretty(&self.s) {
            let _ = std::fs::write(cfg_path(), js);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _f: &mut eframe::Frame) {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
        if self.first {
            self.first = false;
            if self.s.autostart {
                self.start();
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("RadioMoon MPX Receiver");
            ui.label("Receives the MPX stream from the MPX Transmitter and plays it on a 192 kHz sound card (exciter MPX input).");
            ui.add_space(8.0);
            let mut ch = false;
            ui.horizontal(|ui| {
                ui.label("Protocol");
                ch |= ui.radio_value(&mut self.s.proto, "udp".to_string(), "UDP (RTP)").changed();
                ch |= ui.radio_value(&mut self.s.proto, "tcp".to_string(), "TCP").changed();
                ui.label("Port");
                ch |= ui.add(egui::DragValue::new(&mut self.s.port).range(1..=65535)).changed();
            });
            ui.label(RichText::new("The format is detected automatically: L24 uncompressed or FLAC lossless. Lost packets are concealed without a jump of the pilot or RDS phase.").small());
            ui.horizontal(|ui| {
                ui.label("Sound device");
                let shown = if self.s.device.is_empty() { "Default".to_string() } else { self.s.device.clone() };
                egui::ComboBox::from_id_salt("dev").width(340.0).selected_text(shown).show_ui(ui, |ui| {
                    ch |= ui.selectable_value(&mut self.s.device, String::new(), "Default").changed();
                    for d in &self.devs {
                        ch |= ui.selectable_value(&mut self.s.device, d.clone(), d).changed();
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label("Buffer");
                ch |= ui.add(egui::DragValue::new(&mut self.s.buffer_ms).range(20..=2000).suffix(" ms")).changed();
                ch |= ui.checkbox(&mut self.s.autostart, "Start receiving when the program starts").changed();
            });
            ui.add_space(8.0);
            let running = self.rx.is_some();
            ui.horizontal(|ui| {
                if !running {
                    if ui.button(RichText::new("  Start  ").size(16.0)).clicked() {
                        self.start();
                    }
                } else if ui.button(RichText::new("  Stop  ").size(16.0)).clicked() {
                    self.stop();
                }
            });
            ui.add_space(6.0);
            match &self.rx {
                Some(r) => {
                    ui.colored_label(Color32::from_rgb(120, 230, 150), r.status_text());
                    let p = r.peak();
                    ui.add(egui::ProgressBar::new(p.min(1.0)).desired_width(380.0).text(format!("{:.1} kHz  ({:.0}%)", p * 75.0, p * 100.0)));
                    ui.add_space(6.0);
                    mpxscope::draw(ui, &r.scope);
                }
                None => {
                    ui.label("Stopped");
                }
            }
            ui.add_space(6.0);
            ui.label(RichText::new("Changes are applied when you press Start again.").small());
            if ch {
                self.persist();
            }
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop();
    }
}

fn main() -> eframe::Result<()> {
    let s: Saved = std::fs::read_to_string(cfg_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let devs: Vec<String> = rodio::cpal::default_host().output_devices().map(|d| d.filter_map(|x| x.name().ok()).collect()).unwrap_or_default();
    let icon = image::load_from_memory(include_bytes!("../../packaging/icon.png")).map(|i| i.to_rgba8().into_raw()).unwrap_or_else(|_| vec![0; 256 * 256 * 4]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 700.0])
            .with_min_inner_size([520.0, 360.0])
            .with_title("RadioMoon MPX Receiver")
            .with_icon(std::sync::Arc::new(egui::IconData { rgba: icon, width: 256, height: 256 })),
        ..Default::default()
    };
    eframe::run_native(
        "RadioMoon MPX Receiver",
        options,
        Box::new(|cc| {
            let mut fd = egui::FontDefinitions::default();
            fd.font_data.insert("dejavu".into(), egui::FontData::from_static(include_bytes!("../../assets/fonts/DejaVuSans.ttf")));
            fd.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "dejavu".into());
            cc.egui_ctx.set_fonts(fd);
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            let mut v = egui::Visuals::dark();
            v.override_text_color = Some(Color32::WHITE);
            v.panel_fill = Color32::from_rgb(12, 14, 18);
            cc.egui_ctx.set_visuals(v);
            Ok(Box::new(App { s, rx: None, devs, first: true }))
        }),
    )
}
