//! The Receiver without its window: settings and the loop that runs it with the saved settings. Used by the
//! window, the service and the headless edition (web interface).
use crate::clock::ClockCfg;
use crate::mpxrx::{BackupCfg, RxCfg, RxHandle};
use crate::sfn::SfnCfg;
use crate::status::{self, RxStatus};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Saved {
    pub proto: String,
    pub port: u16,
    pub device: String,
    pub buffer_ms: u32,
    pub autostart: bool,
    pub close_to_tray: bool,
    pub fit_window: bool,
    /// SFN: play every sample at its time stamp + delay (UDP only).
    pub sfn_on: bool,
    pub sfn_delay_ms: f64,
    pub sfn_correction_us: f64,
    pub clock: ClockCfg,
    /// Protection: password of an encrypted stream (empty = unencrypted streams only).
    pub password: String,
    /// Allowed senders (addresses / networks; empty = everyone).
    pub allow: String,
    /// Backup MPX from a sound card input, on air while the stream is missing.
    pub backup: BackupCfg,
}

impl Default for Saved {
    fn default() -> Self {
        Saved {
            proto: "udp".into(),
            port: 5004,
            device: String::new(),
            buffer_ms: 250,
            autostart: true,
            close_to_tray: true,
            fit_window: true,
            sfn_on: false,
            sfn_delay_ms: 500.0,
            sfn_correction_us: 0.0,
            clock: ClockCfg::default(),
            password: String::new(),
            allow: String::new(),
            backup: BackupCfg::default(),
        }
    }
}

/// The settings live in ProgramData (Windows) so the window and the service (another user account) share them.
pub fn cfg_path() -> PathBuf {
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    if std::env::var_os("RADIOMOON_DATA").is_none() && exe.join("portable.flag").exists() {
        return exe.join("mpx-receiver.json");
    }
    let file = status::shared_dir().join("mpx-receiver.json");
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

pub fn load() -> Saved {
    std::fs::read_to_string(cfg_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(s: &Saved) -> std::io::Result<()> {
    let js = serde_json::to_string_pretty(s).map_err(std::io::Error::other)?;
    let p = cfg_path();
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, js)?;
    std::fs::rename(&tmp, &p)
}

pub fn rx_cfg(s: &Saved) -> RxCfg {
    let sfn = (s.sfn_on && s.proto == "udp").then(|| (SfnCfg { delay_ms: s.sfn_delay_ms, correction_us: s.sfn_correction_us }, s.clock.clone()));
    RxCfg { proto: s.proto.clone(), port: s.port, device: s.device.clone(), buffer_ms: s.buffer_ms, sfn, password: s.password.clone(), allow: s.allow.clone(), backup: s.backup.clone() }
}

/// Runs the receiver with the saved settings until `stop` and writes its live status. Changed settings start it anew
/// (the event log is kept). `on_event`: called with every new log line (the headless edition prints them).
pub fn run(stop: Arc<AtomicBool>, mut on_event: impl FnMut(&crate::events::Event)) {
    let path = cfg_path();
    let st_path = status::path_for(&path);
    let mut s = load();
    let mut stamp = status::modified(&path);
    let mut r = RxHandle::start(&rx_cfg(&s));
    let mut checked = Instant::now();
    let mut kept: std::collections::VecDeque<crate::events::Event> = Default::default();
    let mut last_printed: Option<crate::events::Event> = None;
    while !stop.load(Ordering::Relaxed) {
        if checked.elapsed() > Duration::from_millis(500) {
            checked = Instant::now();
            let st = status::modified(&path);
            if st != stamp {
                stamp = st;
                let n = load();
                if n != s {
                    s = n;
                    // the log of the old run goes on in the new one
                    kept = r.view().events;
                    r.stop();
                    std::thread::sleep(Duration::from_millis(400));
                    crate::events::push(&mut kept, crate::events::Event { t: status::now_ms(), level: 0, text: "Settings changed: restarted".into() });
                    r = RxHandle::start(&rx_cfg(&s));
                }
            }
        }
        let mut view = r.view();
        if !kept.is_empty() {
            let mut all = kept.clone();
            for e in view.events.iter() {
                crate::events::push(&mut all, e.clone());
            }
            view.events = all;
        }
        // new lines for the system log
        let start = last_printed.as_ref().and_then(|lp| view.events.iter().rposition(|e| e == lp)).map(|i| i + 1).unwrap_or(0);
        for e in view.events.iter().skip(start) {
            on_event(e);
        }
        if let Some(last) = view.events.back() {
            last_printed = Some(last.clone());
        }
        let snap = RxStatus { time_ms: status::now_ms(), status: r.status_text(), peak: r.peak(), view, scope: r.scope.lock().map(|g| g.clone()).unwrap_or_default() };
        status::write(&st_path, &snap);
        std::thread::sleep(Duration::from_millis(250));
    }
    r.stop();
    status::remove(&st_path);
}

/// The work of the Windows service / systemd service.
pub fn service_work(stop: Arc<AtomicBool>) {
    run(stop, |_| {});
}
