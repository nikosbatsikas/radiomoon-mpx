//! RadioMoon MPX Transmitter - a small stand-alone program: it takes audio from a sound card input, an internet radio
//! stream or a folder of audio files (random order), builds the FM multiplex (stereo, pilot, RDS) and sends it to a
//! 192 kHz sound card and/or over IP (to the RadioMoon MPX Receiver). It runs by itself, no RadioMoon needed.
//! To put RadioMoon on air, use its internet broadcast as the stream source: the song titles become the RadioText.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use radiomoon_mpx::{model, mpx, mpxscope, srcin};
use eframe::egui::{self, Color32, RichText};
use model::MpxCfg;
use mpx::MpxHandle;
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};
use srcin::{SrcCfg, SrcHandle};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Saved {
    /// "input" | "stream" | "folder"
    kind: String,
    device: String,
    url: String,
    folder: String,
    shuffle: bool,
    gain_db: f32,
    mpx: MpxCfg,
}

impl Default for Saved {
    fn default() -> Self {
        let mpx = MpxCfg { autostart: true, card_on: true, rt_from_playlist: true, ..MpxCfg::default() };
        Saved { kind: "folder".into(), device: String::new(), url: String::new(), folder: String::new(), shuffle: true, gain_db: 0.0, mpx }
    }
}

fn cfg_path() -> PathBuf {
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    if exe.join("portable.flag").exists() {
        return exe.join("mpx-transmitter.json");
    }
    let base = std::env::var_os("PROGRAMDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or(exe);
    let dir = base.join("RadioMoon");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("mpx-transmitter.json")
}

struct App {
    s: Saved,
    src: Option<SrcHandle>,
    mpx: Option<MpxHandle>,
    outs: Vec<String>,
    ins: Vec<String>,
    last_now: String,
    first: bool,
}

impl App {
    fn start(&mut self) {
        self.stop();
        let c = SrcCfg { kind: self.s.kind.clone(), device: self.s.device.clone(), url: self.s.url.clone(), folder: self.s.folder.clone(), shuffle: self.s.shuffle, gain_db: self.s.gain_db };
        let (h, rx) = srcin::start(&c);
        self.mpx = Some(MpxHandle::start(&self.s.mpx, rx));
        self.src = Some(h);
        self.last_now.clear();
    }

    fn stop(&mut self) {
        if let Some(m) = self.mpx.take() {
            m.stop();
        }
        if let Some(s) = self.src.take() {
            s.stop();
        }
    }

    fn persist(&self) {
        if let Ok(js) = serde_json::to_string_pretty(&self.s) {
            let _ = std::fs::write(cfg_path(), js);
        }
    }
}

fn combo(ui: &mut egui::Ui, id: &str, cur: &mut String, items: &[String]) -> bool {
    let mut ch = false;
    let shown = if cur.is_empty() { "Default".to_string() } else { cur.clone() };
    egui::ComboBox::from_id_salt(id).width(340.0).selected_text(shown).show_ui(ui, |ui| {
        ch |= ui.selectable_value(cur, String::new(), "Default").changed();
        for d in items {
            ch |= ui.selectable_value(cur, d.clone(), d).changed();
        }
    });
    ch
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _f: &mut eframe::Frame) {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
        if self.first {
            self.first = false;
            if self.s.mpx.autostart {
                self.start();
            }
        }
        // RadioText follows the file that is playing
        if let (Some(src), Some(m)) = (&self.src, &self.mpx) {
            let now = src.now();
            if !now.is_empty() && now != self.last_now {
                m.set_radiotext(&self.s.mpx, &now);
                self.last_now = now;
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.heading("RadioMoon MPX Transmitter");
                ui.label("Audio source -> FM multiplex (stereo, pilot, RDS) -> 192 kHz sound card and / or IP.");
                ui.add_space(6.0);
                let mut ch = false; // changes that can be applied while running
                let mut restart = false; // changes of the source or the outputs
                ui.group(|ui| {
                    ui.label(RichText::new("Audio source").strong());
                    ui.horizontal(|ui| {
                        restart |= ui.radio_value(&mut self.s.kind, "folder".to_string(), "Folder of audio files").changed();
                        restart |= ui.radio_value(&mut self.s.kind, "stream".to_string(), "Internet stream").changed();
                        restart |= ui.radio_value(&mut self.s.kind, "input".to_string(), "Sound input").changed();
                    });
                    match self.s.kind.as_str() {
                        "folder" => {
                            ui.horizontal(|ui| {
                                ui.label("Folder");
                                restart |= ui.add(egui::TextEdit::singleline(&mut self.s.folder).desired_width((ui.available_width() - 100.0).max(100.0))).changed();
                                if ui.button("Browse...").clicked() {
                                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                                        self.s.folder = p.to_string_lossy().to_string();
                                        restart = true;
                                    }
                                }
                            });
                            restart |= ui.checkbox(&mut self.s.shuffle, "Random order (a different order at every start)").changed();
                        }
                        "stream" => {
                            ui.horizontal(|ui| {
                                ui.label("Stream URL");
                                restart |= ui.add(egui::TextEdit::singleline(&mut self.s.url).desired_width((ui.available_width() - 8.0).max(100.0))).changed();
                            });
                            ui.label(RichText::new("MP3 or AAC over HTTP / HTTPS").small());
                        }
                        _ => {
                            ui.horizontal(|ui| {
                                ui.label("Sound input");
                                restart |= combo(ui, "in", &mut self.s.device, &self.ins);
                            });
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label("Level (dB)");
                        restart |= ui.add(egui::Slider::new(&mut self.s.gain_db, -24.0..=12.0)).changed();
                    });
                });
                ui.add_space(4.0);
                ui.group(|ui| {
                    ui.label(RichText::new("Outputs").strong());
                    let c = &mut self.s.mpx;
                    restart |= ui.checkbox(&mut c.card_on, "192 kHz sound card (MPX input of the exciter)").changed();
                    ui.horizontal(|ui| {
                        ui.label("Device");
                        restart |= combo(ui, "out", &mut c.card_device, &self.outs);
                    });
                    restart |= ui.checkbox(&mut c.ip_on, "Send over IP (24-bit, 192 kHz)").changed();
                    ui.horizontal(|ui| {
                        ui.label("Format");
                        restart |= ui.radio_value(&mut c.ip_codec, "l24".to_string(), "L24 uncompressed (4.6 Mbit/s)").changed();
                        restart |= ui.radio_value(&mut c.ip_codec, "flac".to_string(), "FLAC lossless (bit-exact, about 3-4 Mbit/s)").changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("Protocol");
                        restart |= ui.radio_value(&mut c.ip_proto, "udp".to_string(), "UDP (RTP)").changed();
                        restart |= ui.radio_value(&mut c.ip_proto, "tcp".to_string(), "TCP").changed();
                        ui.label("Address");
                        restart |= ui.add(egui::TextEdit::singleline(&mut c.ip_host).desired_width(130.0)).changed();
                        ui.label("Port");
                        restart |= ui.add(egui::DragValue::new(&mut c.ip_port).range(1..=65535)).changed();
                    });
                });
                ui.add_space(4.0);
                ui.group(|ui| {
                    ui.label(RichText::new("Signal").strong());
                    let c = &mut self.s.mpx;
                    let pct = |v: f64, _: std::ops::RangeInclusive<usize>| format!("{:.0}%", v * 100.0);
                    ch |= ui.checkbox(&mut c.stereo, "Stereo (19 kHz pilot and 38 kHz subcarrier)").changed();
                    ch |= ui.add(egui::Slider::new(&mut c.audio_level, 0.3..=1.0).custom_formatter(pct).text("Audio level")).changed();
                    ch |= ui.add(egui::Slider::new(&mut c.pilot_level, 0.0..=0.12).custom_formatter(pct).text("Pilot level")).changed();
                    ch |= ui.add(egui::Slider::new(&mut c.rds_level, 0.0..=0.08).custom_formatter(pct).text("RDS level")).changed();
                    ch |= ui.add(egui::Slider::new(&mut c.output, 0.1..=1.0).custom_formatter(pct).text("Output level")).changed();
                    ui.horizontal(|ui| {
                        ui.label("Pre-emphasis");
                        ch |= ui.radio_value(&mut c.preemph_us, 50.0, "50 us").changed();
                        ch |= ui.radio_value(&mut c.preemph_us, 75.0, "75 us").changed();
                        ch |= ui.radio_value(&mut c.preemph_us, 0.0, "Off").changed();
                    });
                    ch |= ui.checkbox(&mut c.limiter, "Peak limiter").changed();
                });
                ui.add_space(4.0);
                ui.group(|ui| {
                    ui.label(RichText::new("RDS").strong());
                    let c = &mut self.s.mpx;
                    ch |= ui.checkbox(&mut c.rds_on, "Send RDS").changed();
                    ui.horizontal(|ui| {
                        ui.label("PI (hex)");
                        ch |= ui.add(egui::TextEdit::singleline(&mut c.pi).desired_width(60.0)).changed();
                        ui.label("Station name (8 characters)");
                        ch |= ui.add(egui::TextEdit::singleline(&mut c.ps).desired_width(110.0).char_limit(8)).changed();
                        ui.label("PTY");
                        ch |= ui.add(egui::DragValue::new(&mut c.pty).range(0..=31)).changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("RadioText");
                        ch |= ui.add(egui::TextEdit::singleline(&mut c.rt).desired_width(320.0).char_limit(64)).changed();
                    });
                    ch |= ui.checkbox(&mut c.rt_from_playlist, "RadioText from the song: file name (folder) or song title sent by the stream server").changed();
                    ui.horizontal(|ui| {
                        ch |= ui.checkbox(&mut c.tp, "TP").changed();
                        ch |= ui.checkbox(&mut c.ta, "TA").changed();
                        ch |= ui.checkbox(&mut c.music, "Music").changed();
                    });
                });
                ui.add_space(4.0);
                ui.checkbox(&mut self.s.mpx.autostart, "Start transmitting when the program starts").changed();
                ui.add_space(6.0);
                let running = self.mpx.is_some();
                ui.horizontal(|ui| {
                    if !running {
                        if ui.button(RichText::new("  Start  ").size(16.0)).clicked() {
                            self.start();
                        }
                    } else if ui.button(RichText::new("  Stop  ").size(16.0)).clicked() {
                        self.stop();
                    }
                });
                if restart && running {
                    ui.label(RichText::new("The source and output changes are applied when you press Stop and Start.").small());
                }
                ui.add_space(6.0);
                match (&self.src, &self.mpx) {
                    (Some(s), Some(m)) => {
                        ui.colored_label(Color32::from_rgb(120, 230, 150), s.status());
                        for l in m.status_lines() {
                            ui.label(l);
                        }
                        let p = m.peak();
                        ui.add(egui::ProgressBar::new(p.min(1.0)).desired_width(380.0).text(format!("{:.1} kHz  ({:.0}%)", p * 75.0, p * 100.0)));
                        ui.add_space(6.0);
                        mpxscope::draw(ui, &m.scope);
                        if ch {
                            m.update(&self.s.mpx);
                        }
                    }
                    _ => {
                        ui.label("Stopped");
                    }
                }
                if ch || restart {
                    self.persist();
                }
            });
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop();
        self.persist();
    }
}

fn main() -> eframe::Result<()> {
    let s: Saved = std::fs::read_to_string(cfg_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let outs: Vec<String> = rodio::cpal::default_host().output_devices().map(|d| d.filter_map(|x| x.name().ok()).collect()).unwrap_or_default();
    let ins = srcin::list_inputs();
    let icon = image::load_from_memory(include_bytes!("../../packaging/icon.png")).map(|i| i.to_rgba8().into_raw()).unwrap_or_else(|_| vec![0; 256 * 256 * 4]);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([660.0, 860.0])
            .with_min_inner_size([560.0, 420.0])
            .with_title("RadioMoon MPX Transmitter")
            .with_icon(std::sync::Arc::new(egui::IconData { rgba: icon, width: 256, height: 256 })),
        ..Default::default()
    };
    eframe::run_native(
        "RadioMoon MPX Transmitter",
        options,
        Box::new(|cc| {
            let mut fd = egui::FontDefinitions::default();
            fd.font_data.insert("dejavu".into(), egui::FontData::from_static(include_bytes!("../../assets/fonts/DejaVuSans.ttf")));
            fd.families.entry(egui::FontFamily::Proportional).or_default().push("dejavu".into());
            cc.egui_ctx.set_fonts(fd);
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            let mut v = egui::Visuals::dark();
            v.override_text_color = Some(Color32::from_rgb(0xd6, 0xdc, 0xe4));
            v.panel_fill = Color32::from_rgb(0x11, 0x15, 0x1b);
            cc.egui_ctx.set_visuals(v);
            Ok(Box::new(App { s, src: None, mpx: None, outs, ins, last_now: String::new(), first: true }))
        }),
    )
}
