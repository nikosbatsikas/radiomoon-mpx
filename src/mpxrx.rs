//! MPX receiver (transmitter site): IP -> 192 kHz sound card. Used by the separate receiver program.
//! L24 and FLAC are detected automatically (RTP payload type over UDP, the stream start over TCP).
use crate::mpx::push_hist;
use crate::compact::{CompactRx, CompactTcp, TCP_COMPACT_MAGIC};
use crate::mpxflac::{Depacketizer, RxStats, TcpDeframer};
use crate::clock::RefClock;
use crate::mpxio::{card_thread, get, set, sfn_card_thread, track_peak, Queue, Text};
use crate::rdsdec::RdsDecoder;
use crate::clock::{ClockCfg, ClockState};
use crate::sfn::{SfnCfg, SfnPlayout, SfnStatus};
use std::collections::VecDeque;
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
    /// SFN playout (UDP only): plays every sample at its time stamp + delay by the reference clock.
    pub sfn: Option<(SfnCfg, ClockCfg)>,
}

/// Where the received samples go: the normal buffer, or the SFN playout.
enum Sink {
    Plain(Arc<Queue>),
    Sfn(Arc<SfnPlayout>),
}

impl Sink {
    fn push(&self, v: &[f32], stamp: Option<(usize, i64)>) {
        match self {
            Sink::Plain(q) => q.push(v),
            Sink::Sfn(p) => p.push(v, stamp),
        }
    }
    fn ms(&self) -> u32 {
        match self {
            Sink::Plain(q) => q.ms(),
            Sink::Sfn(p) => p.ms(),
        }
    }
}

/// Serialisation of the packet statistics (kept free of serde in mpxflac.rs).
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(remote = "RxStats")]
struct RxStatsDef {
    packets: u64,
    lost: u64,
    recovered: u64,
    concealed: u64,
    bad: u64,
    late: u64,
    flac: bool,
    bits: u32,
    compact: u8,
}

/// RDS decoded from the received signal.
#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RdsView {
    pub synced: bool,
    pub pi: Option<u16>,
    pub ps: String,
    pub rt: String,
    pub pty: Option<u8>,
    pub tp: bool,
    pub ta: bool,
    pub music: bool,
    /// Share of correct blocks, 0..1.
    pub quality: f32,
    pub groups: u64,
    pub af: Vec<f32>,
    pub ct: String,
}

impl RdsView {
    fn from(d: &RdsDecoder, receiving: bool) -> RdsView {
        let i = &d.info;
        RdsView { synced: i.synced && receiving, pi: i.pi, ps: i.ps.clone(), rt: i.rt.clone(), pty: i.pty, tp: i.tp, ta: i.ta, music: i.music, quality: i.quality, groups: i.groups, af: i.af.clone(), ct: i.ct.clone() }
    }
}

/// Live numbers of the receiver (read by the window).
#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RxView {
    /// Data arrived in the last half second.
    pub receiving: bool,
    /// Address of the sender.
    pub from: String,
    pub flac: bool,
    /// Bandwidth on the network now, Mbit/s (including IP/UDP or TCP headers).
    pub mbps: f32,
    /// Received since start, megabytes.
    pub total_mb: f64,
    /// Audio waiting in the buffer, ms.
    pub buffer_ms: u32,
    #[serde(with = "RxStatsDef")]
    pub stats: RxStats,
    /// State of the sound card output.
    pub card: String,
    /// Bandwidth history (oldest first, 2 values per second).
    pub hist: VecDeque<f32>,
    /// RDS decoded from the received MPX.
    pub rds: RdsView,
    /// SFN playout and its reference clock (when SFN is on).
    pub sfn: Option<SfnStatus>,
    pub clock: Option<ClockState>,
}

pub struct RxHandle {
    stop: Arc<AtomicBool>,
    status: Text,
    peak: Arc<AtomicU32>,
    view: Arc<Mutex<RxView>>,
    pub scope: crate::mpxscope::Scope,
}

/// Collects the received bytes and publishes the numbers twice a second.
struct Meter {
    bytes: u64,
    total: u64,
    at: Instant,
}

impl Meter {
    fn new() -> Self {
        Meter { bytes: 0, total: 0, at: Instant::now() }
    }
    fn add(&mut self, wire_bytes: u64) {
        self.bytes += wire_bytes;
        self.total += wire_bytes;
    }
    /// Returns true when the numbers were published.
    fn tick(&mut self, view: &Mutex<RxView>, from: &str, st: &RxStats, q: &Sink, card: &Text, rds: &RdsDecoder) -> bool {
        if self.at.elapsed() < Duration::from_millis(500) {
            return false;
        }
        let secs = self.at.elapsed().as_secs_f64();
        let mbps = (self.bytes as f64 * 8.0 / secs / 1e6) as f32;
        if let Ok(mut v) = view.lock() {
            v.receiving = self.bytes > 0;
            v.from = from.to_string();
            v.flac = st.flac;
            v.mbps = mbps;
            v.total_mb = self.total as f64 / 1e6;
            v.buffer_ms = q.ms();
            if let Sink::Sfn(p) = q {
                v.sfn = p.status.lock().ok().map(|g| {
                    let mut st = g.clone();
                    st.stamp_jitter_us = p.stamp_jitter_us();
                    st
                });
                v.clock = Some(p.clock.state());
            }
            v.stats = *st;
            v.card = get(card);
            v.rds = RdsView::from(rds, self.bytes > 0);
            push_hist(&mut v.hist, mbps);
        }
        self.bytes = 0;
        self.at = Instant::now();
        true
    }
}

/// A TCP stream: the format is known after its first 4 bytes.
enum TcpIn {
    Unknown(Vec<u8>),
    Compact(CompactTcp),
    Other(TcpDeframer),
}

impl TcpIn {
    fn feed(&mut self, data: &[u8], out: &mut Vec<f32>) {
        if let TcpIn::Unknown(b) = self {
            b.extend_from_slice(data);
            if b.len() < 4 {
                return;
            }
            let first = std::mem::take(b);
            if &first[..4] == TCP_COMPACT_MAGIC {
                let mut c = CompactTcp::new();
                c.feed(&first[4..], out);
                *self = TcpIn::Compact(c);
            } else {
                let mut d = TcpDeframer::new();
                d.feed(&first, out);
                *self = TcpIn::Other(d);
            }
            return;
        }
        match self {
            TcpIn::Compact(c) => c.feed(data, out),
            TcpIn::Other(d) => d.feed(data, out),
            TcpIn::Unknown(_) => {}
        }
    }
    fn stats(&self) -> RxStats {
        match self {
            TcpIn::Compact(c) => c.stats,
            TcpIn::Other(d) => d.stats,
            TcpIn::Unknown(_) => RxStats::default(),
        }
    }
}

/// Name of the received format, e.g. "FLAC 24-bit", "L16", "Compact MPX (Opus)".
pub fn format_name(st: &RxStats) -> String {
    let bits = if st.bits == 0 { 24 } else { st.bits };
    match st.compact {
        1 => "Compact MPX (FLAC)".into(),
        2 => "Compact MPX (Opus)".into(),
        _ if st.flac => format!("FLAC {bits}-bit"),
        _ => format!("L{bits}"),
    }
}

fn status_line(v: &RxView) -> String {
    let fmt = format_name(&v.stats);
    let mut t = format!("Receiving {fmt} from {}, {:.2} Mbit/s, buffer {} ms", v.from, v.mbps, v.buffer_ms);
    if v.stats.lost > 0 || v.stats.bad > 0 {
        t += &format!("; lost packets {} (restored {}, concealed {:.0} ms)", v.stats.lost + v.stats.bad, v.stats.recovered, v.stats.concealed as f64 / 192.0);
    }
    t
}

impl RxHandle {
    pub fn start(cfg: &RxCfg) -> RxHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let status: Text = Arc::new(Mutex::new("Starting...".into()));
        let peak = Arc::new(AtomicU32::new(0));
        let view = Arc::new(Mutex::new(RxView::default()));
        let card: Text = Arc::new(Mutex::new(String::new()));
        let q = match (&cfg.sfn, cfg.proto == "tcp") {
            (Some((sc, cc)), false) => {
                let pl = SfnPlayout::new(sc.clone(), RefClock::start(cc));
                sfn_card_thread(cfg.device.clone(), pl.clone(), stop.clone(), card.clone(), "Sound card");
                Sink::Sfn(pl)
            }
            _ => {
                let q = Queue::new(cfg.buffer_ms.clamp(20, 2000));
                card_thread(cfg.device.clone(), q.clone(), stop.clone(), card.clone(), "Sound card");
                Sink::Plain(q)
            }
        };
        let scope = crate::mpxscope::new_scope();
        let scope2 = scope.clone();
        let (st, stt, pk, vw, port, tcp) = (stop.clone(), status.clone(), peak.clone(), view.clone(), cfg.port, cfg.proto == "tcp");
        std::thread::spawn(move || {
            let mut samples: Vec<f32> = Vec::with_capacity(8192);
            let mut an = crate::mpxscope::Analyzer::new();
            let mut meter = Meter::new();
            let mut rds = RdsDecoder::new(192_000);
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
                            Err(_) => {
                                meter.tick(&vw, "-", &RxStats::default(), &q, &card, &rds);
                                std::thread::sleep(Duration::from_millis(100));
                            }
                        }
                    };
                    let _ = s.set_nonblocking(false);
                    let _ = s.set_read_timeout(Some(Duration::from_millis(300)));
                    let mut buf = vec![0u8; 16384];
                    let mut de = TcpIn::Unknown(Vec::new()); // a new connection may use another format
                    let who = who.to_string();
                    while !st.load(Ordering::Relaxed) {
                        match s.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                meter.add(n as u64 + (n as u64 / 1448 + 1) * 40);
                                samples.clear();
                                de.feed(&buf[..n], &mut samples);
                                if !samples.is_empty() {
                                    track_peak(&samples, &pk);
                                    an.push(&samples, &scope2);
                                    rds.push(&samples);
                                    q.push(&samples, None); // TCP carries no SFN time stamps
                                }
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
                            Err(_) => break,
                        }
                        if meter.tick(&vw, &who, &de.stats(), &q, &card, &rds) {
                            if let Ok(v) = vw.lock() {
                                set(&stt, &status_line(&v));
                            }
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
                let _ = sock.set_read_timeout(Some(Duration::from_millis(250)));
                let mut buf = vec![0u8; 65536];
                let mut from = String::from("-");
                let mut de = Depacketizer::new();
                let mut cx = CompactRx::new();
                let mut compact = false;
                set(&stt, &format!("Waiting for packets on UDP port {port} ..."));
                while !st.load(Ordering::Relaxed) {
                    if let Ok((n, a)) = sock.recv_from(&mut buf) {
                        samples.clear();
                        let p = &buf[..n];
                        compact = CompactRx::is_compact(p);
                        let ok = if compact { cx.packet(p, &mut samples) } else { de.packet(p, &mut samples) };
                        if ok {
                            from = a.to_string();
                            meter.add(n as u64 + 28);
                            if !samples.is_empty() {
                                track_peak(&samples, &pk);
                                an.push(&samples, &scope2);
                                rds.push(&samples);
                                q.push(&samples, if compact { cx.stamp } else { de.stamp });
                            }
                        }
                    }
                    if meter.tick(&vw, &from, if compact { &cx.stats } else { &de.stats }, &q, &card, &rds) {
                        if let Ok(v) = vw.lock() {
                            if v.receiving {
                                set(&stt, &status_line(&v));
                            } else {
                                set(&stt, &format!("Waiting for packets on UDP port {port} ..."));
                            }
                        }
                    }
                }
            }
        });
        RxHandle { stop, status, peak, view, scope }
    }

    pub fn status_text(&self) -> String {
        get(&self.status)
    }

    /// Live numbers: bandwidth, format, sender, losses, buffer and history.
    pub fn view(&self) -> RxView {
        self.view.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn peak(&self) -> f32 {
        f32::from_bits(self.peak.load(Ordering::Relaxed))
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
