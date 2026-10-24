//! MPX receiver (transmitter site): IP -> 192 kHz sound card. Used by the separate receiver program.
//! L24 and FLAC are detected automatically (RTP payload type over UDP, the stream start over TCP).
use crate::mpxflac::{Depacketizer, RxStats, TcpDeframer};
use crate::mpxio::{card_thread, get, set, track_peak, Queue, Text};
use std::io::Read;
use std::net::{TcpListener, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct RxCfg {
    pub proto: String,
    pub port: u16,
    pub device: String,
    pub buffer_ms: u32,
}

pub struct RxHandle {
    stop: Arc<AtomicBool>,
    status: Text,
    peak: Arc<AtomicU32>,
    pub scope: crate::mpxscope::Scope,
}

impl RxHandle {
    pub fn start(cfg: &RxCfg) -> RxHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let status: Text = Arc::new(Mutex::new("Starting...".into()));
        let peak = Arc::new(AtomicU32::new(0));
        let q = Queue::new(cfg.buffer_ms.clamp(20, 2000));
        let card: Text = Arc::new(Mutex::new(String::new()));
        card_thread(cfg.device.clone(), q.clone(), stop.clone(), card.clone(), "Sound card");
        let scope = crate::mpxscope::new_scope();
        let scope2 = scope.clone();
        let (st, stt, pk, port, tcp) = (stop.clone(), status.clone(), peak.clone(), cfg.port, cfg.proto == "tcp");
        std::thread::spawn(move || {
            let mut samples: Vec<f32> = Vec::with_capacity(8192);
            let mut an = crate::mpxscope::Analyzer::new();
            let mut shown = Instant::now();
            let mut bytes: u64 = 0;
            let show = |stt: &Text, from: &str, st: &RxStats, rate: f64, q: &Queue, card: &Text| {
                let fmt = if st.flac { "FLAC lossless" } else { "L24" };
                let mut t = format!("Receiving {fmt} from {from}, {rate:.2} Mbit/s, buffer {} ms", q.ms());
                if st.lost > 0 || st.bad > 0 {
                    t += &format!("; lost packets {} (restored {}, concealed {:.0} ms)", st.lost + st.bad, st.recovered, st.concealed as f64 / 192.0);
                }
                set(stt, &format!("{t}; {}", get(card)));
            };
            if tcp {
                let l = match TcpListener::bind(("0.0.0.0", port)) {
                    Ok(l) => l,
                    Err(e) => {
                        set(&stt, &format!("Cannot listen on port {port}: {e}"));
                        return;
                    }
                };
                let _ = l.set_nonblocking(true);
                'accept: while !st.load(Ordering::Relaxed) {
                    set(&stt, &format!("Waiting for a sender on TCP port {port} ..."));
                    let (mut s, who) = loop {
                        if st.load(Ordering::Relaxed) {
                            break 'accept;
                        }
                        match l.accept() {
                            Ok(x) => break x,
                            Err(_) => std::thread::sleep(Duration::from_millis(100)),
                        }
                    };
                    let _ = s.set_nonblocking(false);
                    let _ = s.set_read_timeout(Some(Duration::from_millis(300)));
                    let mut buf = vec![0u8; 16384];
                    let mut de = TcpDeframer::new(); // a new connection may use the other format
                    while !st.load(Ordering::Relaxed) {
                        match s.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                bytes += n as u64;
                                samples.clear();
                                de.feed(&buf[..n], &mut samples);
                                if !samples.is_empty() {
                                    track_peak(&samples, &pk);
                                    an.push(&samples, &scope2);
                                    q.push(&samples);
                                }
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
                            Err(_) => break,
                        }
                        if shown.elapsed() > Duration::from_millis(500) {
                            let rate = bytes as f64 * 8.0 / shown.elapsed().as_secs_f64() / 1e6;
                            shown = Instant::now();
                            bytes = 0;
                            show(&stt, &who.to_string(), &de.stats, rate, &q, &card);
                        }
                    }
                }
            } else {
                let sock = match UdpSocket::bind(("0.0.0.0", port)) {
                    Ok(s) => s,
                    Err(e) => {
                        set(&stt, &format!("Cannot listen on UDP port {port}: {e}"));
                        return;
                    }
                };
                let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
                let mut buf = vec![0u8; 65536];
                let mut from = String::from("-");
                let mut de = Depacketizer::new();
                while !st.load(Ordering::Relaxed) {
                    match sock.recv_from(&mut buf) {
                        Ok((n, a)) => {
                            samples.clear();
                            if de.packet(&buf[..n], &mut samples) {
                                from = a.to_string();
                                bytes += n as u64;
                                if !samples.is_empty() {
                                    track_peak(&samples, &pk);
                                    an.push(&samples, &scope2);
                                    q.push(&samples);
                                }
                            }
                        }
                        Err(_) => {
                            if shown.elapsed() > Duration::from_secs(2) && q.len() == 0 {
                                set(&stt, &format!("Waiting for packets on UDP port {port} ..."));
                            }
                        }
                    }
                    if shown.elapsed() > Duration::from_millis(500) && q.len() > 0 {
                        let rate = bytes as f64 * 8.0 / shown.elapsed().as_secs_f64() / 1e6;
                        shown = Instant::now();
                        bytes = 0;
                        show(&stt, &from, &de.stats, rate, &q, &card);
                    }
                }
            }
        });
        RxHandle { stop, status, peak, scope }
    }

    pub fn status_text(&self) -> String {
        get(&self.status)
    }

    pub fn peak(&self) -> f32 {
        f32::from_bits(self.peak.load(Ordering::Relaxed))
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

