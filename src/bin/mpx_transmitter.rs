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
use radiomoon_mpx::srcin::{self, Aes67Cfg, Aes67Finder, SrcCfg, SrcHandle};
use radiomoon_mpx::status::{self, TxStatus};
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const VERSION: &str = env!("CARGO_PKG_VERSION");

use radiomoon_mpx::txcore::{cfg_path, load, service_work, start_chain, Saved};

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
    /// Listens for AES67 stream announcements while the AES67 source is shown.
    finder: Option<Aes67Finder>,
    /// The AF list while it is being typed: sent only when finished (Enter or leaving the field), so radios
    /// never store a half-typed frequency ("95" while typing "95.7").
    af_edit: Option<String>,
    /// Opened by the CLAP plug-in (`--plugin`) as its window: no own transmitting, no source / service tabs; the
    /// value is the host program's name (or empty).
    plugin_window: Option<String>,
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
    /// The status of a transmitter running elsewhere: the service, or the CLAP plug-in in an audio program.
    fn read_service(&mut self) {
        if self.mpx.is_some() {
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
                let in_plugin = self.plugin_window.is_some() || self.svc.as_ref().map(|s| s.engine == "plugin").unwrap_or(false);
                if self.plugin_window.is_some() && live.is_none() {
                    ui.label(RichText::new("starts when the audio program plays").small().color(look::DIM));
                } else if in_plugin {
                    // the CLAP plug-in in an audio program does the work: it starts and stops with the program
                    ui.label(RichText::new("runs in the CLAP plug-in").small().color(look::DIM));
                } else if svc.installed {
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
                if in_plugin {
                    pill(ui, "PLUG-IN", look::ACCENT);
                } else if svc.installed {
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
                restart |= ui.selectable_value(&mut self.s.kind, "aes67".to_string(), "AES67 / Dante").changed();
                restart |= ui.selectable_value(&mut self.s.kind, "mpx".to_string(), "MPX input (192 kHz)").changed();
            });
            ui.add_space(2.0);
            match self.s.kind.as_str() {
                "stream" => {
                    restart |= ui.add(egui::TextEdit::singleline(&mut self.s.url).hint_text("http://server:8000/stream").desired_width(ui.available_width())).changed();
                    ui.label(RichText::new("MP3 or AAC over HTTP / HTTPS. The song titles sent by the server become the RadioText.").small().color(look::DIM));
                }
                "aes67" => restart |= self.aes67_ui(ui),
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

    /// AES67 source: announced streams (SAP) and the stream settings.
    fn aes67_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let mut ch = false;
        if self.finder.is_none() {
            self.finder = Some(Aes67Finder::start(&self.s.aes67.iface));
        }
        let found = self.finder.as_ref().map(|f| f.streams()).unwrap_or_default();
        let a = &mut self.s.aes67;
        ui.horizontal(|ui| {
            cap(ui, "Announced streams");
            let shown = if found.is_empty() { "None heard yet".to_string() } else { format!("{} found - choose ...", found.len()) };
            egui::ComboBox::from_id_salt("aesfound").width(260.0).selected_text(shown).show_ui(ui, |ui| {
                for f in &found {
                    let label = format!("{} - {}:{} {} {} Hz {} ch", if f.name.is_empty() { "(no name)" } else { &f.name }, f.group, f.port, f.encoding, f.rate, f.channels);
                    if ui.selectable_label(a.group == f.group && a.port == f.port, label).clicked() {
                        let iface = a.iface.clone();
                        *a = f.clone();
                        a.iface = iface;
                        ch = true;
                    }
                }
            });
        });
        egui::Grid::new("aes").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
            cap(ui, "Multicast address");
            ui.horizontal(|ui| {
                ch |= ui.add(egui::TextEdit::singleline(&mut a.group).hint_text("239.69.x.x (empty = unicast)").desired_width(150.0)).changed();
                cap(ui, "Port");
                ch |= ui.add(egui::DragValue::new(&mut a.port).range(1..=65535)).changed();
            });
            ui.end_row();
            cap(ui, "Format");
            ui.horizontal(|ui| {
                ch |= ui.selectable_value(&mut a.encoding, "L24".to_string(), "L24").changed();
                ch |= ui.selectable_value(&mut a.encoding, "L16".to_string(), "L16").changed();
                for r in [44_100u32, 48_000, 96_000] {
                    ch |= ui.selectable_value(&mut a.rate, r, format!("{} kHz", r as f32 / 1000.0)).changed();
                }
            });
            ui.end_row();
            cap(ui, "Channels");
            ui.horizontal(|ui| {
                ch |= ui.add(egui::DragValue::new(&mut a.channels).range(1..=64).suffix(" in the stream")).changed();
                let n = a.channels.max(1);
                cap(ui, "Left");
                ch |= ui.add(egui::DragValue::new(&mut a.ch_l).range(0..=n - 1).custom_formatter(|v, _| format!("{}", v as u32 + 1))).changed();
                cap(ui, "Right");
                ch |= ui.add(egui::DragValue::new(&mut a.ch_r).range(0..=n - 1).custom_formatter(|v, _| format!("{}", v as u32 + 1))).changed();
            });
            ui.end_row();
            cap(ui, "Interface address");
            ch |= ui.add(egui::TextEdit::singleline(&mut a.iface).hint_text("default").desired_width(150.0)).changed();
            ui.end_row();
        });
        ui.label(
            RichText::new("AES67: RTP L24 / L16, usually 48 kHz, multicast. Streams that announce themselves (SAP) appear in the list. Dante: switch the Dante device to AES67 mode in Dante Controller (Device Config > AES67 Config), set a multicast address and create a multicast transmit flow; it then appears here. Dante Virtual Soundcard also works through \"Sound input\". The sender's PTP clock is not needed here: the MPX follows the stream's own rate.")
                .small()
                .color(look::DIM),
        );
        ch
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
                cap(ui, "PI code (hex)");
                ui.horizontal(|ui| {
                    ch |= ui.add(egui::TextEdit::singleline(&mut c.pi).desired_width(60.0).char_limit(4)).changed();
                    cap(ui, "ECC");
                    ch |= ui
                        .add(egui::TextEdit::singleline(&mut c.ecc).desired_width(34.0).char_limit(2).hint_text("E1"))
                        .on_hover_text("Extended Country Code (group 1A), hex: with the first digit of the PI it names the country world-wide. Greece E1 (PI 1xxx), Cyprus E2 (PI 2xxx). Empty: not sent. Cars with hybrid radio (RadioDNS) need it to find the station's logo.")
                        .changed();
                    if !c.ecc.trim().is_empty() && radiomoon_mpx::mpxdsp::parse_ecc(&c.ecc).is_none() {
                        ui.label(RichText::new("not valid (A0 - F4)").small().color(look::WARN));
                    }
                    cap(ui, "PTY");
                    ch |= ui.add(egui::DragValue::new(&mut c.pty).range(0..=31)).changed();
                    ui.label(RichText::new(radiomoon_mpx::rdsdec::pty_name(c.pty)).small().color(look::DIM));
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
        card(ui, "Station name (PS)", |ui| {
            let c = &mut self.s.mpx;
            ui.horizontal_wrapped(|ui| {
                ch |= ui.selectable_value(&mut c.ps_mode, "static".to_string(), "Fixed (8 characters)").changed();
                ch |= ui.selectable_value(&mut c.ps_mode, "pages".to_string(), "Long text in pages").changed();
                ch |= ui.selectable_value(&mut c.ps_mode, "scroll".to_string(), "Long text scrolling").changed();
            });
            let long = c.ps_mode != "static";
            ch |= ui
                .add(egui::TextEdit::singleline(&mut c.ps).hint_text(if long { "RADIO MOON FM - THE BEST MUSIC" } else { "RADIOMON" }).desired_width(ui.available_width()).char_limit(if long { 128 } else { 8 }))
                .changed();
            if long {
                ui.horizontal(|ui| {
                    cap(ui, if c.ps_mode == "scroll" { "Seconds per character" } else { "Seconds per page" });
                    ch |= ui.add(egui::DragValue::new(&mut c.ps_interval).range(0.3..=30.0).speed(0.1).fixed_decimals(1).suffix(" s")).changed();
                });
                // preview of what radios show
                let pages = radiomoon_mpx::mpxdsp::PsPager::pages(&c.ps, &c.ps_mode);
                let shown = if c.ps_mode == "scroll" { format!("{} steps", pages.len()) } else { format!("{} pages", pages.len()) };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(shown).small().color(look::DIM));
                    for p in pages.iter().take(if c.ps_mode == "scroll" { 6 } else { 12 }) {
                        egui::Frame::none().fill(look::INSET).rounding(4.0).inner_margin(egui::Margin::symmetric(5.0, 1.0)).show(ui, |ui| {
                            ui.label(RichText::new(p.as_str()).monospace().color(look::GOOD));
                        });
                    }
                });
                ui.label(
                    RichText::new("A complete station name is always sent before the next page, so radios never show a mix. Note: the RDS standard asks for a fixed PS; some car radios also use it to store the station, so keep the pages slow (3 s or more).")
                        .small()
                        .color(look::DIM),
                );
            }
        });
        card(ui, "Alternative frequencies (AF)", |ui| {
            let c = &mut self.s.mpx;
            let edit = self.af_edit.get_or_insert_with(|| c.af.clone());
            let r = ui.add(egui::TextEdit::singleline(edit).hint_text("e.g. 94.3, 101.5, 105.0").desired_width(ui.available_width()));
            if r.lost_focus() {
                // written back tidy: "957 1015" becomes "95.7, 101.5"
                let codes = radiomoon_mpx::mpxdsp::parse_af(edit);
                let tidy = codes.iter().map(|&k| format!("{:.1}", radiomoon_mpx::mpxdsp::af_mhz(k))).collect::<Vec<_>>().join(", ");
                let tidy = if codes.is_empty() { edit.trim().to_string() } else { tidy };
                if tidy != c.af {
                    c.af = tidy;
                    ch = true;
                }
                self.af_edit = None;
            } else if !r.has_focus() {
                // not being edited: follow the settings
                self.af_edit = None;
            } else if self.af_edit.as_deref() != Some(c.af.as_str()) {
                ui.label(RichText::new("Press Enter (or click elsewhere) to send the new list.").small().color(look::WARN));
            }
            let codes = radiomoon_mpx::mpxdsp::parse_af(&c.af);
            let list: Vec<String> = codes.iter().map(|&k| format!("{:.1}", radiomoon_mpx::mpxdsp::af_mhz(k))).collect();
            ui.label(
                RichText::new(if list.is_empty() { "No AF list is sent. Enter the other frequencies of this programme (87.6 - 107.9 MHz), so radios can switch to the strongest one.".to_string() } else { format!("Sent: {} MHz ({} of up to 25)", list.join(", "), list.len()) })
                    .small()
                    .color(look::DIM),
            );
        });
        card(ui, "Clock time (CT)", |ui| {
            let c = &mut self.s.mpx;
            ch |= ui.checkbox(&mut c.ct_on, "Send the time and date once a minute (radios set their clocks)").changed();
            ui.add_enabled_ui(c.ct_on, |ui| {
                ui.horizontal(|ui| {
                    cap(ui, "Local time offset");
                    ch |= ui.add(egui::DragValue::new(&mut c.ct_offset_h).range(-12.0..=14.0).speed(0.5).fixed_decimals(1).prefix("UTC ")).on_hover_text("The standard (winter) offset when daylight saving time is automatic").changed();
                    ui.label(RichText::new(if c.ct_dst == "off" { "Greece: +2 in winter, +3 in summer" } else { "standard (winter) time; Greece: +2" }).small().color(look::DIM));
                });
                ui.horizontal(|ui| {
                    cap(ui, "Daylight saving");
                    let name = |k: &str| match k {
                        "eu" => "Automatic (Europe)",
                        "us" => "Automatic (USA / Canada)",
                        _ => "Off (fixed offset)",
                    };
                    egui::ComboBox::from_id_salt("ctdst").width(170.0).selected_text(name(&c.ct_dst)).show_ui(ui, |ui| {
                        for k in ["eu", "us", "off"] {
                            ch |= ui.selectable_value(&mut c.ct_dst, k.to_string(), name(k)).changed();
                        }
                    });
                });
                // what is sent now
                let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
                let d = radiomoon_mpx::mpx::rds_of(c, "");
                let off = radiomoon_mpx::mpxdsp::ct_offset_now(&d, secs);
                let summer = off != d.ct_offset;
                let local = secs + off as i64 * 1800;
                ui.label(
                    RichText::new(format!(
                        "Sent now: {:02}:{:02}, UTC{:+}{} {}",
                        local.rem_euclid(86_400) / 3600,
                        local.rem_euclid(3600) / 60,
                        off / 2,
                        if off % 2 != 0 { ":30" } else { "" },
                        if c.ct_dst == "off" { "" } else if summer { "(summer time)" } else { "(winter time)" }
                    ))
                    .small()
                    .color(look::DIM),
                );
            });
            ui.label(RichText::new("The time comes from this computer's clock (UTC); keep it synchronised.").small().color(look::DIM));
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
            restart |= radiomoon_mpx::host::clock_ui(ui, &mut self.s.mpx.clock, clock.as_ref(), &self.ports, false);
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
                    // protection: password (encryption), FEC (UDP)
                    ui.horizontal_wrapped(|ui| {
                        cap(ui, "Password");
                        let id = egui::Id::new(("pwshow", i));
                        let show = ui.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
                        restart |= ui
                            .add(egui::TextEdit::singleline(&mut t.password).password(!show).hint_text("none: not encrypted").desired_width(120.0))
                            .on_hover_text("Encrypts the stream (ChaCha20-Poly1305). The receiver needs the same password; others cannot play it or send a false stream to it.")
                            .changed();
                        if !t.password.is_empty() && ui.small_button(if show { "Hide" } else { "Show" }).clicked() {
                            ui.data_mut(|d| d.insert_temp(id, !show));
                        }
                        if t.proto != "tcp" {
                            egui::ComboBox::from_id_salt(("fec", i)).width(150.0).selected_text(if t.fec == 0 { "FEC off".to_string() } else { format!("FEC {}", radiomoon_mpx::protect::fec_name(t.fec)) }).show_ui(ui, |ui| {
                                ui.label(RichText::new("Forward error correction: extra packets that restore lost ones").small().color(look::DIM));
                                for n in radiomoon_mpx::protect::FEC_CHOICES {
                                    let note = match n {
                                        0 => "",
                                        20 => " - good networks",
                                        10 => " - internet, recommended",
                                        _ => " - lossy links (radio, mobile)",
                                    };
                                    restart |= ui.selectable_value(&mut t.fec, n, format!("{}{note}", radiomoon_mpx::protect::fec_name(n))).changed();
                                }
                            })
                            .response
                            .on_hover_text("One FEC packet for every n packets restores one lost packet of them. Needs the Receiver 1.10.0 or newer.");
                            if t.fec > 0 {
                                let shown = |d: u8| if d <= 1 { "single losses".to_string() } else { format!("bursts of {d}") };
                                egui::ComboBox::from_id_salt(("burst", i)).width(110.0).selected_text(shown(t.fec_burst)).show_ui(ui, |ui| {
                                    for d in radiomoon_mpx::protect::DEPTH_CHOICES {
                                        restart |= ui.selectable_value(&mut t.fec_burst, d, format!("Restores {}", shown(d))).changed();
                                    }
                                })
                                .response
                                .on_hover_text("Burst protection: also restores this many packets lost in a row (interleaved FEC), at the cost of more delay");
                                ui.label(RichText::new(format!("+{:.0} ms delay", t.fec_delay_ms())).small().color(look::DIM));
                            }
                        }
                    });
                    if let Some(l) = live {
                        let saving = if t.codec != "l24" && l.l24_mbps > 0.0 { format!(" · {:.0}% of L24", l.mbps / l.l24_mbps * 100.0) } else { String::new() };
                        ui.label(RichText::new(format!("{}{saving} · sent {:.1} MB", l.text, l.total_mb)).small().color(if l.ok { look::DIM } else { look::BAD }));
                        if !l.time.is_empty() {
                            ui.label(RichText::new(&l.time).small().color(if l.time.contains("follows") { look::GOOD } else { look::DIM }));
                        }
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
            let plugin = status::read::<TxStatus>(&status::path_for(&cfg_path()), |s| s.time_ms).map(|s| s.engine == "plugin").unwrap_or(false);
            if self.s.mpx.autostart && !svc.installed && !plugin && self.plugin_window.is_none() {
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
        self.host.frame(ctx, self.s.close_to_tray && self.plugin_window.is_none(), live.is_some(), &tip);
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
                    "Internet stream, sound input, AES67 / Dante or a ready MPX input (192 kHz) as the source",
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
                    let tabs: &[(Tab, &str)] = if self.plugin_window.is_some() {
                        &[(Tab::Source, "Level"), (Tab::Ip, "IP outputs"), (Tab::Card, "Sound card"), (Tab::Signal, "Signal"), (Tab::Rds, "RDS"), (Tab::Sfn, "SFN")]
                    } else {
                        &[(Tab::Source, "Audio source"), (Tab::Ip, "IP outputs"), (Tab::Card, "Sound card"), (Tab::Signal, "Signal"), (Tab::Rds, "RDS"), (Tab::Sfn, "SFN"), (Tab::Service, "Service & tray")]
                    };
                    for &(t, name) in tabs {
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
                    Tab::Source if self.plugin_window.is_some() => {
                        let host = self.plugin_window.clone().unwrap_or_default();
                        card(ui, "Audio from the host program", |ui| {
                            ui.label(RichText::new(format!("The plug-in takes the audio of its track{}: the audio source of the Transmitter is not used.", if host.is_empty() { String::new() } else { format!(" in {host}") })).color(look::DIM));
                            ui.horizontal(|ui| {
                                ui.label("Level");
                                ch |= ui.add(egui::Slider::new(&mut self.s.gain_db, -24.0..=12.0).suffix(" dB")).changed();
                            });
                        });
                    }
                    Tab::Service if self.plugin_window.is_some() => {}
                    Tab::Source => restart |= self.source_card(ui),
                    Tab::Ip => restart |= self.ip_card(ui, &links, running),
                    Tab::Card => restart |= self.card_output(ui),
                    Tab::Signal | Tab::Rds if self.s.kind == "mpx" && self.plugin_window.is_none() => {
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
                if self.plugin_window.is_none() && (self.pending || (restart && svc.running)) {
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
    // opened by the CLAP plug-in as its window: `--plugin [host name]`
    let args: Vec<String> = std::env::args().collect();
    let plugin_window = args.iter().position(|a| a == "--plugin").map(|i| args.get(i + 1).filter(|a| !a.starts_with("--")).cloned().unwrap_or_default());
    let title = match &plugin_window {
        Some(h) if !h.is_empty() => format!("RadioMoon MPX Transmitter - plug-in in {h}"),
        Some(_) => "RadioMoon MPX Transmitter - plug-in".to_string(),
        None => "RadioMoon MPX Transmitter".to_string(),
    };
    let s = load();
    let outs: Vec<String> = rodio::cpal::default_host().output_devices().map(|d| d.filter_map(|x| x.name().ok()).collect()).unwrap_or_default();
    let ins = srcin::list_inputs();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([860.0, 720.0])
            .with_min_inner_size([520.0, 400.0])
            .with_title(title.clone())
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
                // RM_TAB opens another tab first (used for the screenshots of the wiki)
                tab: match std::env::var("RM_TAB").unwrap_or_default().as_str() {
                    "ip" => Tab::Ip,
                    "card" => Tab::Card,
                    "signal" => Tab::Signal,
                    "rds" => Tab::Rds,
                    "sfn" => Tab::Sfn,
                    "service" => Tab::Service,
                    _ if plugin_window.is_some() => Tab::Ip,
                    _ => Tab::Source,
                },
                host,
                svc: None,
                svc_read: Instant::now(),
                svc_scope: mpxscope::new_scope(),
                about: false,
                about_tex: None,
                logo_tex: None,
                fit_last: 0.0,
                ports: radiomoon_mpx::clock::serial_ports(),
                finder: None,
                af_edit: None,
                plugin_window,
            }))
        }),
    )
}
