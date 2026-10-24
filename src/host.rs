//! The window side of the service and the tray icon, shared by both programs: watches the service state, installs /
//! removes / starts / stops it, keeps the tray icon up to date and hides the window to the tray on close.
use crate::look::{self, card};
use crate::service::{self, Kind, State};
use crate::tray::{self, Tray};
use eframe::egui::{self, RichText};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct Host {
    kind: Kind,
    state: Arc<Mutex<State>>,
    busy: Arc<AtomicBool>,
    msg: Arc<Mutex<String>>,
    pub tray: Tray,
    quitting: bool,
}

/// Native window handle on Windows (0 elsewhere).
fn hwnd_of(cc: &eframe::CreationContext) -> isize {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get(),
        _ => 0,
    }
}

impl Host {
    pub fn new(cc: &eframe::CreationContext, kind: Kind, png: &'static [u8]) -> Host {
        let state = Arc::new(Mutex::new(service::query(&kind)));
        // the service state is checked every 2 seconds in the background
        let (st, ctx) = (state.clone(), cc.egui_ctx.clone());
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(2));
            let now = service::query(&kind);
            let changed = st.lock().map(|mut g| std::mem::replace(&mut *g, now) != now).unwrap_or(false);
            if changed {
                ctx.request_repaint();
            }
        });
        let tray = tray::start(kind.display, png, hwnd_of(cc), cc.egui_ctx.clone());
        Host { kind, state, busy: Arc::new(AtomicBool::new(false)), msg: Arc::new(Mutex::new(String::new())), tray, quitting: false }
    }

    pub fn state(&self) -> State {
        self.state.lock().map(|g| *g).unwrap_or_default()
    }

    /// Runs a service action in the background (it may wait for the administrator prompt).
    fn act(&self, what: &'static str, f: fn(&Kind) -> Result<(), String>) {
        if self.busy.swap(true, Ordering::Relaxed) {
            return;
        }
        let (k, st, busy, msg) = (self.kind, self.state.clone(), self.busy.clone(), self.msg.clone());
        if let Ok(mut m) = msg.lock() {
            *m = format!("{what} ...");
        }
        std::thread::spawn(move || {
            let r = f(&k);
            std::thread::sleep(Duration::from_millis(600));
            let now = service::query(&k);
            if let Ok(mut g) = st.lock() {
                *g = now;
            }
            if let Ok(mut m) = msg.lock() {
                *m = match r {
                    Ok(()) => format!("{what}: done"),
                    Err(e) => format!("{what}: {e}"),
                };
            }
            busy.store(false, Ordering::Relaxed);
        });
    }

    pub fn start_service(&self) {
        self.act("Starting the service", service::start);
    }

    pub fn stop_service(&self) {
        self.act("Stopping the service", service::stop);
    }

    fn restart_service(k: &Kind) -> Result<(), String> {
        let _ = service::stop(k);
        std::thread::sleep(Duration::from_millis(1500));
        service::start(k)
    }

    /// Call once per frame: updates the tray, handles "Exit" from the tray and closing to the tray.
    pub fn frame(&mut self, ctx: &egui::Context, close_to_tray: bool, running: bool, tooltip: &str) {
        self.tray.set(running, tooltip);
        if self.tray.exit.load(Ordering::Relaxed) && !self.quitting {
            self.quitting = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && close_to_tray && self.tray.available() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            tray::hide_window(ctx, self.tray.hwnd);
        }
    }

    /// The "Service and tray" settings. Returns true when `close_to_tray` or `fit` was changed.
    pub fn settings_ui(&mut self, ui: &mut egui::Ui, close_to_tray: &mut bool, fit: &mut bool) -> bool {
        let mut ch = false;
        let s = self.state();
        let busy = self.busy.load(Ordering::Relaxed);
        card(ui, "Run as a service", |ui| {
            if !s.supported {
                ui.label(RichText::new("Services are not available on this system.").color(look::DIM));
                return;
            }
            let (color, text) = match (s.installed, s.running) {
                (true, true) => (look::GOOD, "Installed and running"),
                (true, false) => (look::WARN, "Installed, stopped"),
                _ => (look::DIM, "Not installed"),
            };
            ui.horizontal(|ui| {
                look::dot(ui, color);
                ui.label(RichText::new(text).strong());
            });
            ui.label(
                RichText::new(if cfg!(windows) {
                    "As a Windows service the program starts with the computer, before anyone logs in, and keeps running after logoff. It uses the settings of this window; this window shows its status."
                } else {
                    "As a systemd user service the program starts with the computer (with \"linger\") and keeps running after logoff. It uses the settings of this window; this window shows its status."
                })
                .small()
                .color(look::DIM),
            );
            ui.add_space(2.0);
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if !s.installed {
                        if ui.button("Install as service").on_hover_text(if cfg!(windows) { "Asks for administrator rights" } else { "" }).clicked() {
                            self.act("Installing the service", service::install);
                        }
                    } else {
                        if s.running {
                            if ui.button("Stop service").clicked() {
                                self.stop_service();
                            }
                            if ui.button("Restart (apply settings)").clicked() {
                                self.act("Restarting the service", Self::restart_service);
                            }
                        } else if ui.button("Start service").clicked() {
                            self.start_service();
                        }
                        if ui.button("Remove service").clicked() {
                            self.act("Removing the service", service::uninstall);
                        }
                    }
                });
            });
            let m = self.msg.lock().map(|g| g.clone()).unwrap_or_default();
            if !m.is_empty() {
                ui.label(RichText::new(m).small().color(look::DIM));
            }
        });
        card(ui, "System tray", |ui| {
            if self.tray.available() {
                ui.horizontal(|ui| {
                    look::dot(ui, look::GOOD);
                    ui.label("The tray icon shows whether it is running (coloured) or stopped (grey).");
                });
                ch |= ui.checkbox(close_to_tray, "Closing the window keeps the program running in the tray").changed();
                ui.label(RichText::new("Use Exit in the tray icon's menu to end the program.").small().color(look::DIM));
            } else {
                ui.label(RichText::new("No system tray on this desktop.").color(look::DIM));
            }
        });
        card(ui, "Window", |ui| {
            ch |= ui.checkbox(fit, "Fit the window height to its content").changed();
        });
        ch
    }
}

// ---------------------------------------------------------------- SFN reference clock settings (both programs)

/// Settings of the SFN reference clock, with its live state. Returns true when something was changed.
pub fn clock_ui(ui: &mut egui::Ui, c: &mut crate::clock::ClockCfg, state: Option<&crate::clock::ClockState>, ports: &[String]) -> bool {
    let mut ch = false;
    card(ui, "Reference clock", |ui| {
        ui.horizontal_wrapped(|ui| {
            ch |= ui.selectable_value(&mut c.source, "system".to_string(), "System clock").changed();
            ch |= ui.selectable_value(&mut c.source, "ptp".to_string(), "PTP (IEEE 1588v2)").changed();
            ch |= ui.selectable_value(&mut c.source, "gps".to_string(), "GPS receiver").changed();
        });
        let dim = |ui: &mut egui::Ui, t: &str| {
            ui.label(RichText::new(t).small().color(look::DIM));
        };
        match c.source.as_str() {
            "ptp" => {
                egui::Grid::new("ptp").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                    ui.label(RichText::new("Domain").color(look::DIM));
                    ch |= ui.add(egui::DragValue::new(&mut c.ptp_domain).range(0..=127)).changed();
                    ui.end_row();
                    ui.label(RichText::new("Interface address").color(look::DIM));
                    ch |= ui.add(egui::TextEdit::singleline(&mut c.ptp_iface).hint_text("default").desired_width(130.0)).changed();
                    ui.end_row();
                });
                dim(ui, "Built-in PTP client: default profile, multicast 224.0.1.129, end-to-end delay, UDP ports 319 / 320. Software time stamps: about ±10-100 µs, depending on the network and the computer. On Linux the ports need root or CAP_NET_BIND_SERVICE; if ptp4l already runs on this computer, choose \"System clock\" instead (more precise).");
            }
            "gps" => {
                egui::Grid::new("gps").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                    ui.label(RichText::new("Serial port").color(look::DIM));
                    let shown = if c.gps_port.is_empty() { "Choose ...".to_string() } else { c.gps_port.clone() };
                    egui::ComboBox::from_id_salt("gpsport").width(150.0).selected_text(shown).show_ui(ui, |ui| {
                        for p in ports {
                            ch |= ui.selectable_value(&mut c.gps_port, p.clone(), p).changed();
                        }
                    });
                    ui.end_row();
                    ui.label(RichText::new("Port name").color(look::DIM));
                    ch |= ui.add(egui::TextEdit::singleline(&mut c.gps_port).hint_text("COM3 or /dev/ttyACM0").desired_width(150.0)).changed();
                    ui.end_row();
                    ui.label(RichText::new("Baud rate").color(look::DIM));
                    egui::ComboBox::from_id_salt("gpsbaud").width(90.0).selected_text(c.gps_baud.to_string()).show_ui(ui, |ui| {
                        for b in [4800u32, 9600, 19200, 38400, 57600, 115200] {
                            ch |= ui.selectable_value(&mut c.gps_baud, b, b.to_string()).changed();
                        }
                    });
                    ui.end_row();
                });
                ch |= ui.checkbox(&mut c.gps_pps, "PPS (1 pulse per second) on the DCD line of the port").changed();
                if !c.gps_pps {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("NMEA delay").color(look::DIM));
                        ch |= ui.add(egui::DragValue::new(&mut c.gps_delay_ms).range(0.0..=900.0).speed(1.0).suffix(" ms")).on_hover_text("How long after the start of each second the receiver sends its sentences (see its data sheet)").changed();
                    });
                }
                dim(ui, "NMEA only: about ±10 ms, for tests. With PPS on DCD: about ±0.1-1 ms. For real SFN coverage overlap (a few µs) synchronise the system clock to the GPS PPS with gpsd + chrony (Linux) and choose \"System clock\".");
            }
            _ => {
                dim(ui, "The computer's own clock. Use this when the clock is synchronised by PTP (ptp4l + phc2sys on Linux, the PTP client of Windows) or by GPS with PPS (gpsd + chrony): that is the most precise choice. NTP alone (a few ms) is enough only for tests.");
            }
        }
        if let Some(s) = state {
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                look::dot(ui, if s.synced { look::GOOD } else { look::WARN });
                ui.label(&s.text);
            });
            if s.synced && c.source != "system" {
                ui.label(RichText::new(format!("Offset to the system clock {:+.1} µs, spread ±{:.1} µs", s.offset_us, s.jitter_us)).small().color(look::DIM));
            }
        }
    });
    ch
}
