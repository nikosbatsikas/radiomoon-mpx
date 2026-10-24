//! RadioMoon MPX Transmitter - a small stand-alone program: it takes audio from an internet radio stream or a sound
//! card input, builds the FM multiplex (stereo, pilot, RDS) and sends it to a 192 kHz sound card and/or over IP to any
//! number of RadioMoon MPX Receivers. It runs by itself (no RadioMoon needed), in its window or as a service.
//! To put RadioMoon on air, use its internet broadcast as the stream source: the song titles become the RadioText.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use eframe::egui::{self, RichText};
use radiomoon_mpx::host::Host;
use radiomoon_mpx::look::{self, card, inset, pill, Tile};
use radiomoon_mpx::model::{IpTarget, MpxCfg};
use radiomoon_mpx::mpx::{LinkStat, MpxHandle};
use radiomoon_mpx::mpxscope::{self, Scope};
use radiomoon_mpx::service;
use radiomoon_mpx::srcin::{self, SrcCfg, SrcHandle};
use radiomoon_mpx::status::{self, TxStatus};
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Saved {
    /// "stream" | "input" (audio) | "mpx" (a ready composite signal from a 192 kHz sound card input)
    kind: String,
    device: String,
    mpx_device: String,
    /// MPX input: 0 = left channel, 1 = right channel
    mpx_channel: u8,
    url: String,
    gain_db: f32,
    close_to_tray: bool,
    fit_window: bool,
    mpx: MpxCfg,
}

impl Default for Saved {
    fn default() -> Self {
        let mpx = MpxCfg { autostart: true, card_on: true, rt_from_playlist: true, ..MpxCfg::default() };
        Saved { kind: "stream".into(), device: String::new(), mpx_device: String::new(), mpx_channel: 0, url: String::new(), gain_db: 0.0, close_to_tray: true, fit_window: true, mpx }
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

fn load() -> Saved {
    let mut s: Saved = std::fs::read_to_string(cfg_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    s.mpx.migrate();
    if s.kind != "input" && s.kind != "mpx" {
        s.kind = "stream".into(); // the folder source of older versions is gone
    }
    s
}

fn src_cfg(s: &Saved) -> SrcCfg {
    SrcCfg { kind: s.kind.clone(), device: s.device.clone(), url: s.url.clone(), gain_db: s.gain_db }
}

/// Starts the source and the transmitter: an audio source through the MPX generator, or a ready MPX input as it is.
fn start_chain(s: &Saved) -> (SrcHandle, MpxHandle) {
    if s.kind == "mpx" {
        let (h, rx) = srcin::start_mpx(&s.mpx_device, s.mpx_channel as usize, s.gain_db);
        (h, MpxHandle::start_input(&s.mpx, rx))
    } else {
        let (h, rx) = srcin::start(&src_cfg(s));
        (h, MpxHandle::start(&s.mpx, rx))
    }
}

fn modified(p: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

// ---------------------------------------------------------------- service (no window)

/// The work of the service: runs the transmitter with the saved settings and writes its live status for the window.
/// Level and RDS changes saved by the window are applied while running; source and outputs need a restart.
fn service_work(stop: Arc<AtomicBool>) {
    let path = cfg_path();
    let st_path = status::path_for(&path);
    let mut s = load();
    let mut stamp = modified(&path);
    let (src, m) = start_chain(&s);
    let mut last_now = String::new();
    let mut checked = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let now = src.now();
        if !now.is_empty() && now != last_now {
            m.set_radiotext(&s.mpx, &now);
            last_now = now;
        }
        if checked.elapsed() > Duration::from_secs(1) {
            checked = Instant::now();
            let st = modified(&path);
            if st != stamp {
                stamp = st;
                s = load();
                m.update(&s.mpx);
            }
        }
        let snap = TxStatus {
            time_ms: status::now_ms(),
            status: m.status(),
            source: src.status(),
            now: last_now.clone(),
            peak: m.peak(),
            card: m.card_status(),
            links: m.links(),
            scope: m.scope.lock().map(|g| g.clone()).unwrap_or_default(),
            clock: m.clock_state(),
        };
        status::write(&st_path, &snap);
        std::thread::sleep(Duration::from_millis(250));
    }
    m.stop();
    src.stop();
    status::remove(&st_path);
}

// ---------------------------------------------------------------- window

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Source,
    Ip,
    Card,
    Signal,
    Rds,
    Sfn,
    Service,
}

/// What the window shows while running: from the engine in this program or from the service's status file.
struct Live {
    status: String,
    source: String,
    now: String,
    peak: f32,
    card: String,
    links: Vec<(IpTarget, LinkStat)>,
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
    tab: Tab,
    host: Host,
    /// Latest status of the service, and its analyzer data.
    svc: Option<TxStatus>,
    svc_read: Instant,
    svc_scope: Scope,
    about: bool,
    about_tex: Option<egui::TextureHandle>,
    logo_tex: Option<egui::TextureHandle>,
    fit_last: f32,
    ports: Vec<String>,
}

impl App {
    fn start(&mut self) {
        self.stop();
        let (h, m) = start_chain(&self.s);
        self.mpx = Some(m);
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

    /// Small program icon for the header.
    fn logo(&mut self, ctx: &egui::Context) -> egui::TextureId {
        if self.logo_tex.is_none() {
            if let Ok(img) = image::load_from_memory(look::TRANSMITTER_ICON) {
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

    /// The service status file, read 4 times a second while the service runs.
    fn read_service(&mut self) {
        if !self.host.state().running {
            self.svc = None;
            return;
        }
        if self.svc_read.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.svc_read = Instant::now();
        self.svc = status::read::<TxStatus>(&status::path_for(&cfg_path()), |s| s.time_ms);
        if let Some(st) = &self.svc {
            if let Ok(mut g) = self.svc_scope.lock() {
                let max = g.dev_max.max(st.scope.dev_max);
                *g = st.scope.clone();
                g.dev_max = max;
            }
        }
    }

    fn live(&self) -> Option<Live> {
        if let Some(st) = &self.svc {
            return Some(Live { status: st.status.clone(), source: st.source.clone(), now: st.now.clone(), peak: st.peak, card: st.card.clone(), links: st.links.clone() });
        }
        let (src, m) = (self.src.as_ref()?, self.mpx.as_ref()?);
        Some(Live { status: m.status(), source: src.status(), now: src.now(), peak: m.peak(), card: m.card_status(), links: m.links() })
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
    fn header(&mut self, ui: &mut egui::Ui, live: Option<&Live>) {
        let svc = self.host.state();
        ui.horizontal(|ui| {
            ui.add(egui::Image::new((self.logo(ui.ctx()), egui::vec2(20.0, 20.0))));
            ui.label(RichText::new("RadioMoon MPX Transmitter").size(15.0).strong());
            ui.label(RichText::new(format!("v{VERSION}")).small().color(look::DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("About").clicked() {
                    self.about = true;
                }
                if svc.installed {
                    // the service does the work: Start / Stop control the service
                    if svc.running {
                        if look::danger_button(ui, "■  Stop").on_hover_text("Stops the service").clicked() {
                            self.host.stop_service();
                        }
                    } else if look::primary_button(ui, "▶  Start").on_hover_text("Starts the service").clicked() {
                        self.host.start_service();
                    }
                } else if self.mpx.is_none() {
                    if look::primary_button(ui, "▶  Start").clicked() {
                        self.start();
                    }
                } else if look::danger_button(ui, "■  Stop").clicked() {
                    self.stop();
                }
                match live {
                    Some(l) if l.status.starts_with("Waiting") => pill(ui, "NO AUDIO", look::WARN),
                    Some(_) => pill(ui, "ON AIR", look::GOOD),
                    None if svc.running => pill(ui, "STARTING", look::WARN),
                    None => pill(ui, "STOPPED", look::DIM),
                }
                if svc.installed {
                    pill(ui, "SERVICE", look::ACCENT);
                }
            });
        });
    }

    // ------------------------------------------------------------ live overview
    fn overview(&self, ui: &mut egui::Ui, l: &Live) {
        card(ui, "On air", |ui| {
            look::deviation_bar(ui, l.peak);
            ui.add_space(4.0);
            let total: f32 = l.links.iter().map(|(_, x)| x.mbps).sum();
            let up = l.links.iter().filter(|(_, x)| x.ok).count();
            let card_on = !l.card.is_empty();
            look::tiles(
                ui,
                &[
                    Tile::new("Deviation peak", format!("{:.1}", l.peak * 75.0), "kHz", if l.peak > 1.0 { look::BAD } else { look::TEXT }),
                    Tile::new("IP bandwidth", look::mbps(total), "Mbit/s", look::ACCENT),
                    Tile::new("IP outputs", format!("{up} / {}", l.links.len()), "working", if up == l.links.len() { look::GOOD } else { look::WARN }),
                    Tile::new("Sound card", if card_on { "On" } else { "Off" }, "192 kHz", if card_on { look::GOOD } else { look::DIM }),
                ],
            );
            ui.horizontal(|ui| {
                look::dot(ui, look::GOOD);
                ui.label(&l.source);
            });
            if !l.now.is_empty() {
                ui.label(RichText::new(format!("RadioText: {}", l.now)).color(look::DIM));
            }
            if !l.card.is_empty() {
                ui.label(RichText::new(&l.card).small().color(look::DIM));
            }
        });
    }

    // ------------------------------------------------------------ settings tabs
    fn source_card(&mut self, ui: &mut egui::Ui) -> bool {
        let mut restart = false;
        card(ui, "Audio source", |ui| {
            ui.horizontal_wrapped(|ui| {
                restart |= ui.selectable_value(&mut self.s.kind, "stream".to_string(), "Internet stream").changed();
                restart |= ui.selectable_value(&mut self.s.kind, "input".to_string(), "Sound input").changed();
                restart |= ui.selectable_value(&mut self.s.kind, "mpx".to_string(), "MPX input (192 kHz)").changed();
            });
            ui.add_space(2.0);
            match self.s.kind.as_str() {
                "stream" => {
                    restart |= ui.add(egui::TextEdit::singleline(&mut self.s.url).hint_text("http://server:8000/stream").desired_width(ui.available_width())).changed();
                    ui.label(RichText::new("MP3 or AAC over HTTP / HTTPS. The song titles sent by the server become the RadioText.").small().color(look::DIM));
                }
                "mpx" => {
                    let w = (ui.available_width() - 10.0).max(150.0);
                    restart |= combo(ui, "mpxin", &mut self.s.mpx_device, &self.ins, w);
                    ui.horizontal(|ui| {
                        cap(ui, "Channel");
                        restart |= ui.selectable_value(&mut self.s.mpx_channel, 0, "Left").changed();
                        restart |= ui.selectable_value(&mut self.s.mpx_channel, 1, "Right").changed();
                    });
                    ui.label(
                        RichText::new("A ready FM composite signal (e.g. the MPX output of a processor or stereo encoder) from a sound card input that records at 192 kHz. PCM and FLAC outputs send it as it is. Compact MPX outputs decode it (stereo audio, pilot and RDS levels, RDS station name and RadioText) and the receiver rebuilds it. The Signal and RDS settings are not used.")
                            .small()
                            .color(look::DIM),
                    );
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
            ch |= ui.checkbox(&mut c.rt_from_playlist, "RadioText from the song title sent by the stream server").changed();
            ui.horizontal(|ui| {
                ch |= ui.checkbox(&mut c.tp, "TP").changed();
                ch |= ui.checkbox(&mut c.ta, "TA").changed();
                ch |= ui.checkbox(&mut c.music, "Music").changed();
            });
        });
        ch
    }

    /// SFN: time stamps from a reference clock in every UDP packet.
    fn sfn_card(&mut self, ui: &mut egui::Ui) -> bool {
        let mut restart = false;
        let clock = self.mpx.as_ref().and_then(|m| m.clock_state()).or_else(|| self.svc.as_ref().and_then(|s| s.clock.clone()));
        card(ui, "SFN (single frequency network)", |ui| {
            restart |= ui.checkbox(&mut self.s.mpx.sfn_on, "Send SFN time stamps in every UDP packet").changed();
            ui.label(
                RichText::new("Each packet carries the UTC time of its first sample from the reference clock. The MPX Receivers with SFN on play every sample at this time + their delay (the same delay at all sites), so all transmitters of the network are on air in step. Only UDP (RTP) outputs carry the time stamps; receivers without SFN simply ignore them.")
                    .small()
                    .color(look::DIM),
            );
        });
        if self.s.mpx.sfn_on {
            restart |= radiomoon_mpx::host::clock_ui(ui, &mut self.s.mpx.clock, clock.as_ref(), &self.ports);
            if ui.small_button("Refresh the list of serial ports").clicked() {
                self.ports = radiomoon_mpx::clock::serial_ports();
            }
        }
        restart
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
    fn ip_card(&mut self, ui: &mut egui::Ui, links: &[(IpTarget, LinkStat)], running: bool) -> bool {
        let mut restart = false;
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
                        restart |= ui.add(egui::TextEdit::singleline(&mut t.name).hint_text("Name (site)").desired_width(130.0)).changed();
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if n > 1 && ui.small_button("✖").on_hover_text("Remove this receiver").clicked() {
                                remove = Some(i);
                            }
                            if let Some(l) = live {
                                ui.label(RichText::new(format!("{} Mbit/s", look::mbps(l.mbps))).strong().size(12.5).color(look::ACCENT));
                            }
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        cap(ui, "Address");
                        restart |= ui.add(egui::TextEdit::singleline(&mut t.host).hint_text("IP or host name").desired_width(120.0)).changed();
                        cap(ui, "Port");
                        restart |= ui.add(egui::DragValue::new(&mut t.port).range(1..=65535)).changed();
                        egui::ComboBox::from_id_salt(("proto", i)).width(82.0).selected_text(if t.proto == "tcp" { "TCP" } else { "UDP (RTP)" }).show_ui(ui, |ui| {
                            restart |= ui.selectable_value(&mut t.proto, "udp".to_string(), "UDP (RTP)").changed();
                            restart |= ui.selectable_value(&mut t.proto, "tcp".to_string(), "TCP").changed();
                        });
                        let fmt_name = match t.codec.as_str() {
                            "flac" => "FLAC lossless",
                            "cflac" => "Compact · FLAC",
                            "copus" => "Compact · Opus",
                            _ => "PCM uncompressed",
                        };
                        egui::ComboBox::from_id_salt(("codec", i)).width(118.0).selected_text(fmt_name).show_ui(ui, |ui| {
                            ui.label(RichText::new("The composite (192 kHz)").small().color(look::DIM));
                            restart |= ui.selectable_value(&mut t.codec, "flac".to_string(), "FLAC lossless").changed();
                            restart |= ui.selectable_value(&mut t.codec, "l24".to_string(), "PCM uncompressed").changed();
                            ui.separator();
                            ui.label(RichText::new("Compact MPX: audio + RDS, the receiver rebuilds the composite").small().color(look::DIM));
                            restart |= ui.selectable_value(&mut t.codec, "cflac".to_string(), "Compact · FLAC (lossless audio, about 0.9 Mbit/s)").changed();
                            restart |= ui.selectable_value(&mut t.codec, "copus".to_string(), "Compact · Opus (0.15 - 0.35 Mbit/s)").changed();
                        })
                        .response
                        .on_hover_text("Compact MPX needs the RadioMoon MPX Receiver 1.4.0 or newer");
                        if t.codec == "copus" {
                            egui::ComboBox::from_id_salt(("kbps", i)).width(96.0).selected_text(format!("{} kbit/s", t.opus_kbps)).show_ui(ui, |ui| {
                                for (k, note) in [(320u32, "best"), (256, "transparent, recommended"), (192, "very good"), (128, "good, weak links")] {
                                    restart |= ui.selectable_value(&mut t.opus_kbps, k, format!("{k} kbit/s ({note})")).changed();
                                }
                            });
                        } else if !t.is_compact() {
                            let est = |codec: &str, bits: u32| -> f32 {
                                // typical bandwidth incl. headers; FLAC depends on the audio
                                let pcm = 192_000.0 * bits as f32 / 1e6 + 0.32;
                                if codec == "flac" { pcm * 0.8 } else { pcm }
                            };
                            let shown_bits = t.bits();
                            egui::ComboBox::from_id_salt(("bits", i)).width(120.0).selected_text(format!("{shown_bits}-bit, 192 kHz")).show_ui(ui, |ui| {
                                for b in [24u32, 20, 16] {
                                    let txt = format!("{b}-bit (about {:.1} Mbit/s)", est(&t.codec, b));
                                    restart |= ui.selectable_value(&mut t.bits, b, txt).changed();
                                }
                            });
                        }
                        egui::ComboBox::from_id_salt(("dscp", i)).width(80.0).selected_text(format!("QoS {}", t.dscp_name())).show_ui(ui, |ui| {
                            for (v, name) in [(46u8, "EF (46): real-time audio, recommended"), (34, "AF41 (34): AES67 audio"), (40, "CS5 (40)"), (48, "CS6 (48)"), (0, "Off: no marking")] {
                                restart |= ui.selectable_value(&mut t.dscp, v, name).changed();
                            }
                        })
                        .response
                        .on_hover_text("DSCP class written into every packet, so routers and switches with QoS give the MPX priority");
                        restart |= ui.add(egui::DragValue::new(&mut t.dscp).range(0..=63).prefix("DSCP ")).changed();
                    });
                    if let Some(l) = live {
                        let saving = if t.codec != "l24" && l.l24_mbps > 0.0 { format!(" · {:.0}% of L24", l.mbps / l.l24_mbps * 100.0) } else { String::new() };
                        ui.label(RichText::new(format!("{}{saving} · sent {:.1} MB", l.text, l.total_mb)).small().color(if l.ok { look::DIM } else { look::BAD }));
                        look::sparkline(ui, &l.hist, 32.0, look::ACCENT);
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
        ctx.request_repaint_after(Duration::from_millis(250));
        let svc = self.host.state();
        if self.first {
            self.first = false;
            if self.s.mpx.autostart && !svc.installed {
                self.start();
            }
        }
        // with the service installed, the transmitter runs there and not in this window
        if svc.installed && self.mpx.is_some() {
            self.stop();
        }
        self.read_service();
        // RadioText follows the stream's song title
        if let (Some(src), Some(m)) = (&self.src, &self.mpx) {
            let now = src.now();
            if !now.is_empty() && now != self.last_now {
                m.set_radiotext(&self.s.mpx, &now);
                self.last_now = now;
            }
        }
        let live = self.live();
        let tip = match &live {
            Some(l) => format!("{}{} · {:.2} Mbit/s over IP", if svc.running { "Service · " } else { "" }, if l.status.starts_with("Waiting") { "No audio" } else { "On air" }, l.links.iter().map(|(_, x)| x.mbps).sum::<f32>()),
            None => "Stopped".to_string(),
        };
        self.host.frame(ctx, self.s.close_to_tray, live.is_some(), &tip);
        let header_h = egui::TopBottomPanel::top("header")
            .frame(egui::Frame::none().fill(look::CARD).inner_margin(egui::Margin::symmetric(12.0, 6.0)))
            .show(ctx, |ui| {
                self.header(ui, live.as_ref());
            })
            .response
            .rect
            .height();
        look::about_window(
            ctx,
            &mut self.about,
            &mut self.about_tex,
            &look::About {
                name: "RadioMoon MPX Transmitter",
                tagline: "FM multiplex generator with RDS, sent to sound cards and over IP",
                features: &[
                    "Stereo MPX with 19 kHz pilot, RDS and RadioText from the song",
                    "Internet stream, sound input or a ready MPX input (192 kHz) as the source",
                    "Any number of IP receivers: UDP or TCP, L16/L20/L24, FLAC lossless or Compact MPX (FLAC / Opus)",
                    "QoS (DSCP), live bandwidth, SFN time stamps from GPS or PTP (IEEE 1588v2)",
                    "Runs as a Windows / Linux service, system tray icon",
                ],
                png: look::TRANSMITTER_ICON,
                version: VERSION,
            },
        );
        let content_h = egui::CentralPanel::default().frame(egui::Frame::none().fill(look::PANEL).inner_margin(10.0)).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let mut ch = false; // changes applied while running
                let mut restart = false; // changes of the source or the outputs
                if let Some(l) = &live {
                    self.overview(ui, l);
                }
                // ---- tabs
                ui.horizontal_wrapped(|ui| {
                    for (t, name) in [(Tab::Source, "Audio source"), (Tab::Ip, "IP outputs"), (Tab::Card, "Sound card"), (Tab::Signal, "Signal"), (Tab::Rds, "RDS"), (Tab::Sfn, "SFN"), (Tab::Service, "Service & tray")] {
                        let on = self.tab == t;
                        let txt = RichText::new(name).strong().color(if on { look::ACCENT } else { look::TEXT });
                        if ui.add(egui::Button::new(txt).fill(if on { look::CARD } else { look::PANEL }).stroke(egui::Stroke::new(1.0_f32, if on { look::ACCENT } else { look::BORDER }))).clicked() {
                            self.tab = t;
                        }
                    }
                });
                ui.add_space(4.0);
                let links: Vec<(IpTarget, LinkStat)> = live.as_ref().map(|l| l.links.clone()).unwrap_or_default();
                let running = live.is_some();
                match self.tab {
                    Tab::Source => restart |= self.source_card(ui),
                    Tab::Ip => restart |= self.ip_card(ui, &links, running),
                    Tab::Card => restart |= self.card_output(ui),
                    Tab::Signal | Tab::Rds if self.s.kind == "mpx" => {
                        card(ui, "", |ui| {
                            ui.label(RichText::new("The source is an MPX input: the composite signal comes ready from the sound card, so these settings are not used.").color(look::WARN));
                        });
                    }
                    Tab::Signal => ch |= self.signal_card(ui),
                    Tab::Rds => ch |= self.rds_card(ui),
                    Tab::Sfn => restart |= self.sfn_card(ui),
                    Tab::Service => {
                        let (mut c2t, mut fit) = (self.s.close_to_tray, self.s.fit_window);
                        if self.host.settings_ui(ui, &mut c2t, &mut fit) {
                            self.s.close_to_tray = c2t;
                            self.s.fit_window = fit;
                            ch = true;
                        }
                        card(ui, "", |ui| {
                            ch |= ui.checkbox(&mut self.s.mpx.autostart, "Start transmitting when the program starts (without the service)").changed();
                        });
                    }
                }
                if self.pending || (restart && svc.running) {
                    let msg = if svc.running { "Source and output changes are applied when the service is restarted (Service & tray > Restart)." } else { "Source and output changes are applied when you press Stop and Start." };
                    ui.label(RichText::new(msg).color(look::WARN));
                }
                if live.is_some() {
                    card(ui, "Signal analysis", |ui| {
                        if let Some(m) = &self.mpx {
                            mpxscope::draw(ui, &m.scope);
                        } else {
                            mpxscope::draw(ui, &self.svc_scope);
                        }
                    });
                }
                if ch {
                    if let Some(m) = &self.mpx {
                        m.update(&self.s.mpx);
                    }
                }
                if restart && self.mpx.is_some() {
                    self.pending = true;
                }
                if ch || restart {
                    self.persist(); // a running service picks up level and RDS changes from the saved file
                }
            })
            .content_size
            .y
        });
        if self.s.fit_window {
            look::fit_height(ctx, header_h + content_h.inner + 22.0, &mut self.fit_last);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop();
        self.persist();
    }
}

fn main() -> eframe::Result<()> {
    if service::requested() {
        service::run(&service::TRANSMITTER, service_work);
        return Ok(());
    }
    let s = load();
    let outs: Vec<String> = rodio::cpal::default_host().output_devices().map(|d| d.filter_map(|x| x.name().ok()).collect()).unwrap_or_default();
    let ins = srcin::list_inputs();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([860.0, 720.0])
            .with_min_inner_size([520.0, 400.0])
            .with_title("RadioMoon MPX Transmitter")
            .with_icon(std::sync::Arc::new(look::icon(look::TRANSMITTER_ICON))),
        ..Default::default()
    };
    eframe::run_native(
        "RadioMoon MPX Transmitter",
        options,
        Box::new(|cc| {
            look::apply_style(&cc.egui_ctx);
            let host = Host::new(cc, service::TRANSMITTER, look::TRANSMITTER_ICON);
            Ok(Box::new(App {
                s,
                src: None,
                mpx: None,
                outs,
                ins,
                last_now: String::new(),
                first: true,
                pending: false,
                tab: Tab::Source,
                host,
                svc: None,
                svc_read: Instant::now(),
                svc_scope: mpxscope::new_scope(),
                about: false,
                about_tex: None,
                logo_tex: None,
                fit_last: 0.0,
                ports: radiomoon_mpx::clock::serial_ports(),
            }))
        }),
    )
}
