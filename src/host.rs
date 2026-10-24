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
