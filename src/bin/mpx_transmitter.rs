//! RadioMoon MPX Transmitter - a small stand-alone program: it takes audio from a sound card input, an internet radio
//! stream or a folder of audio files (random order), builds the FM multiplex (stereo, pilot, RDS) and sends it to a
//! 192 kHz sound card and/or over IP to any number of RadioMoon MPX Receivers. It runs by itself, no RadioMoon needed.
//! To put RadioMoon on air, use its internet broadcast as the stream source: the song titles become the RadioText.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use eframe::egui::{self, RichText};
use radiomoon_mpx::look::{self, card, inset, pill, tile};
use radiomoon_mpx::model::{IpTarget, MpxCfg};
use radiomoon_mpx::mpx::{LinkStat, MpxHandle};
use radiomoon_mpx::mpxscope;
use radiomoon_mpx::srcin::{self, SrcCfg, SrcHandle};
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const VERSION: &str = env!("CARGO_PKG_VERSION");

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
    /// Settings that only apply after Stop / Start were changed while running.
    pending: bool,
}

impl App {
    fn start(&mut self) {
        self.stop();
        let c = SrcCfg { kind: self.s.kind.clone(), device: self.s.device.clone(), url: self.s.url.clone(), folder: self.s.folder.clone(), shuffle: self.s.shuffle, gain_db: self.s.gain_db };
        let (h, rx) = srcin::start(&c);
        self.mpx = Some(MpxHandle::start(&self.s.mpx, rx));
        self.src = Some(h);
        self.last_now.clear();
        self.pending = false;
    }

    fn stop(&mut self) {
        if let Some(m) = self.mpx.take() {
            m.stop();
        }
        if let Some(s) = self.src.take() {
            s.stop();
        }
        self.pending = false;
    }

    fn persist(&self) {
        if let Ok(js) = serde_json::to_string_pretty(&self.s) {
            let _ = std::fs::write(cfg_path(), js);
        }
    }
}

fn combo(ui: &mut egui::Ui, id: &str, cur: &mut String, items: &[String], width: f32) -> bool {
    let mut ch = false;
    let shown = if cur.is_empty() { "Default".to_string() } else { cur.clone() };
    egui::ComboBox::from_id_salt(id).width(width).selected_text(shown).show_ui(ui, |ui| {
        ch |= ui.selectable_value(cur, String::new(), "Default").changed();
        for d in items {
            ch |= ui.selectable_value(cur, d.clone(), d).changed();
        }
    });
    ch
}

/// Label column of the settings grids.
fn cap(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(look::DIM));
}

impl App {
    // ------------------------------------------------------------ header
    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("RadioMoon MPX Transmitter").size(19.0).strong());
            ui.label(RichText::new(format!("v{VERSION}")).small().color(look::DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.mpx.is_none() {
                    if look::primary_button(ui, "▶  Start").clicked() {
                        self.start();
                    }
                    pill(ui, "STOPPED", look::DIM);
                } else {
                    if look::danger_button(ui, "■  Stop").clicked() {
                        self.stop();
                    }
                    let waiting = self.mpx.as_ref().map(|m| m.status().starts_with("Waiting")).unwrap_or(false);
                    if waiting {
                        pill(ui, "NO AUDIO", look::WARN);
                    } else {
                        pill(ui, "ON AIR", look::GOOD);
                    }
                }
            });
        });
    }

    // ------------------------------------------------------------ live overview
    fn overview(&mut self, ui: &mut egui::Ui, links: &[(IpTarget, LinkStat)]) {
        let (Some(src), Some(m)) = (&self.src, &self.mpx) else { return };
        card(ui, "On air", |ui| {
            look::deviation_bar(ui, m.peak());
            ui.add_space(6.0);
            let total: f32 = links.iter().map(|(_, l)| l.mbps).sum();
            let up = links.iter().filter(|(_, l)| l.ok).count();
            ui.horizontal_wrapped(|ui| {
                tile(ui, "Deviation peak", &format!("{:.1}", m.peak() * 75.0), "kHz", if m.peak() > 1.0 { look::BAD } else { look::TEXT }, 120.0);
                tile(ui, "IP bandwidth (total)", &look::mbps(total), "Mbit/s", look::ACCENT, 150.0);
                tile(ui, "IP outputs", &format!("{up} / {}", links.len()), "working", if up == links.len() { look::GOOD } else { look::WARN }, 120.0);
                let card_on = !m.card_status().is_empty();
                tile(ui, "Sound card", if card_on { "On" } else { "Off" }, "192 kHz", if card_on { look::GOOD } else { look::DIM }, 100.0);
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                look::dot(ui, look::GOOD);
                ui.label(src.status());
            });
            let now = src.now();
            if !now.is_empty() {
                ui.label(RichText::new(format!("RadioText: {now}")).color(look::DIM));
            }
            let cs = m.card_status();
            if !cs.is_empty() {
                ui.label(RichText::new(cs).small().color(look::DIM));
            }
        });
    }

    // ------------------------------------------------------------ settings cards
    fn source_card(&mut self, ui: &mut egui::Ui) -> bool {
        let mut restart = false;
        card(ui, "Audio source", |ui| {
            ui.horizontal_wrapped(|ui| {
                restart |= ui.selectable_value(&mut self.s.kind, "folder".to_string(), "Folder").changed();
                restart |= ui.selectable_value(&mut self.s.kind, "stream".to_string(), "Internet stream").changed();
                restart |= ui.selectable_value(&mut self.s.kind, "input".to_string(), "Sound input").changed();
            });
            ui.add_space(4.0);
            match self.s.kind.as_str() {
                "folder" => {
                    ui.horizontal(|ui| {
                        restart |= ui.add(egui::TextEdit::singleline(&mut self.s.folder).hint_text("Folder of audio files").desired_width((ui.available_width() - 90.0).max(100.0))).changed();
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
                    restart |= ui.add(egui::TextEdit::singleline(&mut self.s.url).hint_text("http://server:8000/stream").desired_width(ui.available_width())).changed();
                    ui.label(RichText::new("MP3 or AAC over HTTP / HTTPS. The song titles sent by the server become the RadioText.").small().color(look::DIM));
                }
                _ => {
                    let w = (ui.available_width() - 10.0).max(150.0);
                    restart |= combo(ui, "in", &mut self.s.device, &self.ins, w);
                }
            }
            ui.horizontal(|ui| {
                cap(ui, "Level");
                restart |= ui.add(egui::Slider::new(&mut self.s.gain_db, -24.0..=12.0).suffix(" dB")).changed();
            });
        });
        restart
    }

    fn signal_card(&mut self, ui: &mut egui::Ui) -> bool {
        let mut ch = false;
        card(ui, "Signal", |ui| {
            let c = &mut self.s.mpx;
            let pct = |v: f64, _: std::ops::RangeInclusive<usize>| format!("{:.0}%", v * 100.0);
            ch |= ui.checkbox(&mut c.stereo, "Stereo (19 kHz pilot and 38 kHz subcarrier)").changed();
            egui::Grid::new("sig").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                cap(ui, "Audio level");
                ch |= ui.add(egui::Slider::new(&mut c.audio_level, 0.3..=1.0).custom_formatter(pct)).changed();
                ui.end_row();
                cap(ui, "Pilot level");
                ch |= ui.add(egui::Slider::new(&mut c.pilot_level, 0.0..=0.12).custom_formatter(pct)).changed();
                ui.end_row();
                cap(ui, "RDS level");
                ch |= ui.add(egui::Slider::new(&mut c.rds_level, 0.0..=0.08).custom_formatter(pct)).changed();
                ui.end_row();
                cap(ui, "Output level");
                ch |= ui.add(egui::Slider::new(&mut c.output, 0.1..=1.0).custom_formatter(pct)).changed();
                ui.end_row();
                cap(ui, "Pre-emphasis");
                ui.horizontal(|ui| {
                    ch |= ui.selectable_value(&mut c.preemph_us, 50.0, "50 µs").changed();
                    ch |= ui.selectable_value(&mut c.preemph_us, 75.0, "75 µs").changed();
                    ch |= ui.selectable_value(&mut c.preemph_us, 0.0, "Off").changed();
                });
                ui.end_row();
            });
            ch |= ui.checkbox(&mut c.limiter, "Peak limiter").changed();
        });
        ch
    }

    fn rds_card(&mut self, ui: &mut egui::Ui) -> bool {
        let mut ch = false;
        card(ui, "RDS", |ui| {
            let c = &mut self.s.mpx;
            ch |= ui.checkbox(&mut c.rds_on, "Send RDS").changed();
            egui::Grid::new("rds").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                cap(ui, "Station name (PS)");
                ch |= ui.add(egui::TextEdit::singleline(&mut c.ps).desired_width(110.0).char_limit(8)).changed();
                ui.end_row();
                cap(ui, "PI code (hex)");
                ui.horizontal(|ui| {
                    ch |= ui.add(egui::TextEdit::singleline(&mut c.pi).desired_width(60.0).char_limit(4)).changed();
                    cap(ui, "PTY");
                    ch |= ui.add(egui::DragValue::new(&mut c.pty).range(0..=31)).changed();
                });
                ui.end_row();
                cap(ui, "RadioText");
                ch |= ui.add(egui::TextEdit::singleline(&mut c.rt).desired_width(ui.available_width().max(160.0)).char_limit(64)).changed();
                ui.end_row();
            });
            ch |= ui.checkbox(&mut c.rt_from_playlist, "RadioText from the song: file name (folder) or song title sent by the stream server").changed();
            ui.horizontal(|ui| {
                ch |= ui.checkbox(&mut c.tp, "TP").changed();
                ch |= ui.checkbox(&mut c.ta, "TA").changed();
                ch |= ui.checkbox(&mut c.music, "Music").changed();
            });
        });
        ch
    }

    fn card_output(&mut self, ui: &mut egui::Ui) -> bool {
        let mut restart = false;
        card(ui, "Sound card output", |ui| {
            let c = &mut self.s.mpx;
            restart |= ui.checkbox(&mut c.card_on, "192 kHz sound card (MPX input of the exciter)").changed();
            let on = c.card_on;
            let outs = &self.outs;
            ui.add_enabled_ui(on, |ui| {
                let w = (ui.available_width() - 10.0).max(150.0);
                restart |= combo(ui, "out", &mut c.card_device, outs, w);
            });
        });
        restart
    }

    /// The list of receivers, each with its settings and, while running, its live bandwidth.
    fn ip_card(&mut self, ui: &mut egui::Ui, links: &[(IpTarget, LinkStat)]) -> bool {
        let mut restart = false;
        let running = self.mpx.is_some();
        card(ui, "IP outputs", |ui| {
            let c = &mut self.s.mpx;
            restart |= ui.checkbox(&mut c.ip_on, "Send over IP (24-bit, 192 kHz) to these receivers").changed();
            ui.add_space(4.0);
            let mut remove: Option<usize> = None;
            let n = c.targets.len();
            let ip_on = c.ip_on;
            for (i, t) in c.targets.iter_mut().enumerate() {
                let live = links.iter().find(|(lt, _)| *lt == *t).map(|(_, l)| l);
                inset(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let color = match live {
                            Some(l) if l.ok => look::GOOD,
                            Some(_) => look::BAD,
                            None => look::DIM,
                        };
                        look::dot(ui, color);
                        restart |= ui.checkbox(&mut t.enabled, "").on_hover_text("Send to this receiver").changed();
                        restart |= ui.add(egui::TextEdit::singleline(&mut t.name).hint_text("Name (site)").desired_width(150.0)).changed();
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if n > 1 && ui.small_button("✖").on_hover_text("Remove this receiver").clicked() {
                                remove = Some(i);
                            }
                            if let Some(l) = live {
                                ui.label(RichText::new(format!("{} Mbit/s", look::mbps(l.mbps))).strong().size(15.0).color(look::ACCENT));
                            }
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        cap(ui, "Address");
                        restart |= ui.add(egui::TextEdit::singleline(&mut t.host).hint_text("IP or host name").desired_width(140.0)).changed();
                        cap(ui, "Port");
                        restart |= ui.add(egui::DragValue::new(&mut t.port).range(1..=65535)).changed();
                        egui::ComboBox::from_id_salt(("proto", i)).width(100.0).selected_text(if t.proto == "tcp" { "TCP" } else { "UDP (RTP)" }).show_ui(ui, |ui| {
                            restart |= ui.selectable_value(&mut t.proto, "udp".to_string(), "UDP (RTP)").changed();
                            restart |= ui.selectable_value(&mut t.proto, "tcp".to_string(), "TCP").changed();
                        });
                        egui::ComboBox::from_id_salt(("codec", i)).width(130.0).selected_text(if t.codec == "flac" { "FLAC lossless" } else { "PCM uncompressed" }).show_ui(ui, |ui| {
                            restart |= ui.selectable_value(&mut t.codec, "flac".to_string(), "FLAC lossless").changed();
                            restart |= ui.selectable_value(&mut t.codec, "l24".to_string(), "PCM uncompressed").changed();
                        });
                        let est = |codec: &str, bits: u32| -> f32 {
                            // typical bandwidth incl. headers; FLAC depends on the audio
                            let pcm = 192_000.0 * bits as f32 / 1e6 + 0.32;
                            if codec == "flac" { pcm * 0.8 } else { pcm }
                        };
                        let shown_bits = t.bits();
                        egui::ComboBox::from_id_salt(("bits", i)).width(170.0).selected_text(format!("{shown_bits}-bit, 192 kHz")).show_ui(ui, |ui| {
                            for b in [24u32, 20, 16] {
                                let txt = format!("{b}-bit (about {:.1} Mbit/s)", est(&t.codec, b));
                                restart |= ui.selectable_value(&mut t.bits, b, txt).changed();
                            }
                        });
                        egui::ComboBox::from_id_salt(("dscp", i)).width(110.0).selected_text(format!("QoS {}", t.dscp_name())).show_ui(ui, |ui| {
                            for (v, name) in [(46u8, "EF (46): real-time audio, recommended"), (34, "AF41 (34): AES67 audio"), (40, "CS5 (40)"), (48, "CS6 (48)"), (0, "Off: no marking")] {
                                restart |= ui.selectable_value(&mut t.dscp, v, name).changed();
                            }
                        })
                        .response
                        .on_hover_text("DSCP class written into every packet, so routers and switches with QoS give the MPX priority");
                        restart |= ui.add(egui::DragValue::new(&mut t.dscp).range(0..=63).prefix("DSCP ")).changed();
                    });
                    if let Some(l) = live {
                        let saving = if t.codec == "flac" && l.l24_mbps > 0.0 { format!(" · {:.0}% of L24", l.mbps / l.l24_mbps * 100.0) } else { String::new() };
                        ui.label(RichText::new(format!("{}{saving} · sent {:.1} MB", l.text, l.total_mb)).small().color(if l.ok { look::DIM } else { look::BAD }));
                        look::sparkline(ui, &l.hist, 46.0, look::ACCENT);
                    } else if running && t.enabled && ip_on {
                        ui.label(RichText::new("Changed: applied when you press Stop and Start").small().color(look::WARN));
                    }
                });
                ui.add_space(6.0);
            }
            if let Some(i) = remove {
                c.targets.remove(i);
                restart = true;
            }
            ui.horizontal(|ui| {
                if ui.button("+  Add receiver").clicked() {
                    let mut t = IpTarget::default();
                    if let Some(last) = c.targets.last() {
                        t.proto = last.proto.clone();
                        t.codec = last.codec.clone();
                        t.port = last.port;
                    }
                    t.name = format!("Receiver {}", c.targets.len() + 1);
                    c.targets.push(t);
                    restart = true;
                }
                let on: usize = c.targets.iter().filter(|t| t.enabled).count();
                ui.label(RichText::new(format!("{on} of {} enabled. Bandwidth includes the IP/UDP (or TCP) headers.", c.targets.len())).small().color(look::DIM));
            });
        });
        restart
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _f: &mut eframe::Frame) {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        if self.first {
            self.first = false;
            if self.s.mpx.autostart {
                self.start();
            }
        }
        // RadioText follows the file that is playing or the stream's song title
        if let (Some(src), Some(m)) = (&self.src, &self.mpx) {
            let now = src.now();
            if !now.is_empty() && now != self.last_now {
                m.set_radiotext(&self.s.mpx, &now);
                self.last_now = now;
            }
        }
        let links: Vec<(IpTarget, LinkStat)> = self.mpx.as_ref().map(|m| m.links()).unwrap_or_default();
        egui::TopBottomPanel::top("header").frame(egui::Frame::none().fill(look::CARD).inner_margin(egui::Margin::symmetric(16.0, 10.0))).show(ctx, |ui| {
            self.header(ui);
        });
        egui::CentralPanel::default().frame(egui::Frame::none().fill(look::PANEL).inner_margin(14.0)).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let mut ch = false; // changes applied while running
                let mut restart = false; // changes of the source or the outputs
                self.overview(ui, &links);
                let wide = ui.available_width() >= 900.0;
                if wide {
                    ui.columns(2, |cols| {
                        restart |= self.source_card(&mut cols[0]);
                        ch |= self.signal_card(&mut cols[0]);
                        ch |= self.rds_card(&mut cols[0]);
                        restart |= self.card_output(&mut cols[1]);
                        restart |= self.ip_card(&mut cols[1], &links);
                    });
                } else {
                    restart |= self.source_card(ui);
                    restart |= self.card_output(ui);
                    restart |= self.ip_card(ui, &links);
                    ch |= self.signal_card(ui);
                    ch |= self.rds_card(ui);
                }
                card(ui, "", |ui| {
                    ch |= ui.checkbox(&mut self.s.mpx.autostart, "Start transmitting when the program starts").changed();
                    if self.pending {
                        ui.label(RichText::new("Source and output changes are applied when you press Stop and Start.").color(look::WARN));
                    }
                });
                if let Some(m) = &self.mpx {
                    card(ui, "Signal analysis", |ui| {
                        mpxscope::draw(ui, &m.scope);
                    });
                    if ch {
                        m.update(&self.s.mpx);
                    }
                }
                if restart && self.mpx.is_some() {
                    self.pending = true;
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
    let mut s: Saved = std::fs::read_to_string(cfg_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    s.mpx.migrate();
    let outs: Vec<String> = rodio::cpal::default_host().output_devices().map(|d| d.filter_map(|x| x.name().ok()).collect()).unwrap_or_default();
    let ins = srcin::list_inputs();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 900.0])
            .with_min_inner_size([600.0, 480.0])
            .with_title("RadioMoon MPX Transmitter")
            .with_icon(std::sync::Arc::new(look::icon())),
        ..Default::default()
    };
    eframe::run_native(
        "RadioMoon MPX Transmitter",
        options,
        Box::new(|cc| {
            look::apply_style(&cc.egui_ctx);
            Ok(Box::new(App { s, src: None, mpx: None, outs, ins, last_now: String::new(), first: true, pending: false }))
        }),
    )
}

