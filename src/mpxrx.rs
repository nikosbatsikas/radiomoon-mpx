//! MPX receiver (transmitter site): IP -> 192 kHz sound card. Used by the separate receiver program.
//! L24 and FLAC are detected automatically (RTP payload type over UDP, the stream start over TCP).
use crate::mpx::push_hist;
use crate::compact::{CompactRx, CompactTcp, TCP_COMPACT_MAGIC};
use crate::mpxflac::{Depacketizer, RxStats, TcpDeframer};
use crate::clock::RefClock;
use crate::mpxio::{card_thread, get, set, sfn_card_thread, track_peak, Queue, Text};
use crate::rdsdec::RdsDecoder;
use crate::protect::{AllowList, Gate, ProtStats, TcpGate, Unprotector};
use crate::clock::{ClockCfg, ClockState};
use crate::sfn::{SfnCfg, SfnPlayout, SfnStatus};
use std::collections::VecDeque;
use std::io::Read;
use std::net::{TcpListener, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
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
    /// Protection: password of an encrypted stream (empty = accept unencrypted streams only).
    pub password: String,
    /// Allowed senders: addresses / networks (empty = everyone).
    pub allow: String,
    /// Backup MPX from a sound card input when the stream stops.
    pub backup: BackupCfg,
}

/// Backup source of the receiver: a ready MPX signal from a sound card input (e.g. an analog link, an off-air
/// receiver or a local processor), put on air when the IP stream stops.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct BackupCfg {
    pub on: bool,
    /// Sound input that records at 192 kHz.
    pub device: String,
    /// 0 = left, 1 = right.
    pub channel: usize,
    pub gain_db: f32,
    /// Switch to the backup when no stream came for this long.
    pub timeout_ms: u32,
    /// Switch back after the stream came without a break for this long.
    pub return_s: f32,
    /// Also switch when the stream carries silence (its MPX below about -40 dB).
    pub on_silence: bool,
}

impl Default for BackupCfg {
    fn default() -> Self {
        BackupCfg { on: false, device: String::new(), channel: 0, gain_db: 0.0, timeout_ms: 1000, return_s: 5.0, on_silence: false }
    }
}

/// Live state of the backup source.
#[derive(Clone, Default, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct BackupView {
    pub enabled: bool,
    /// The backup is on air.
    pub active: bool,
    /// Switches to the backup since start.
    pub switches: u64,
    /// State of the sound input.
    pub input: String,
    /// Level of the backup input, 0..1.
    pub level: f32,
    /// Seconds until the stream is back on air (while it returns), else 0.
    pub return_in_s: f32,
}

/// Shared by the receiving thread and the backup thread.
struct Backup {
    t0: Instant,
    /// Time of the last (good) stream samples, ms since t0 (0 = none yet).
    last_ip: AtomicU64,
    active: AtomicBool,
    view: Mutex<BackupView>,
    on_silence: bool,
}

impl Backup {
    fn ms(&self) -> u64 {
        self.t0.elapsed().as_millis() as u64 + 1
    }
}

/// What goes on air: the samples of the stream, or of the backup while it is active; analysed (scope, RDS,
/// peak) in both cases.
struct OnAir {
    sink: Sink,
    an: crate::mpxscope::Analyzer,
    rds: RdsDecoder,
}

/// Where the received samples go: the normal buffer, or the SFN playout.
#[derive(Clone)]
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
    /// Extended Country Code.
    pub ecc: Option<u8>,
}

impl RdsView {
    fn from(d: &RdsDecoder, receiving: bool) -> RdsView {
        let i = &d.info;
        RdsView { synced: i.synced && receiving, pi: i.pi, ps: i.ps.clone(), rt: i.rt.clone(), pty: i.pty, tp: i.tp, ta: i.ta, music: i.music, quality: i.quality, groups: i.groups, af: i.af.clone(), ct: i.ct.clone(), ecc: i.ecc }
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
    /// Protection: encryption, FEC, rejected and blocked packets.
    pub protect: ProtStats,
    /// Backup source (sound card input) and whether it is on air.
    pub backup: BackupView,
    /// Event log: stream lost / back, lost packets, refused senders, backup ... (oldest first).
    pub events: VecDeque<crate::events::Event>,
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
        3 => "AM audio (FLAC)".into(),
        4 => "AM audio (Opus)".into(),
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
    if v.backup.active {
        t = format!("ON BACKUP (sound card input); the stream is back on air in {:.0} s. {t}", v.backup.return_in_s);
    }
    let p = &v.protect;
    if p.encrypted {
        t += "; encrypted";
    }
    if p.fec_n > 0 {
        t += &format!("; FEC restored {}", p.restored);
    }
    t
}

/// Why nothing plays although packets arrive (protection), or the plain waiting text.
fn waiting_line(p: &ProtStats, waiting: String) -> String {
    if p.need_password {
        "An encrypted stream arrives: enter its password (Input tab)".into()
    } else if p.rejected > 0 {
        format!("Packets rejected ({}): wrong password, or changed on the way", p.rejected)
    } else if p.plain_rejected > 0 {
        format!("An unencrypted stream was ignored ({} packets): a password is set here", p.plain_rejected)
    } else if p.blocked > 0 {
        format!("{waiting} (blocked {} from {}: not an allowed sender)", p.blocked, p.blocked_from)
    } else {
        waiting
    }
}

/// Captures the backup MPX all the time (so it is ready at once) and puts it on air while the stream is missing.
fn backup_thread(c: BackupCfg, b: Arc<Backup>, air: Arc<Mutex<OnAir>>, stop: Arc<AtomicBool>, pk: Arc<AtomicU32>, scope: crate::mpxscope::Scope) {
    std::thread::spawn(move || {
        let (src, rx) = crate::srcin::start_mpx(&c.device, c.channel, c.gain_db);
        let mut good_since: Option<u64> = None;
        let mut fade = 0usize; // fade-in after switching, samples
        const FADE: usize = 1920; // 10 ms
        let mut level = 0f32;
        while !stop.load(Ordering::Relaxed) {
            let blk = rx.recv_timeout(Duration::from_millis(100)).ok();
            let now = b.ms();
            let last = b.last_ip.load(Ordering::Relaxed);
            let age = if last == 0 { now } else { now - last.min(now) };
            let active = b.active.load(Ordering::Relaxed);
            // the stream is missing: backup on air
            if !active && age > c.timeout_ms as u64 {
                b.active.store(true, Ordering::Relaxed);
                fade = 0;
                good_since = None;
                if let Ok(mut v) = b.view.lock() {
                    v.switches += 1;
                }
            }
            let mut return_in = 0f32;
            if b.active.load(Ordering::Relaxed) {
                // the stream must come back without a break for return_s before it goes on air again
                if age < 300 {
                    let since = *good_since.get_or_insert(now);
                    let waited = (now - since) as f32 / 1000.0;
                    return_in = (c.return_s - waited).max(0.0);
                    if waited >= c.return_s {
                        b.active.store(false, Ordering::Relaxed);
                        good_since = None;
                    }
                } else {
                    good_since = None;
                    return_in = c.return_s;
                }
            }
            if let Some(mut blk) = blk {
                level = blk.iter().fold(level * 0.9, |m, v| m.max(v.abs()));
                if b.active.load(Ordering::Relaxed) {
                    for v in blk.iter_mut() {
                        if fade < FADE {
                            *v *= fade as f32 / FADE as f32;
                            fade += 1;
                        }
                    }
                    track_peak(&blk, &pk);
                    if let Ok(mut a) = air.lock() {
                        a.an.push(&blk, &scope);
                        a.rds.push(&blk);
                        a.sink.push(&blk, None);
                    }
                }
            }
            if let Ok(mut v) = b.view.lock() {
                v.active = b.active.load(Ordering::Relaxed);
                v.input = src.status();
                v.level = level;
                v.return_in_s = return_in;
            }
        }
        src.stop();
    });
}

impl RxHandle {
    pub fn start(cfg: &RxCfg) -> RxHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let status: Text = Arc::new(Mutex::new("Starting...".into()));
        let peak = Arc::new(AtomicU32::new(0));
        let view = Arc::new(Mutex::new(RxView::default()));
        let card: Text = Arc::new(Mutex::new(String::new()));
        // SFN with the Transmitter's clock: the stream's time packets feed it
        let mut time_clock: Option<Arc<RefClock>> = None;
        let q = match (&cfg.sfn, cfg.proto == "tcp") {
            (Some((sc, cc)), false) => {
                let rc = RefClock::start(cc);
                if cc.source == "transmitter" {
                    time_clock = Some(rc.clone());
                }
                let pl = SfnPlayout::new(sc.clone(), rc);
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
        let air = Arc::new(Mutex::new(OnAir { sink: q, an: crate::mpxscope::Analyzer::new(), rds: RdsDecoder::new(192_000) }));
        let backup = cfg.backup.on.then(|| {
            let b = Arc::new(Backup { t0: Instant::now(), last_ip: AtomicU64::new(0), active: AtomicBool::new(false), view: Mutex::new(BackupView { enabled: true, ..Default::default() }), on_silence: cfg.backup.on_silence });
            backup_thread(cfg.backup.clone(), b.clone(), air.clone(), stop.clone(), peak.clone(), scope.clone());
            b
        });
        let (st, stt, pk, vw, port, tcp) = (stop.clone(), status.clone(), peak.clone(), view.clone(), cfg.port, cfg.proto == "tcp");
        // the stream's samples: on air unless the backup is
        let bk = backup.clone();
        let feed = move |samples: &[f32], stamp: Option<(usize, i64)>, air: &Mutex<OnAir>| {
            if let Some(b) = &bk {
                let good = !b.on_silence || samples.iter().fold(0f32, |m, v| m.max(v.abs())) > 0.01;
                if good {
                    b.last_ip.store(b.ms(), Ordering::Relaxed);
                }
                if b.active.load(Ordering::Relaxed) {
                    return;
                }
            }
            track_peak(samples, &pk);
            if let Ok(mut a) = air.lock() {
                a.an.push(samples, &scope2);
                a.rds.push(samples);
                a.sink.push(samples, stamp);
            }
        };
        let bk = backup.clone();
        let (password, allow) = (cfg.password.clone(), AllowList::parse(&cfg.allow));
        std::thread::spawn(move || {
            // the key is made from the password once (it takes some milliseconds)
            let mut unprot = Unprotector::new(&password);
            let key = (!password.is_empty()).then(|| crate::crypt::stream_key(&password));
            // answers the Transmitter's time packets (and feeds the "transmitter" reference clock)
            let mut responder = crate::timesync::Responder::new(&password, time_clock);
            let mut samples: Vec<f32> = Vec::with_capacity(8192);
            let mut meter = Meter::new();
            // the numbers for the window (also the backup's state)
            let mut tracker = crate::events::Tracker::new();
            let mut tick = |meter: &mut Meter, from: &str, stats: &RxStats| -> bool {
                let Ok(a) = air.lock() else { return false };
                let done = meter.tick(&vw, from, stats, &a.sink, &card, &a.rds);
                drop(a);
                if done {
                    if let Ok(mut v) = vw.lock() {
                        if let Some(b) = &bk {
                            v.backup = b.view.lock().map(|g| g.clone()).unwrap_or_default();
                        }
                        let evs = tracker.observe(&v, crate::status::now_ms());
                        for e in evs {
                            crate::events::push(&mut v.events, e);
                        }
                    }
                }
                done
            };
            let note = |level: u8, text: String| {
                if let Ok(mut v) = vw.lock() {
                    crate::events::push(&mut v.events, crate::events::Event { t: crate::status::now_ms(), level, text });
                }
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
                            Ok(x) if !allow.allows(x.1.ip()) => {
                                unprot.stats.blocked += 1;
                                unprot.stats.blocked_from = x.1.ip().to_string();
                                drop(x);
                                if let Ok(mut v) = vw.lock() {
                                    v.protect = unprot.stats.clone();
                                }
                                set(&stt, &waiting_line(&unprot.stats, format!("Waiting for a sender on TCP port {port} ...")));
                            }
                            Ok(x) => break x,
                            Err(_) => {
                                tick(&mut meter, "-", &RxStats::default());
                                std::thread::sleep(Duration::from_millis(100));
                            }
                        }
                    };
                    let _ = s.set_nonblocking(false);
                    let _ = s.set_read_timeout(Some(Duration::from_millis(300)));
                    let mut buf = vec![0u8; 16384];
                    note(0, format!("TCP connection from {who}"));
                    let mut de = TcpIn::Unknown(Vec::new()); // a new connection may use another format
                    let mut gate = TcpGate::new();
                    let mut plain: Vec<u8> = Vec::with_capacity(16384);
                    let who = who.to_string();
                    while !st.load(Ordering::Relaxed) {
                        match s.read(&mut buf) {
                            Ok(0) => break,
                            Ok(n) => {
                                plain.clear();
                                let refused = match gate.feed(key.as_ref(), &buf[..n], &mut plain) {
                                    Gate::Data => false,
                                    Gate::NeedPassword => {
                                        unprot.stats.need_password = true;
                                        true
                                    }
                                    Gate::PlainRejected => {
                                        unprot.stats.plain_rejected += 1;
                                        true
                                    }
                                    Gate::Rejected => {
                                        unprot.stats.rejected += 1;
                                        true
                                    }
                                };
                                if refused {
                                    unprot.stats.encrypted = false;
                                    if let Ok(mut v) = vw.lock() {
                                        v.protect = unprot.stats.clone();
                                    }
                                    set(&stt, &waiting_line(&unprot.stats, String::new()));
                                    std::thread::sleep(Duration::from_millis(500));
                                    break; // closes the connection
                                }
                                unprot.stats.need_password = false;
                                unprot.stats.encrypted = matches!(gate, TcpGate::Secure(_));
                                meter.add(n as u64 + (n as u64 / 1448 + 1) * 40);
                                samples.clear();
                                de.feed(&plain, &mut samples);
                                if !samples.is_empty() {
                                    feed(&samples, None, &air); // TCP carries no SFN time stamps
                                }
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
                            Err(_) => break,
                        }
                        if tick(&mut meter, &who, &de.stats()) {
                            if let Ok(mut v) = vw.lock() {
                                v.protect = unprot.stats.clone();
                                set(&stt, &status_line(&v));
                            }
                        }
                    }
                    note(1, format!("TCP connection from {who} closed"));
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
                let mut inner: Vec<Vec<u8>> = Vec::new();
                let mut accepted = false;
                set(&stt, &format!("Waiting for packets on UDP port {port} ..."));
                while !st.load(Ordering::Relaxed) {
                    let got = sock.recv_from(&mut buf).ok().filter(|&(n, a)| {
                        // SFN time from the Transmitter: answered at once, not part of the stream
                        if crate::timesync::is_time_packet(&buf[..n]) {
                            if allow.allows(a.ip()) {
                                responder.packet(&buf[..n], a, &sock);
                            }
                            return false;
                        }
                        true
                    });
                    if let Some((n, a)) = got {
                        let d = &buf[..n];
                        inner.clear();
                        let protected = Unprotector::is_protected(d);
                        if !allow.allows(a.ip()) {
                            unprot.stats.blocked += 1;
                            unprot.stats.blocked_from = a.ip().to_string();
                        } else if protected {
                            unprot.udp(d, &mut inner);
                        } else if unprot.has_password() {
                            unprot.stats.plain_rejected += 1;
                        } else {
                            inner.push(d.to_vec());
                        }
                        let mut any = false;
                        for p in &inner {
                            samples.clear();
                            compact = CompactRx::is_compact(p);
                            let ok = if compact { cx.packet(p, &mut samples) } else { de.packet(p, &mut samples) };
                            if ok {
                                any = true;
                                if !samples.is_empty() {
                                    feed(&samples, if compact { cx.stamp } else { de.stamp }, &air);
                                }
                            }
                        }
                        if any {
                            accepted = true;
                        } else if !inner.is_empty() || (protected && unprot.stats.need_password) {
                            accepted = false;
                        }
                        // the bandwidth counts every datagram of an accepted stream (also FEC packets and packets
                        // held back for FEC)
                        if any || (protected && accepted && allow.allows(a.ip())) {
                            from = a.to_string();
                            meter.add(n as u64 + 28);
                        }
                    }
                    if tick(&mut meter, &from, if compact { &cx.stats } else { &de.stats }) {
                        if let Ok(mut v) = vw.lock() {
                            v.protect = unprot.stats.clone();
                            if v.receiving {
                                set(&stt, &status_line(&v));
                            } else {
                                set(&stt, &waiting_line(&unprot.stats, format!("Waiting for packets on UDP port {port} ...")));
                            }
                        }
                    }
                }
            }
        });
        RxHandle { stop, status, peak, view, scope }
    }

    pub fn status_text(&self) -> String {
        let s = get(&self.status);
        match self.view.lock().map(|v| v.backup.clone()) {
            Ok(b) if b.active && !s.starts_with("ON BACKUP") => format!("ON BACKUP (sound card input). {s}"),
            _ => s,
        }
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
