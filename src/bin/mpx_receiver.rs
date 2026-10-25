//! RadioMoon MPX Receiver - a small separate program for the transmitter site.
//! Receives the MPX stream sent by the RadioMoon MPX Transmitter over IP (UDP/RTP or TCP; L16/L20/L24 or FLAC) and
//! plays it on a 192 kHz sound card. It runs in its window or as a service.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use eframe::egui::{self, RichText};
use radiomoon_mpx::host::Host;
use radiomoon_mpx::look::{self, card, pill, Tile};
use radiomoon_mpx::mpxrx::{BackupCfg, RxCfg, RxHandle, RxView};
use radiomoon_mpx::mpxscope::{self, Scope};
use radiomoon_mpx::clock::ClockCfg;
use radiomoon_mpx::service;
use radiomoon_mpx::sfn::SfnCfg;
use radiomoon_mpx::status::{self, RxStatus};
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const VERSION: &str = env!("CARGO_PKG_VERSION");

use radiomoon_mpx::rxcore::{cfg_path, load, rx_cfg, service_work, Saved};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Rds,
    Input,
    Output,
    Sfn,
    Log,
    Service,
}

// ---------------------------------------------------------------- window

struct App {
    s: Saved,
    rx: Option<RxHandle>,
    devs: Vec<String>,
    /// Sound inputs (for the backup source).
    ins: Vec<String>,
    first: bool,
    host: Host,
    svc: Option<RxStatus>,
    svc_read: Instant,
    svc_scope: Scope,
    about: bool,
    about_tex: Option<egui::TextureHandle>,
    logo_tex: Option<egui::TextureHandle>,
    fit_last: f32,
    tab: Tab,
    ports: Vec<String>,
}

/// What the window shows: from the receiver in this program or from the service's status file.
struct Live {
    status: String,
    peak: f32,
    v: RxView,
}

impl App {
    fn start(&mut self) {
        self.stop();
        self.rx = Some(RxHandle::start(&rx_cfg(&self.s)));
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

    fn read_service(&mut self) {
        if !self.host.state().running {
            self.svc = None;
            return;
        }
        if self.svc_read.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.svc_read = Instant::now();
        self.svc = status::read::<RxStatus>(&status::path_for(&cfg_path()), |s| s.time_ms);
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
            return Some(Live { status: st.status.clone(), peak: st.peak, v: st.view.clone() });
        }
        let r = self.rx.as_ref()?;
        Some(Live { status: r.status_text(), peak: r.peak(), v: r.view() })
    }
}

fn cap(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(look::DIM));
}

impl App {
    fn header(&mut self, ui: &mut egui::Ui, live: Option<&Live>) {
        let svc = self.host.state();
        ui.horizontal(|ui| {
            ui.add(egui::Image::new((self.logo(ui.ctx()), egui::vec2(20.0, 20.0))));
            ui.label(RichText::new("RadioMoon MPX Receiver").size(15.0).strong());
            ui.label(RichText::new(format!("v{VERSION}")).small().color(look::DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("About").clicked() {
                    self.about = true;
                }
                if svc.installed {
                    if svc.running {
                        if look::danger_button(ui, "■  Stop").on_hover_text("Stops the service").clicked() {
                            self.host.stop_service();
                        }
                    } else if look::primary_button(ui, "▶  Start").on_hover_text("Starts the service").clicked() {
                        self.host.start_service();
                    }
                } else if self.rx.is_none() {
                    if look::primary_button(ui, "▶  Start").clicked() {
                        self.start();
                    }
                } else if look::danger_button(ui, "■  Stop").clicked() {
                    self.stop();
                }
                match live {
                    Some(l) if l.v.receiving => pill(ui, "RECEIVING", look::GOOD),
                    Some(_) => pill(ui, "NO SIGNAL", look::WARN),
                    None if svc.running => pill(ui, "STARTING", look::WARN),
                    None => pill(ui, "STOPPED", look::DIM),
                }
                if svc.installed {
                    pill(ui, "SERVICE", look::ACCENT);
                }
                if live.map(|l| l.v.backup.active).unwrap_or(false) {
                    pill(ui, "ON BACKUP", look::WARN);
                }
                if live.map(|l| l.v.receiving && l.v.protect.encrypted).unwrap_or(false) {
                    pill(ui, "ENCRYPTED", look::GOOD);
                }
                if let Some(s) = live.and_then(|l| l.v.sfn.as_ref()) {
                    pill(ui, if s.locked { "SFN LOCKED" } else { "SFN" }, if s.locked { look::GOOD } else { look::WARN });
                }
            });
        });
    }

    /// Live numbers of the link: bandwidth, format, buffer, losses and the history graph.
    fn link_card(&self, ui: &mut egui::Ui, live: Option<&Live>) {
        card(ui, "Link", |ui| {
            let Some(l) = live else {
                ui.label(RichText::new("Not running. Press Start to receive.").color(look::DIM));
                return;
            };
            let v = &l.v;
            let bits = if v.stats.bits == 0 { 24 } else { v.stats.bits };
            let (fmt, sub) = match v.stats.compact {
                _ if !v.receiving => ("-".to_string(), String::new()),
                1 => ("Compact".to_string(), "FLAC".to_string()),
                2 => ("Compact".to_string(), "Opus".to_string()),
                _ if v.flac => ("FLAC".to_string(), format!("{bits}-bit")),
                _ => (format!("L{bits}"), "192 kHz".to_string()),
            };
            let low = v.buffer_ms < self.s.buffer_ms / 3;
            let lost = v.stats.lost + v.stats.bad;
            look::tiles(
                ui,
                &[
                    Tile::new("Bandwidth", look::mbps(v.mbps), "Mbit/s", look::ACCENT),
                    Tile::new("Format", fmt, sub, look::TEXT),
                    Tile::new("Buffer", format!("{}", v.buffer_ms), "ms", if v.receiving && low { look::WARN } else { look::TEXT }),
                    Tile::new("Lost packets", format!("{lost}"), format!("{:.0} ms hidden", v.stats.concealed as f64 / 192.0), if lost > 0 { look::WARN } else { look::GOOD }),
                    Tile::new("Received", format!("{:.1}", v.total_mb), "MB", look::TEXT),
                ],
            );
            ui.add_space(2.0);
            look::sparkline(ui, &v.hist, 52.0, look::ACCENT);
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                look::dot(ui, if v.receiving { look::GOOD } else { look::WARN });
                ui.label(if v.receiving { format!("From {}", v.from) } else { l.status.clone() });
            });
            if !v.card.is_empty() {
                ui.label(RichText::new(&v.card).small().color(look::DIM));
            }
        });
    }

    /// RDS read from the received signal: station name, RadioText, PI, programme type and flags.
    fn rds_card(&self, ui: &mut egui::Ui, live: Option<&Live>) {
        card(ui, "RDS", |ui| {
            let Some(l) = live else {
                ui.label(RichText::new("Not running.").color(look::DIM));
                return;
            };
            let r = &l.v.rds;
            if !r.synced {
                ui.horizontal(|ui| {
                    look::dot(ui, look::DIM);
                    ui.label(RichText::new(if l.v.receiving { "No RDS in the signal" } else { "No signal" }).color(look::DIM));
                });
                return;
            }
            ui.horizontal(|ui| {
                // the station name as on a radio display
                egui::Frame::none().fill(look::INSET).rounding(5.0).stroke(egui::Stroke::new(1.0_f32, look::BORDER)).inner_margin(egui::Margin::symmetric(10.0, 4.0)).show(ui, |ui| {
                    ui.label(RichText::new(r.ps.as_str()).monospace().size(22.0).strong().color(look::GOOD));
                });
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("PI").small().color(look::DIM));
                        ui.label(RichText::new(r.pi.map(|p| format!("{p:04X}")).unwrap_or_else(|| "-".into())).monospace().strong());
                        if let Some(e) = r.ecc {
                            ui.label(RichText::new("ECC").small().color(look::DIM));
                            ui.label(RichText::new(format!("{e:02X}")).monospace().strong());
                        }
                        ui.label(RichText::new("PTY").small().color(look::DIM));
                        ui.label(r.pty.map(|p| format!("{p} {}", radiomoon_mpx::rdsdec::pty_name(p))).unwrap_or_else(|| "-".into()));
                    });
                    ui.horizontal(|ui| {
                        for (on, name) in [(r.tp, "TP"), (r.ta, "TA"), (r.music, "MUSIC")] {
                            pill(ui, name, if on { look::ACCENT } else { look::BORDER });
                        }
                        ui.label(RichText::new(format!("{:.0}% blocks ok", r.quality * 100.0)).small().color(if r.quality > 0.9 { look::DIM } else { look::WARN }));
                    });
                });
            });
            ui.add_space(2.0);
            if !r.af.is_empty() || !r.ct.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    if !r.af.is_empty() {
                        ui.label(RichText::new("AF").small().color(look::DIM));
                        ui.label(r.af.iter().map(|f| format!("{f:.1}")).collect::<Vec<_>>().join("  "));
                        ui.label(RichText::new("MHz").small().color(look::DIM));
                    }
                    if !r.ct.is_empty() {
                        ui.label(RichText::new("   CT").small().color(look::DIM));
                        ui.label(&r.ct);
                    }
                });
            }
            ui.label(RichText::new("RadioText").small().color(look::DIM));
            egui::Frame::none().fill(look::INSET).rounding(5.0).inner_margin(egui::Margin::symmetric(8.0, 4.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(if r.rt.is_empty() { "-" } else { r.rt.as_str() }).color(look::TEXT));
            });
        });
    }

    /// The event log: stream lost / back, lost packets, refused senders, backup, SFN, sound card.
    fn log_card(&self, ui: &mut egui::Ui, live: Option<&Live>) {
        card(ui, "Log", |ui| {
            let Some(l) = live else {
                ui.label(RichText::new("Not running.").color(look::DIM));
                return;
            };
            let ev = &l.v.events;
            let now = radiomoon_mpx::status::now_ms();
            ui.horizontal(|ui| {
                let faults = ev.iter().filter(|e| e.level == 2).count();
                let warns = ev.iter().filter(|e| e.level == 1).count();
                ui.label(RichText::new(format!("{} events · {faults} faults · {warns} warnings · times in UTC", ev.len())).small().color(look::DIM));
                if ui.small_button("Copy").on_hover_text("Copy the log as text").clicked() {
                    let text: String = ev.iter().map(|e| format!("{} UTC  {}\n", radiomoon_mpx::events::time_text(e.t, 0), e.text)).collect();
                    ui.ctx().copy_text(text);
                }
            });
            egui::ScrollArea::vertical().max_height(320.0).stick_to_bottom(true).show(ui, |ui| {
                egui::Grid::new("log").num_columns(3).spacing([10.0, 3.0]).striped(true).show(ui, |ui| {
                    for e in ev.iter() {
                        let color = match e.level {
                            2 => look::BAD,
                            1 => look::WARN,
                            _ => look::GOOD,
                        };
                        look::dot(ui, color);
                        ui.label(RichText::new(radiomoon_mpx::events::time_text(e.t, 0)).small().monospace().color(look::DIM))
                            .on_hover_text(format!("{} ago", radiomoon_mpx::events::dur(now.saturating_sub(e.t))));
                        ui.label(RichText::new(&e.text).color(if e.level == 2 { look::BAD } else { look::TEXT }));
                        ui.end_row();
                    }
                });
            });
            ui.label(RichText::new("The last 300 events since the Receiver started. Lost packets are summed up every 10 seconds.").small().color(look::DIM));
        });
    }

    fn input_card(&mut self, ui: &mut egui::Ui, live: Option<&Live>) -> bool {
        let mut ch = false;
        card(ui, "Input (from the transmitter)", |ui| {
            egui::Grid::new("in").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
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
            ui.label(RichText::new("The format is detected automatically: L16 / L20 / L24, FLAC or Compact MPX.").small().color(look::DIM));
        });
        card(ui, "Protection", |ui| {
            egui::Grid::new("prot").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                cap(ui, "Password");
                ui.horizontal(|ui| {
                    let id = egui::Id::new("rxpwshow");
                    let show = ui.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
                    ch |= ui.add(egui::TextEdit::singleline(&mut self.s.password).password(!show).hint_text("none").desired_width(150.0)).changed();
                    if !self.s.password.is_empty() && ui.small_button(if show { "Hide" } else { "Show" }).clicked() {
                        ui.data_mut(|d| d.insert_temp(id, !show));
                    }
                });
                ui.end_row();
                cap(ui, "Allowed senders");
                ch |= ui.add(egui::TextEdit::singleline(&mut self.s.allow).hint_text("everyone, or e.g. 192.168.1.10, 10.0.0.0/8").desired_width(ui.available_width().min(260.0))).changed();
                ui.end_row();
            });
            let bad = radiomoon_mpx::protect::AllowList::invalid(&self.s.allow);
            if !bad.is_empty() {
                ui.label(RichText::new(format!("Not an address: {}", bad.join(", "))).small().color(look::WARN));
            }
            ui.label(
                RichText::new(if self.s.password.is_empty() {
                    "Without a password only unencrypted streams play. With a password only the stream encrypted with the same password plays: nobody else can listen or send a false stream. FEC (set in the Transmitter) is used automatically."
                } else {
                    "Only the stream encrypted with this password plays; unencrypted or other streams are ignored. FEC (set in the Transmitter) is used automatically."
                })
                .small()
                .color(look::DIM),
            );
            if let Some(l) = live {
                let p = &l.v.protect;
                ui.add_space(2.0);
                let mut lines: Vec<(egui::Color32, String)> = Vec::new();
                if l.v.receiving {
                    lines.push(if p.encrypted { (look::GOOD, "Encrypted, password correct".to_string()) } else { (look::DIM, "Not encrypted".to_string()) });
                    if p.fec_n > 0 {
                        let burst = if p.fec_depth > 1 { format!(", bursts of {}", p.fec_depth) } else { String::new() };
                        lines.push((if p.lost > 0 { look::WARN } else { look::GOOD }, format!("FEC 1 per {}{burst}: restored {} packets, could not restore {}", p.fec_n, p.restored, p.lost)));
                    }
                }
                if p.need_password {
                    lines.push((look::WARN, "An encrypted stream arrives: enter its password".into()));
                }
                if p.rejected > 0 {
                    lines.push((look::BAD, format!("{} packets rejected: wrong password, or changed on the way", p.rejected)));
                }
                if p.plain_rejected > 0 {
                    lines.push((look::WARN, format!("{} unencrypted packets ignored (a password is set)", p.plain_rejected)));
                }
                if p.blocked > 0 {
                    lines.push((look::WARN, format!("{} packets / connections blocked, last from {}", p.blocked, p.blocked_from)));
                }
                for (c, t) in lines {
                    ui.horizontal_wrapped(|ui| {
                        look::dot(ui, c);
                        ui.label(t);
                    });
                }
            }
            if self.rx.is_some() {
                ui.label(RichText::new("Changes apply after Stop and Start (or Service > Restart).").small().color(look::DIM));
            }
        });
        card(ui, "Backup source (MPX from a sound card input)", |ui| {
            let b = &mut self.s.backup;
            ch |= ui.checkbox(&mut b.on, "When the stream stops, put the MPX of a sound input on air").changed();
            ui.label(
                RichText::new("For an analog backup: an STL / off-air receiver, a local processor or a second line with a ready composite (MPX) signal, recorded at 192 kHz. It is switched back to the stream when the stream has come back without a break.")
                    .small()
                    .color(look::DIM),
            );
            ui.add_enabled_ui(b.on, |ui| {
                egui::Grid::new("bk").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                    cap(ui, "Sound input (192 kHz)");
                    let shown = if b.device.is_empty() { "Default".to_string() } else { b.device.clone() };
                    egui::ComboBox::from_id_salt("bkdev").width(230.0).selected_text(shown).show_ui(ui, |ui| {
                        ch |= ui.selectable_value(&mut b.device, String::new(), "Default").changed();
                        for d in &self.ins {
                            ch |= ui.selectable_value(&mut b.device, d.clone(), d).changed();
                        }
                    });
                    ui.end_row();
                    cap(ui, "Channel");
                    ui.horizontal(|ui| {
                        ch |= ui.selectable_value(&mut b.channel, 0, "Left").changed();
                        ch |= ui.selectable_value(&mut b.channel, 1, "Right").changed();
                    });
                    ui.end_row();
                    cap(ui, "Level");
                    ch |= ui.add(egui::DragValue::new(&mut b.gain_db).range(-24.0..=24.0).speed(0.1).fixed_decimals(1).suffix(" dB")).on_hover_text("Match the deviation of the backup to the stream").changed();
                    ui.end_row();
                    cap(ui, "Switch to backup after");
                    ch |= ui.add(egui::DragValue::new(&mut b.timeout_ms).range(100..=30_000).speed(10.0).suffix(" ms without stream")).changed();
                    ui.end_row();
                    cap(ui, "Back to the stream after");
                    ch |= ui.add(egui::DragValue::new(&mut b.return_s).range(0.0..=600.0).speed(0.5).fixed_decimals(0).suffix(" s of steady stream")).changed();
                    ui.end_row();
                });
                ch |= ui.checkbox(&mut b.on_silence, "Also when the stream carries silence (MPX below -40 dB)").changed();
            });
            if let Some(l) = live.filter(|l| l.v.backup.enabled) {
                let bv = &l.v.backup;
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    if bv.active {
                        look::dot(ui, look::WARN);
                        let back = if l.v.receiving { format!(" - the stream is back, on air in {:.0} s", bv.return_in_s) } else { String::new() };
                        ui.label(RichText::new(format!("BACKUP ON AIR{back}")).strong().color(look::WARN));
                    } else {
                        look::dot(ui, look::GOOD);
                        ui.label("Stream on air, backup ready");
                    }
                    ui.label(RichText::new(format!("· input level {:.0}% · switched {} times", bv.level * 100.0, bv.switches)).small().color(look::DIM));
                });
                if !bv.input.is_empty() {
                    ui.label(RichText::new(&bv.input).small().color(look::DIM));
                }
            }
        });
        ch
    }

    fn output_card(&mut self, ui: &mut egui::Ui, svc_running: bool) -> bool {
        let mut ch = false;
        card(ui, "Output (exciter MPX input)", |ui| {
            cap(ui, "Sound device (192 kHz)");
            let shown = if self.s.device.is_empty() { "Default".to_string() } else { self.s.device.clone() };
            let w = (ui.available_width() - 6.0).clamp(160.0, 420.0);
            egui::ComboBox::from_id_salt("dev").width(w).selected_text(shown).show_ui(ui, |ui| {
                ch |= ui.selectable_value(&mut self.s.device, String::new(), "Default").changed();
                for d in &self.devs {
                    ch |= ui.selectable_value(&mut self.s.device, d.clone(), d).changed();
                }
            });
            ui.horizontal(|ui| {
                cap(ui, "Buffer");
                ui.add_enabled_ui(!self.s.sfn_on, |ui| {
                    ch |= ui.add(egui::DragValue::new(&mut self.s.buffer_ms).range(20..=2000).suffix(" ms")).changed();
                });
                if self.s.sfn_on {
                    ui.label(RichText::new("(SFN: the delay of the SFN tab is used)").small().color(look::DIM));
                }
            });
            ch |= ui.checkbox(&mut self.s.autostart, "Start receiving when the program starts").changed();
            if self.rx.is_some() || svc_running {
                let msg = if svc_running { "Changes apply after Service > Restart." } else { "Changes apply after Stop and Start." };
                ui.label(RichText::new(msg).small().color(look::DIM));
            }
        });
        ch
    }

    /// SFN: playout at the time stamps by the reference clock.
    fn sfn_card(&mut self, ui: &mut egui::Ui, live: Option<&Live>, svc_running: bool) -> bool {
        let mut ch = false;
        let st = live.and_then(|l| l.v.sfn.clone());
        let clock = live.and_then(|l| l.v.clock.clone());
        card(ui, "SFN (single frequency network)", |ui| {
            ch |= ui.checkbox(&mut self.s.sfn_on, "Play every sample at its time stamp + delay").changed();
            ui.label(
                RichText::new("For several transmitters on the same frequency: every site plays the same sample at the same moment, by a common reference clock (GPS or PTP). The Transmitter must send SFN time stamps (its SFN tab), over UDP.")
                    .small()
                    .color(look::DIM),
            );
            if self.s.proto != "udp" && self.s.sfn_on {
                ui.label(RichText::new("SFN needs UDP: TCP carries no time stamps (Input tab).").color(look::WARN));
            }
            if self.s.sfn_on {
                egui::Grid::new("sfn").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                    cap(ui, "Delay (all sites)");
                    ch |= ui
                        .add(egui::DragValue::new(&mut self.s.sfn_delay_ms).range(50.0..=5000.0).speed(5.0).suffix(" ms"))
                        .on_hover_text("From the time stamp to the moment on air. The same value at every site; longer than the slowest network path to any site (see Arrival margin).")
                        .changed();
                    ui.end_row();
                    cap(ui, "Output correction");
                    ch |= ui
                        .add(egui::DragValue::new(&mut self.s.sfn_correction_us).range(-100_000.0..=100_000.0).speed(1.0).suffix(" µs"))
                        .on_hover_text("This site's own delay of sound card, exciter and cables, measured against the reference (e.g. with an oscilloscope and the PPS): positive values play earlier.")
                        .changed();
                    ui.end_row();
                });
            }
            if let Some(s) = &st {
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    look::dot(ui, if s.locked { look::GOOD } else { look::WARN });
                    ui.label(RichText::new(&s.state).strong());
                });
                look::tiles(
                    ui,
                    &[
                        Tile::new("Timing error", format!("{:+.1}", s.error_us), "µs", if s.locked { look::GOOD } else { look::WARN }),
                        Tile::new("Rate trim", format!("{:+.2}", s.ppm), "ppm", look::TEXT),
                        Tile::new("Arrival margin", if s.margin_ms.is_finite() && s.margin_ms < 1e6 { format!("{:.0}", s.margin_ms) } else { "-".into() }, "ms", if s.margin_ms < 20.0 { look::WARN } else { look::TEXT }),
                        Tile::new("Stamp jitter", format!("±{:.1}", s.stamp_jitter_us), "µs", look::TEXT),
                        Tile::new("Restarts", s.resyncs.to_string(), "", look::TEXT),
                    ],
                );
            }
        });
        if self.s.sfn_on {
            ch |= radiomoon_mpx::host::clock_ui(ui, &mut self.s.clock, clock.as_ref(), &self.ports);
            if ui.small_button("Refresh the list of serial ports").clicked() {
                self.ports = radiomoon_mpx::clock::serial_ports();
            }
        }
        if ch && (self.rx.is_some() || svc_running) {
            ui.label(RichText::new(if svc_running { "SFN changes apply after Service > Restart." } else { "SFN changes apply after Stop and Start." }).small().color(look::WARN));
        }
        ch
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _f: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(250));
        let svc = self.host.state();
        if self.first {
            self.first = false;
            if self.s.autostart && !svc.installed {
                self.start();
            }
        }
        // with the service installed, the receiver runs there and not in this window
        if svc.installed && self.rx.is_some() {
            self.stop();
        }
        self.read_service();
        let live = self.live();
        let tip = match &live {
            Some(l) => format!("{}{} · {:.2} Mbit/s", if svc.running { "Service · " } else { "" }, if l.v.receiving { "Receiving" } else { "No signal" }, l.v.mbps),
            None => "Stopped".to_string(),
        };
        self.host.frame(ctx, self.s.close_to_tray, live.as_ref().map(|l| l.v.receiving).unwrap_or(false), &tip);
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
                name: "RadioMoon MPX Receiver",
                tagline: "Receives the MPX over IP and plays it into the exciter",
                features: &[
                    "UDP (RTP) or TCP, format detected automatically",
                    "L16 / L20 / L24, FLAC 16 / 20 / 24-bit and Compact MPX (FLAC / Opus)",
                    "Lost packets hidden without a pilot or RDS phase jump",
                    "RDS display: station name, RadioText, PI, PTY, TP / TA",
                    "SFN playout with GPS or PTP (IEEE 1588v2) time",
                    "Runs as a Windows / Linux service, system tray icon",
                ],
                png: look::RECEIVER_ICON,
                version: VERSION,
            },
        );
        let content_h = egui::CentralPanel::default().frame(egui::Frame::none().fill(look::PANEL).inner_margin(10.0)).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let mut ch = false;
                // the link on top, the settings in tabs below
                self.link_card(ui, live.as_ref());
                ui.horizontal_wrapped(|ui| {
                    for (t, name) in [(Tab::Rds, "RDS"), (Tab::Input, "Input"), (Tab::Output, "Output"), (Tab::Sfn, "SFN"), (Tab::Log, "Log"), (Tab::Service, "Service, tray & window")] {
                        let on = self.tab == t;
                        let txt = RichText::new(name).strong().color(if on { look::ACCENT } else { look::TEXT });
                        if ui.add(egui::Button::new(txt).fill(if on { look::CARD } else { look::PANEL }).stroke(egui::Stroke::new(1.0_f32, if on { look::ACCENT } else { look::BORDER }))).clicked() {
                            self.tab = t;
                        }
                    }
                });
                ui.add_space(4.0);
                match self.tab {
                    Tab::Rds => self.rds_card(ui, live.as_ref()),
                    Tab::Input => ch |= self.input_card(ui, live.as_ref()),
                    Tab::Output => ch |= self.output_card(ui, svc.running),
                    Tab::Sfn => ch |= self.sfn_card(ui, live.as_ref(), svc.running),
                    Tab::Log => self.log_card(ui, live.as_ref()),
                    Tab::Service => {
                        let (mut c2t, mut fit) = (self.s.close_to_tray, self.s.fit_window);
                        if self.host.settings_ui(ui, &mut c2t, &mut fit) {
                            self.s.close_to_tray = c2t;
                            self.s.fit_window = fit;
                            ch = true;
                        }
                    }
                }
                if let Some(l) = &live {
                    card(ui, "Signal analysis", |ui| {
                        look::deviation_bar(ui, l.peak);
                        ui.add_space(4.0);
                        match &self.rx {
                            Some(r) => mpxscope::draw(ui, &r.scope),
                            None => mpxscope::draw(ui, &self.svc_scope),
                        }
                    });
                }
                if ch {
                    self.persist();
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
    }
}

fn main() -> eframe::Result<()> {
    if service::requested() {
        service::run(&service::RECEIVER, service_work);
        return Ok(());
    }
    let s = load();
    let devs: Vec<String> = rodio::cpal::default_host().output_devices().map(|d| d.filter_map(|x| x.name().ok()).collect()).unwrap_or_default();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([820.0, 680.0])
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
            let host = Host::new(cc, service::RECEIVER, look::RECEIVER_ICON);
            Ok(Box::new(App {
                s,
                rx: None,
                devs,
                ins: radiomoon_mpx::srcin::list_inputs(),
                first: true,
                host,
                svc: None,
                svc_read: Instant::now(),
                svc_scope: mpxscope::new_scope(),
                about: false,
                about_tex: None,
                logo_tex: None,
                fit_last: 0.0,
                tab: Tab::Rds,
                ports: radiomoon_mpx::clock::serial_ports(),
            }))
        }),
    )
}
