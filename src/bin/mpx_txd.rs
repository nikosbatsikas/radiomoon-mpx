//! RadioMoon MPX Transmitter, headless edition (Linux, Raspberry Pi, Alpine): runs the transmitter with its settings
//! file and serves a web interface (settings, live status) with a login. Same settings as the window program.
use radiomoon_mpx::status::{self, TxStatus};
use radiomoon_mpx::txcore;
use radiomoon_mpx::web;
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const PAGE: &str = concat!(include_str!("../web/head.html"), include_str!("../web/tx.js"), include_str!("../web/tail.html"));

struct Tx;

impl web::App for Tx {
    fn page(&self) -> &'static str {
        PAGE
    }
    fn status_json(&self) -> String {
        match status::read::<TxStatus>(&status::path_for(&txcore::cfg_path()), |s| s.time_ms) {
            Some(s) => serde_json::to_string(&s).unwrap_or_else(|_| "null".into()),
            None => "null".into(),
        }
    }
    fn settings_json(&self) -> String {
        serde_json::to_string(&txcore::load()).unwrap_or_else(|_| "{}".into())
    }
    fn save_settings(&self, json: &str) -> Result<(), String> {
        let mut s: txcore::Saved = serde_json::from_str(json).map_err(|e| format!("settings not valid: {e}"))?;
        txcore::fix(&mut s);
        for t in &s.mpx.targets {
            if t.enabled && t.host.trim().is_empty() {
                return Err(format!("\"{}\": enter the address of the receiver", t.label()));
            }
        }
        txcore::save(&s).map_err(|e| format!("cannot save the settings: {e}"))
    }
    fn devices_json(&self) -> String {
        let host = rodio::cpal::default_host();
        let outputs: Vec<String> = host.output_devices().map(|d| d.filter_map(|x| x.name().ok()).collect()).unwrap_or_default();
        serde_json::json!({ "outputs": outputs, "inputs": radiomoon_mpx::srcin::list_inputs(), "serial": radiomoon_mpx::clock::serial_ports() }).to_string()
    }
}

fn main() {
    let c = web::cli("radiomoon-mpx-txd", 8080);
    eprintln!("RadioMoon MPX Transmitter {} (headless), settings: {}", env!("CARGO_PKG_VERSION"), txcore::cfg_path().display());
    let stop = Arc::new(AtomicBool::new(false));
    if c.web {
        web::ensure_login();
        match web::start(&c.listen, Arc::new(Tx), stop.clone()) {
            Ok(()) => eprintln!("Web interface on http://{}", c.listen),
            Err(e) => eprintln!("{e}"),
        }
    }
    txcore::run(stop, "headless");
}
