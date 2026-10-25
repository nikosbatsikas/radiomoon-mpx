//! RadioMoon MPX Receiver, headless edition (Linux, Raspberry Pi, Alpine): receives with its settings file, prints
//! its event log (stream lost / back, lost packets, backup ...) and serves a web interface with a login.
use radiomoon_mpx::rxcore;
use radiomoon_mpx::status::{self, RxStatus};
use radiomoon_mpx::web;
use rodio::cpal::traits::{DeviceTrait, HostTrait};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const PAGE: &str = concat!(include_str!("../web/head.html"), include_str!("../web/rx.js"), include_str!("../web/tail.html"));

struct Rx;

impl web::App for Rx {
    fn page(&self) -> &'static str {
        PAGE
    }
    fn status_json(&self) -> String {
        match status::read::<RxStatus>(&status::path_for(&rxcore::cfg_path()), |s| s.time_ms) {
            Some(s) => serde_json::to_string(&s).unwrap_or_else(|_| "null".into()),
            None => "null".into(),
        }
    }
    fn settings_json(&self) -> String {
        serde_json::to_string(&rxcore::load()).unwrap_or_else(|_| "{}".into())
    }
    fn save_settings(&self, json: &str) -> Result<(), String> {
        let s: rxcore::Saved = serde_json::from_str(json).map_err(|e| format!("settings not valid: {e}"))?;
        let bad = radiomoon_mpx::protect::AllowList::invalid(&s.allow);
        if !bad.is_empty() {
            return Err(format!("Allowed senders: not an address: {}", bad.join(", ")));
        }
        rxcore::save(&s).map_err(|e| format!("cannot save the settings: {e}"))
    }
    fn icon(&self) -> &'static [u8] {
        include_bytes!("../../packaging/receiver.png")
    }
    fn devices_json(&self) -> String {
        let host = rodio::cpal::default_host();
        let mut outputs: Vec<String> = host.output_devices().map(|d| d.filter_map(|x| x.name().ok()).collect()).unwrap_or_default();
        outputs.push(radiomoon_mpx::mpxio::NO_CARD.to_string());
        serde_json::json!({ "outputs": outputs, "inputs": radiomoon_mpx::srcin::list_inputs(), "serial": radiomoon_mpx::clock::serial_ports() }).to_string()
    }
}

fn main() {
    let c = web::cli("radiomoon-mpx-rxd", 8081);
    eprintln!("RadioMoon MPX Receiver {} (headless), settings: {}", env!("CARGO_PKG_VERSION"), rxcore::cfg_path().display());
    let stop = Arc::new(AtomicBool::new(false));
    if c.web {
        web::ensure_login();
        match web::start(&c.listen, Arc::new(Rx), stop.clone()) {
            Ok(()) => eprintln!("Web interface on http://{}", c.listen),
            Err(e) => eprintln!("{e}"),
        }
    }
    // the event log goes to the standard output (OpenRC / systemd keep it)
    rxcore::run(stop, |e| {
        let lvl = ["info", "WARN", "FAULT"][e.level.min(2) as usize];
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), "{} UTC  {lvl:<5} {}", radiomoon_mpx::events::time_text(e.t, 0), e.text);
    });
}
