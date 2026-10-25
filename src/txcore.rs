//! The Transmitter without its window: settings, the source + transmitter chain and the loop that runs it with the
//! saved settings. Used by the window, the service and the headless edition (web interface).
use crate::model::MpxCfg;
use crate::mpx::MpxHandle;
use crate::srcin::{self, Aes67Cfg, SrcCfg, SrcHandle};
use crate::status::{self, TxStatus};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Saved {
    /// "stream" | "input" (audio) | "aes67" (AES67 / Dante over IP) | "mpx" (a ready composite signal from a 192 kHz
    /// sound card input)
    pub kind: String,
    pub aes67: Aes67Cfg,
    pub device: String,
    pub mpx_device: String,
    /// MPX input: 0 = left channel, 1 = right channel
    pub mpx_channel: u8,
    pub url: String,
    pub gain_db: f32,
    pub close_to_tray: bool,
    pub fit_window: bool,
    pub mpx: MpxCfg,
}

impl Default for Saved {
    fn default() -> Self {
        let mpx = MpxCfg { autostart: true, card_on: true, rt_from_playlist: true, ..MpxCfg::default() };
        Saved { kind: "stream".into(), aes67: Aes67Cfg::default(), device: String::new(), mpx_device: String::new(), mpx_channel: 0, url: String::new(), gain_db: 0.0, close_to_tray: true, fit_window: true, mpx }
    }
}

pub fn cfg_path() -> PathBuf {
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    if std::env::var_os("RADIOMOON_DATA").is_none() && exe.join("portable.flag").exists() {
        return exe.join("mpx-transmitter.json");
    }
    status::shared_dir().join("mpx-transmitter.json")
}

/// Makes settings from a file (or the web page) valid.
pub fn fix(s: &mut Saved) {
    s.mpx.migrate();
    if !["input", "mpx", "aes67"].contains(&s.kind.as_str()) {
        s.kind = "stream".into(); // the folder source of older versions is gone
    }
}

pub fn load() -> Saved {
    let mut s: Saved = std::fs::read_to_string(cfg_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    fix(&mut s);
    s
}

pub fn save(s: &Saved) -> std::io::Result<()> {
    let js = serde_json::to_string_pretty(s).map_err(std::io::Error::other)?;
    let p = cfg_path();
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, js)?;
    std::fs::rename(&tmp, &p)
}

fn src_cfg(s: &Saved) -> SrcCfg {
    SrcCfg { kind: s.kind.clone(), device: s.device.clone(), url: s.url.clone(), gain_db: s.gain_db }
}

/// Starts the source and the transmitter: an audio source through the MPX generator, or a ready MPX input as it is.
pub fn start_chain(s: &Saved) -> (SrcHandle, MpxHandle) {
    if s.kind == "mpx" {
        let (h, rx) = srcin::start_mpx(&s.mpx_device, s.mpx_channel as usize, s.gain_db);
        (h, MpxHandle::start_input(&s.mpx, rx))
    } else if s.kind == "aes67" {
        let (h, rx, rate) = srcin::start_aes67(&s.aes67, s.gain_db);
        (h, MpxHandle::start_rate(&s.mpx, rx, rate))
    } else {
        let (h, rx) = srcin::start(&src_cfg(s));
        (h, MpxHandle::start(&s.mpx, rx))
    }
}

/// Settings that need a new chain when they change (levels and RDS are applied while running).
pub fn restart_key(s: &Saved) -> String {
    let m = &s.mpx;
    serde_json::to_string(&(&s.kind, &s.aes67, &s.device, &s.mpx_device, s.mpx_channel, &s.url, s.gain_db, m.card_on, &m.card_device, m.ip_on, &m.targets, m.sfn_on, &m.clock)).unwrap_or_default()
}

pub use crate::status::modified;

/// Runs the transmitter with the saved settings until `stop`, and writes its live status (for the window and the web
/// page). Changed settings are applied: levels and RDS at once, the rest with a new start of the chain.
/// `engine`: "service" | "headless".
pub fn run(stop: Arc<AtomicBool>, engine: &str) {
    let path = cfg_path();
    let st_path = status::path_for(&path);
    let mut s = load();
    let mut key = restart_key(&s);
    let mut stamp = modified(&path);
    let (mut src, mut m) = start_chain(&s);
    let mut last_now = String::new();
    let mut checked = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let now = src.now();
        if !now.is_empty() && now != last_now {
            m.set_radiotext(&s.mpx, &now);
            last_now = now;
        }
        if checked.elapsed() > Duration::from_millis(500) {
            checked = Instant::now();
            let st = modified(&path);
            if st != stamp {
                stamp = st;
                s = load();
                if restart_key(&s) != key {
                    key = restart_key(&s);
                    m.stop();
                    src.stop();
                    std::thread::sleep(Duration::from_millis(300));
                    (src, m) = start_chain(&s);
                    last_now.clear();
                } else {
                    m.update(&s.mpx);
                }
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
            engine: engine.into(),
        };
        status::write(&st_path, &snap);
        std::thread::sleep(Duration::from_millis(250));
    }
    m.stop();
    src.stop();
    status::remove(&st_path);
}

/// The work of the Windows service / systemd service.
pub fn service_work(stop: Arc<AtomicBool>) {
    run(stop, "service");
}
