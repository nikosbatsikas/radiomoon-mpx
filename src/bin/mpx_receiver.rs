//! RadioMoon MPX Receiver - a small separate program for the transmitter site.
//! Receives the MPX stream sent by the RadioMoon MPX Transmitter over IP (UDP/RTP or TCP; L16/L20/L24 or FLAC) and
//! plays it on a 192 kHz sound card. It runs in its window or as a service.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use eframe::egui::{self, RichText};
use radiomoon_mpx::host::Host;
use radiomoon_mpx::look::{self, card, pill, tile};
use radiomoon_mpx::mpxrx::{RxCfg, RxHandle, RxView};
use radiomoon_mpx::mpxscope::{self, Scope};
use radiomoon_mpx::service;
use radiomoon_mpx::status::{self, RxStatus};
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
    proto: String,
    port: u16,
    device: String,
    buffer_ms: u32,
    autostart: bool,
    close_to_tray: bool,
}

impl Default for Saved {
    fn default() -> Self {
        Saved { proto: "udp".into(), port: 5004, device: String::new(), buffer_ms: 250, autostart: true, close_to_tray: true }
    }
}

/// The settings live in ProgramData (Windows) so the window and the service (another user account) share them.
fn cfg_path() -> PathBuf {
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    if exe.join("portable.flag").exists() {
        return exe.join("mpx-receiver.json");
    }
    let base = std::env::var_os("PROGRAMDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or(exe);
    let dir = base.join("RadioMoon");
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("mpx-receiver.json");
    // first versions kept the settings in the user's LocalAppData: take them over once
    if !file.exists() {
        if let Some(old) = std::env::var_os("LOCALAPPDATA").map(|o| PathBuf::from(o).join("RadioMoon").join("mpx-receiver.json")) {
            if old.exists() {
                let _ = std::fs::copy(&old, &file);
            }
        }
    }
    file
}

fn load() -> Saved {
    std::fs::read_to_string(cfg_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn rx_cfg(s: &Saved) -> RxCfg {
    RxCfg { proto: s.proto.clone(), port: s.port, device: s.device.clone(), buffer_ms: s.buffer_ms }
}

// ---------------------------------------------------------------- service (no window)

/// The work of the service: receives with the saved settings and writes its live status for the window.
fn service_work(stop: Arc<AtomicBool>) {
    let st_path = status::path_for(&cfg_path());
    let r = RxHandle::start(&rx_cfg(&load()));
    while !stop.load(Ordering::Relaxed) {
        let snap = RxStatus { time_ms: status::now_ms(), status: r.status_text(), peak: r.peak(), view: r.view(), scope: r.scope.lock().map(|g| g.clone()).unwrap_or_default() };
        status::write(&st_path, &snap);
        std::thread::sleep(Duration::from_millis(250));
    }
    r.stop();
    status::remove(&st_path);
}

// ---------------------------------------------------------------- window

struct App {
    s: Saved,
    rx: Option<RxHandle>,
    devs: Vec<String>,
    first: bool,
    host: Host,
    svc: Option<RxStatus>,
    svc_read: Instant,
    svc_scope: Scope,
    about: bool,
    about_tex: Option<egui::TextureHandle>,
    logo_tex: Option<egui::TextureHandle>,
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

    fn settings_cards(&mut self, ui: &mut egui::Ui, svc_running: bool) -> bool {
        let mut ch = false;
        card(ui, "Input", |ui| {
            egui::Grid::new("in").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                cap(ui, "Protocol");
                ui.horizontal(|ui| {
                    ch |= ui.selectable_value(&mut self.s.proto, "udp".to_string(), "UDP").changed();
                    ch |= ui.selectable_value(&mut self.s.proto, "tcp".to_string(), "TCP").changed();
                });
                ui.end_row();
                cap(ui, "Port");
                ch |= ui.add(egui::DragValue::new(&mut self.s.port).range(1..=65535)).changed();
                ui.end_row();
            });
            ui.label(RichText::new("Format detected automatically (L16/L20/L24, FLAC).").small().color(look::DIM));
        });
        card(ui, "Output", |ui| {
            cap(ui, "Sound device (192 kHz)");
            let shown = if self.s.device.is_empty() { "Default".to_string() } else { self.s.device.clone() };
            let w = (ui.available_width() - 6.0).max(120.0);
            egui::ComboBox::from_id_salt("dev").width(w).selected_text(shown).show_ui(ui, |ui| {
                ch |= ui.selectable_value(&mut self.s.device, String::new(), "Default").changed();
                for d in &self.devs {
                    ch |= ui.selectable_value(&mut self.s.device, d.clone(), d).changed();
                }
            });
            ui.horizontal(|ui| {
                cap(ui, "Buffer");
                ch |= ui.add(egui::DragValue::new(&mut self.s.buffer_ms).range(20..=2000).suffix(" ms")).changed();
            });
            ch |= ui.checkbox(&mut self.s.autostart, "Start with the program").changed();
            if self.rx.is_some() || svc_running {
                let msg = if svc_running { "Changes apply after Service > Restart." } else { "Changes apply after Stop and Start." };
                ui.label(RichText::new(msg).small().color(look::DIM));
            }
        });
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
        egui::TopBottomPanel::top("header").frame(egui::Frame::none().fill(look::CARD).inner_margin(egui::Margin::symmetric(12.0, 6.0))).show(ctx, |ui| {
            self.header(ui, live.as_ref());
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
                    "Runs as a Windows / Linux service, system tray icon",
                ],
                png: look::RECEIVER_ICON,
                version: VERSION,
            },
        );
        egui::CentralPanel::default().frame(egui::Frame::none().fill(look::PANEL).inner_margin(10.0)).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let mut ch = false;
                // settings 1/3, link 2/3 of the width (one column on narrow windows)
                let total = ui.available_width();
                if total >= 640.0 {
                    let gap = 8.0;
                    let left = ((total - gap) / 3.0).floor();
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        ui.vertical(|ui| {
                            ui.set_width(left);
                            ch |= self.settings_cards(ui, svc.running);
                        });
                        ui.vertical(|ui| {
                            ui.set_width(total - gap - left);
                            self.link_card(ui, live.as_ref());
                        });
                    });
                } else {
                    self.link_card(ui, live.as_ref());
                    ch |= self.settings_cards(ui, svc.running);
                }
                let mut c2t = self.s.close_to_tray;
                egui::CollapsingHeader::new(RichText::new("Service & tray").strong().color(look::ACCENT)).default_open(svc.installed).show(ui, |ui| {
                    if self.host.settings_ui(ui, &mut c2t) {
                        ch = true;
                    }
                });
                self.s.close_to_tray = c2t;
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
            });
        });
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
                first: true,
                host,
                svc: None,
                svc_read: Instant::now(),
                svc_scope: mpxscope::new_scope(),
                about: false,
                about_tex: None,
                logo_tex: None,
            }))
        }),
    )
}
