//! MPX output: runs the generator (mpxdsp) on the on-air audio and sends the composite signal to a
//! 192 kHz sound card and/or over IP: uncompressed 24-bit (RTP L24 over UDP, raw L24 over TCP) or lossless FLAC
//! (one FLAC frame per RTP packet, or length-prefixed frames over TCP).
use crate::clock::{ClockState, RefClock};
use crate::compact;
use crate::sfn::{rtp_header_sfn, Timeline};
use crate::model::{IpTarget, MpxCfg};
use crate::mpxdsp::{MpxEncoder, MpxParams, RdsData};
use crate::mpxflac::{encode_frame_bits, pack_pcm, Quantizer, BLOCK, PT_FLAC, PT_L16, PT_L20, PT_L24, TCP_FLAC_MAGIC, TCP_PCM_MAGIC};
use crate::mpxio::{card_thread, get, set, Queue, Text, FS};
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PKT: usize = 240; // L24: samples per IP packet (1.25 ms)

fn rtp_header(out: &mut Vec<u8>, pt: u8, seq: u16, ts: u32, ssrc: u32) {
    out.extend_from_slice(&[0x80, pt]);
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&ts.to_be_bytes());
    out.extend_from_slice(&ssrc.to_be_bytes());
}

// ---------------------------------------------------------------- link statistics

/// Values kept in the bandwidth history: 2 per second, 2 minutes.
pub const HIST: usize = 240;

/// Live numbers of one IP output (read by the window).
#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LinkStat {
    /// Bandwidth on the network now, Mbit/s (payload + RTP + IP/UDP or TCP headers).
    pub mbps: f32,
    /// What uncompressed L24 would need for the same samples (shows the FLAC saving).
    pub l24_mbps: f32,
    /// Data sent since start, megabytes.
    pub total_mb: f64,
    /// Connected (TCP) / sending (UDP).
    pub ok: bool,
    /// State or last error.
    pub text: String,
    /// Bandwidth history (oldest first).
    pub hist: std::collections::VecDeque<f32>,
}

pub type Link = Arc<Mutex<LinkStat>>;

fn link_set(l: &Link, f: impl FnOnce(&mut LinkStat)) {
    if let Ok(mut g) = l.lock() {
        f(&mut g);
    }
}

pub fn push_hist(h: &mut std::collections::VecDeque<f32>, v: f32) {
    h.push_back(v);
    while h.len() > HIST {
        h.pop_front();
    }
}

// ---------------------------------------------------------------- IP sender (one thread per target)

const UDP_IP_HEADERS: u64 = 28; // IPv4 20 + UDP 8
const TCP_IP_HEADERS: u64 = 40; // IPv4 20 + TCP 20 (per write; segments may be larger)

/// Marks the packets of a socket with the DSCP class of the target (QoS). Returns a note when it did not work.
fn set_dscp(sock: socket2::SockRef<'_>, dscp: u8) -> String {
    if dscp == 0 {
        return String::new();
    }
    match sock.set_tos(((dscp & 0x3F) as u32) << 2) {
        Ok(()) => String::new(),
        Err(e) => format!(" (QoS not set: {e})"),
    }
}

fn ip_thread(t: IpTarget, q: Arc<Queue>, stop: Arc<AtomicBool>, link: Link, tl: Option<Arc<Timeline>>) {
    std::thread::spawn(move || {
        let addr = t.address();
        let tcp = t.proto == "tcp";
        let flac = t.codec == "flac";
        let bits = t.bits();
        let pt_pcm = match bits {
            16 => PT_L16,
            20 => PT_L20,
            _ => PT_L24,
        };
        let mut quant = Quantizer::new(bits);
        let pkt = if flac { BLOCK } else { PKT };
        let mut udp: Option<UdpSocket> = None;
        let mut tcps: Option<TcpStream> = None;
        let (mut seq, mut ts) = (0u16, 0u32);
        let mut frame_no: u32 = 0;
        // "RMMX" mixed with the process id and the port: every target and every start has its own stream id
        let ssrc: u32 = 0x524D_4D58 ^ (std::process::id() << 8) ^ ((t.port as u32) << 16);
        let mut credit = 0.0f64;
        let mut last = Instant::now();
        let mut samples = Vec::with_capacity(pkt);
        let mut ints: Vec<i32> = Vec::with_capacity(pkt);
        let mut payload = Vec::with_capacity(pkt * 6 + 16);
        let (mut sent, mut raw): (u64, u64) = (0, 0);
        let mut total: u64 = 0;
        let mut last_status = Instant::now();
        let mut qos_note = String::new();
        let qos = if t.dscp == 0 { String::new() } else { format!(", QoS {}", t.dscp_name()) };
        while !stop.load(Ordering::Relaxed) {
            if tcp && tcps.is_none() {
                link_set(&link, |l| {
                    l.ok = false;
                    l.text = format!("Connecting to {addr} ...");
                });
                let conn = addr
                    .to_socket_addrs()
                    .ok()
                    .and_then(|mut a| a.next())
                    .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "address not found"))
                    .and_then(|a| TcpStream::connect_timeout(&a, Duration::from_secs(4)));
                match conn {
                    Ok(mut s) => {
                        let _ = s.set_nodelay(true);
                        qos_note = set_dscp(socket2::SockRef::from(&s), t.dscp);
                        let _ = s.set_write_timeout(Some(Duration::from_secs(3)));
                        let start: Vec<u8> = if flac {
                            TCP_FLAC_MAGIC.to_vec()
                        } else if bits < 24 {
                            [&TCP_PCM_MAGIC[..], &[bits as u8]].concat()
                        } else {
                            Vec::new() // 24-bit PCM: raw stream, as in the first versions
                        };
                        if !start.is_empty() && s.write_all(&start).is_err() {
                            std::thread::sleep(Duration::from_secs(1));
                            continue;
                        }
                        tcps = Some(s);
                        link_set(&link, |l| {
                            l.ok = true;
                            l.text = "Connected".into();
                        });
                        last = Instant::now();
                        credit = 0.0;
                    }
                    Err(e) => {
                        link_set(&link, |l| {
                            l.ok = false;
                            l.text = format!("Connection failed: {e}");
                            l.mbps = 0.0;
                            push_hist(&mut l.hist, 0.0);
                        });
                        for _ in 0..20 {
                            if stop.load(Ordering::Relaxed) {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        continue;
                    }
                }
            }
            if !tcp && udp.is_none() {
                match UdpSocket::bind("0.0.0.0:0").and_then(|s| s.connect(&addr).map(|_| s)) {
                    Ok(s) => {
                        qos_note = set_dscp(socket2::SockRef::from(&s), t.dscp);
                        udp = Some(s);
                        link_set(&link, |l| {
                            l.ok = true;
                            l.text = "Sending".into();
                        });
                    }
                    Err(e) => {
                        link_set(&link, |l| {
                            l.ok = false;
                            l.text = format!("{e}");
                        });
                        std::thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                }
            }
            let now = Instant::now();
            // the receiver's clock is the sender's clock; follow the queue fill level so it neither empties nor grows
            let fill = q.len() as f64 / q.prefill.max(1) as f64;
            let factor = 1.0 + (fill - 1.0).clamp(-1.0, 1.0) * 0.003;
            credit += now.duration_since(last).as_secs_f64() * FS as f64 * factor;
            last = now;
            let mut idle = true;
            while credit >= pkt as f64 && q.len() >= q.prefill.min(pkt * 4).max(pkt) {
                samples.clear();
                let Some(first) = q.pop_n_at(pkt, &mut samples) else { break };
                let stamp = tl.as_ref().and_then(|t| t.stamp(first));
                credit -= pkt as f64;
                idle = false;
                payload.clear();
                raw += (pkt * 3) as u64 + pkt as u64 * (12 + UDP_IP_HEADERS) / PKT as u64; // L24 over RTP/UDP
                ints.clear();
                quant.run(&samples, &mut ints);
                if flac {
                    let frame = encode_frame_bits(&ints, frame_no, bits);
                    frame_no = (frame_no + 1) & 0x7FFF_FFFF;
                    if tcp {
                        payload.extend_from_slice(&(frame.len() as u16).to_be_bytes());
                        payload.extend_from_slice(&frame);
                    } else {
                        // one frame per packet (at most about 1170 bytes, below the internet MTU); flags byte 0
                        match stamp {
                            Some(st) => rtp_header_sfn(&mut payload, PT_FLAC, seq, ts, ssrc, st),
                            None => rtp_header(&mut payload, PT_FLAC, seq, ts, ssrc),
                        }
                        payload.push(0);
                        payload.extend_from_slice(&(frame.len() as u16).to_be_bytes());
                        payload.extend_from_slice(&frame);
                    }
                } else {
                    if !tcp {
                        match stamp {
                            Some(st) => rtp_header_sfn(&mut payload, pt_pcm, seq, ts, ssrc, st),
                            None => rtp_header(&mut payload, pt_pcm, seq, ts, ssrc),
                        }
                    }
                    pack_pcm(&ints, bits, &mut payload);
                }
                if !tcp {
                    seq = seq.wrapping_add(1);
                    ts = ts.wrapping_add(pkt as u32);
                }
                let wire = payload.len() as u64 + if tcp { TCP_IP_HEADERS } else { UDP_IP_HEADERS };
                sent += wire;
                total += wire;
                if let Some(s) = &udp {
                    let _ = s.send(&payload);
                } else if let Some(s) = tcps.as_mut() {
                    if let Err(e) = s.write_all(&payload) {
                        tcps = None;
                        link_set(&link, |l| {
                            l.ok = false;
                            l.text = format!("Disconnected: {e}");
                        });
                        break;
                    }
                }
            }
            if credit > (pkt * 40) as f64 {
                credit = 0.0; // the producer stalled: do not send a burst afterwards
            }
            if last_status.elapsed() >= Duration::from_millis(500) {
                let secs = last_status.elapsed().as_secs_f64();
                last_status = Instant::now();
                let mbps = (sent as f64 * 8.0 / secs / 1e6) as f32;
                let l24 = (raw as f64 * 8.0 / secs / 1e6) as f32;
                let drops = q.drops.load(Ordering::Relaxed);
                link_set(&link, |l| {
                    l.mbps = mbps;
                    l.l24_mbps = l24;
                    l.total_mb = total as f64 / 1e6;
                    push_hist(&mut l.hist, mbps);
                    if l.ok {
                        let state = if tcp { "Connected" } else { "Sending" };
                        l.text = if drops == 0 { format!("{state}{qos}{qos_note}") } else { format!("{state}{qos}{qos_note} (buffer overflows: {drops})") };
                    }
                });
                sent = 0;
                raw = 0;
            }
            if idle {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        link_set(&link, |l| {
            l.ok = false;
            l.mbps = 0.0;
            l.text = "Stopped".into();
        });
    });
}

// ---------------------------------------------------------------- Compact MPX sender (one thread per target)

/// Sends the processed stereo audio of the generator as Compact MPX (FLAC or Opus + levels + RDS), see compact.rs.
/// `q` holds interleaved stereo at 192 kHz; the RTP timestamps count 192 kHz samples like the other formats.
/// `lag`: how many samples the audio in `q` is late against the composite (time stamps are corrected by it).
fn compact_thread(t: IpTarget, q: Arc<Queue>, params: Arc<Mutex<(MpxParams, RdsData)>>, stop: Arc<AtomicBool>, link: Link, tl: Option<Arc<Timeline>>, lag: u64) {
    std::thread::spawn(move || {
        let addr = t.address();
        let tcp = t.proto == "tcp";
        let codec = if t.codec == "copus" { compact::Codec::Opus } else { compact::Codec::Flac };
        let mut enc = match compact::Encoder::new(codec, t.opus_kbps) {
            Ok(e) => e,
            Err(e) => {
                link_set(&link, |l| l.text = format!("Codec error: {e}"));
                return;
            }
        };
        // whole packets: 20 ms (Opus) or 6.7 ms (FLAC) of audio, i.e. this many 192 kHz frames
        let pkt = codec.frame_192();
        let mut udp: Option<UdpSocket> = None;
        let mut tcps: Option<TcpStream> = None;
        let (mut seq, mut ts) = (0u16, 0u32);
        let ssrc: u32 = 0x524D_4D43 ^ (std::process::id() << 8) ^ ((t.port as u32) << 16);
        let mut credit = 0.0f64;
        let mut last = Instant::now();
        let mut samples = Vec::with_capacity(pkt * 2);
        let mut packets: Vec<Vec<u8>> = Vec::new();
        let mut payload = Vec::with_capacity(2048);
        let (mut sent, mut raw, mut total): (u64, u64, u64) = (0, 0, 0);
        let mut last_status = Instant::now();
        let mut qos_note = String::new();
        let qos = if t.dscp == 0 { String::new() } else { format!(", QoS {}", t.dscp_name()) };
        while !stop.load(Ordering::Relaxed) {
            if tcp && tcps.is_none() {
                link_set(&link, |l| {
                    l.ok = false;
                    l.text = format!("Connecting to {addr} ...");
                });
                let conn = addr
                    .to_socket_addrs()
                    .ok()
                    .and_then(|mut a| a.next())
                    .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "address not found"))
                    .and_then(|a| TcpStream::connect_timeout(&a, Duration::from_secs(4)));
                match conn {
                    Ok(mut s) => {
                        let _ = s.set_nodelay(true);
                        let _ = s.set_write_timeout(Some(Duration::from_secs(3)));
                        qos_note = set_dscp(socket2::SockRef::from(&s), t.dscp);
                        if s.write_all(compact::TCP_COMPACT_MAGIC).is_err() {
                            std::thread::sleep(Duration::from_secs(1));
                            continue;
                        }
                        tcps = Some(s);
                        link_set(&link, |l| {
                            l.ok = true;
                            l.text = "Connected".into();
                        });
                        last = Instant::now();
                        credit = 0.0;
                    }
                    Err(e) => {
                        link_set(&link, |l| {
                            l.ok = false;
                            l.text = format!("Connection failed: {e}");
                            l.mbps = 0.0;
                            push_hist(&mut l.hist, 0.0);
                        });
                        for _ in 0..20 {
                            if stop.load(Ordering::Relaxed) {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        continue;
                    }
                }
            }
            if !tcp && udp.is_none() {
                match UdpSocket::bind("0.0.0.0:0").and_then(|s| s.connect(&addr).map(|_| s)) {
                    Ok(s) => {
                        qos_note = set_dscp(socket2::SockRef::from(&s), t.dscp);
                        udp = Some(s);
                        link_set(&link, |l| {
                            l.ok = true;
                            l.text = "Sending".into();
                        });
                    }
                    Err(e) => {
                        link_set(&link, |l| {
                            l.ok = false;
                            l.text = format!("{e}");
                        });
                        std::thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                }
            }
            let now = Instant::now();
            let fill = q.len() as f64 / q.prefill.max(1) as f64;
            let factor = 1.0 + (fill - 1.0).clamp(-1.0, 1.0) * 0.003;
            credit += now.duration_since(last).as_secs_f64() * FS as f64 * factor;
            last = now;
            let mut idle = true;
            while credit >= pkt as f64 && q.len() >= (pkt * 2).max(q.prefill.min(pkt * 8)) {
                samples.clear();
                // interleaved stereo: two values per sample
                let Some(first) = q.pop_n_at(pkt * 2, &mut samples) else { break };
                credit -= pkt as f64;
                idle = false;
                raw += (pkt * 3) as u64 + pkt as u64 * (12 + UDP_IP_HEADERS) / PKT as u64;
                let (pr, rd) = params.lock().map(|g| (g.0.clone(), g.1.clone())).unwrap_or_default();
                // packets cover whole blocks of the queue, so the stamp of this block's first sample belongs to the
                // packet that the encoder finishes with it (Opus: 20 ms = one block per packet)
                let stamp = tl.as_ref().and_then(|t| t.stamp((first / 2).saturating_sub(lag)));
                enc.push(&samples, &pr, &rd, &mut packets);
                for p in packets.drain(..) {
                    payload.clear();
                    if tcp {
                        payload.extend_from_slice(&(p.len() as u16).to_be_bytes());
                    } else {
                        match stamp {
                            Some(st) => rtp_header_sfn(&mut payload, compact::PT_COMPACT, seq, ts, ssrc, st),
                            None => rtp_header(&mut payload, compact::PT_COMPACT, seq, ts, ssrc),
                        }
                        seq = seq.wrapping_add(1);
                        ts = ts.wrapping_add(pkt as u32);
                    }
                    payload.extend_from_slice(&p);
                    let wire = payload.len() as u64 + if tcp { TCP_IP_HEADERS } else { UDP_IP_HEADERS };
                    sent += wire;
                    total += wire;
                    if let Some(s) = &udp {
                        let _ = s.send(&payload);
                    } else if let Some(s) = tcps.as_mut() {
                        if let Err(e) = s.write_all(&payload) {
                            tcps = None;
                            link_set(&link, |l| {
                                l.ok = false;
                                l.text = format!("Disconnected: {e}");
                            });
                            break;
                        }
                    }
                }
            }
            if credit > (pkt * 20) as f64 {
                credit = 0.0;
            }
            if last_status.elapsed() >= Duration::from_millis(500) {
                let secs = last_status.elapsed().as_secs_f64();
                last_status = Instant::now();
                let mbps = (sent as f64 * 8.0 / secs / 1e6) as f32;
                let l24 = (raw as f64 * 8.0 / secs / 1e6) as f32;
                let drops = q.drops.load(Ordering::Relaxed);
                link_set(&link, |l| {
                    l.mbps = mbps;
                    l.l24_mbps = l24;
                    l.total_mb = total as f64 / 1e6;
                    push_hist(&mut l.hist, mbps);
                    if l.ok {
                        let state = if tcp { "Connected" } else { "Sending" };
                        l.text = if drops == 0 { format!("{state}{qos}{qos_note}") } else { format!("{state}{qos}{qos_note} (buffer overflows: {drops})") };
                    }
                });
                sent = 0;
                raw = 0;
            }
            if idle {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        link_set(&link, |l| {
            l.ok = false;
            l.mbps = 0.0;
            l.text = "Stopped".into();
        });
    });
}

// ---------------------------------------------------------------- generator

pub fn params_of(c: &MpxCfg) -> MpxParams {
    MpxParams { stereo: c.stereo, audio_level: c.audio_level, pilot_level: c.pilot_level, rds_level: c.rds_level, preemph_us: c.preemph_us, limiter: c.limiter, output: c.output }
}

pub fn rds_of(c: &MpxCfg, rt: &str) -> RdsData {
    RdsData {
        enabled: c.rds_on,
        pi: u16::from_str_radix(c.pi.trim().trim_start_matches("0x"), 16).unwrap_or(0x1234),
        ps: c.ps.clone(),
        rt: if rt.is_empty() { c.rt.clone() } else { rt.to_string() },
        pty: c.pty,
        tp: c.tp,
        ta: c.ta,
        music: c.music,
    }
}

pub struct MpxHandle {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU32>,
    status: Text,
    card: Text,
    links: Vec<(IpTarget, Link)>,
    clock: Option<Arc<RefClock>>,
    params: Arc<Mutex<(MpxParams, RdsData)>>,
    live_rt: Mutex<String>,
    pub scope: crate::mpxscope::Scope,
}

impl MpxHandle {
    /// `rx` delivers the on-air mix: interleaved stereo at 44.1 kHz.
    pub fn start(cfg: &MpxCfg, rx: Receiver<Vec<f32>>) -> MpxHandle {
        Self::start_with(cfg, rx, false)
    }

    /// `rx` delivers a ready composite signal (MPX) at 192 kHz, e.g. from a sound card input: it is sent to the
    /// outputs as it is (no generator, so the Signal and RDS settings are not used). For Compact MPX outputs it is
    /// decoded first (stereo audio, pilot and RDS levels, RDS data), and the receiver rebuilds the composite.
    pub fn start_input(cfg: &MpxCfg, rx: Receiver<Vec<f32>>) -> MpxHandle {
        Self::start_with(cfg, rx, true)
    }

    fn start_with(cfg: &MpxCfg, rx: Receiver<Vec<f32>>, passthrough: bool) -> MpxHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU32::new(0));
        let status: Text = Arc::new(Mutex::new("Starting...".into()));
        let card: Text = Arc::new(Mutex::new(String::new()));
        let mut links: Vec<(IpTarget, Link)> = Vec::new();
        // SFN: the reference clock and the time line that gives every sample its UTC time
        let clock = cfg.sfn_on.then(|| RefClock::start(&cfg.clock));
        let timeline = clock.as_ref().map(|c| Timeline::new(c.clone()));
        let params = Arc::new(Mutex::new((params_of(cfg), rds_of(cfg, ""))));
        let mut queues: Vec<Arc<Queue>> = Vec::new();
        let mut cqueues: Vec<Arc<Queue>> = Vec::new();
        if cfg.card_on {
            let q = Queue::new(120);
            card_thread(cfg.card_device.clone(), q.clone(), stop.clone(), card.clone(), "Sound card");
            queues.push(q);
        }
        if cfg.ip_on {
            for t in cfg.targets.iter().filter(|t| t.enabled && !t.host.trim().is_empty()) {
                let link: Link = Arc::new(Mutex::new(LinkStat { text: "Starting...".into(), ..LinkStat::default() }));
                if t.is_compact() {
                    // Compact MPX: gets the processed stereo audio (interleaved, 192 kHz) instead of the composite
                    let q = Queue::new(120);
                    // an MPX input reaches the Compact encoder through the stereo decoder, which delays it
                    let lag = if passthrough { crate::demux::StereoDecoder::DELAY as u64 } else { 0 };
                    compact_thread(t.clone(), q.clone(), params.clone(), stop.clone(), link.clone(), timeline.clone(), lag);
                    cqueues.push(q);
                } else {
                    let q = Queue::new(60);
                    ip_thread(t.clone(), q.clone(), stop.clone(), link.clone(), timeline.clone());
                    queues.push(q);
                }
                links.push((t.clone(), link));
            }
        }
        if queues.is_empty() && cqueues.is_empty() {
            set(&status, "No output selected (sound card or IP)");
        }
        let scope = crate::mpxscope::new_scope();
        let scope2 = scope.clone();
        let (st, pk, pa, stt, tl) = (stop.clone(), peak.clone(), params.clone(), status.clone(), timeline.clone());
        let in_rate = 44_100;
        std::thread::spawn(move || {
            let (p0, r0) = pa.lock().map(|g| g.clone()).unwrap_or_default();
            let mut enc = MpxEncoder::new(in_rate, FS, p0, r0);
            if !cqueues.is_empty() && !passthrough {
                enc.tap = Some(Vec::new());
            }
            // MPX input with Compact outputs: decode the composite (stereo audio, pilot and RDS levels, RDS data)
            // so Compact MPX can carry it; the receiver rebuilds the composite from that
            let mut demux = (passthrough && !cqueues.is_empty()).then(|| (crate::demux::StereoDecoder::new(FS), crate::rdsdec::RdsDecoder::new(FS), Vec::<f32>::new()));
            let mut out: Vec<f32> = Vec::new();
            let mut an = crate::mpxscope::Analyzer::new();
            let mut secs_at = Instant::now();
            let (mut produced, mut tl_at): (u64, Instant) = (0, Instant::now());
            // wait for the first audio, then keep going until stopped
            while !st.load(Ordering::Relaxed) {
                match rx.recv_timeout(Duration::from_millis(250)) {
                    Ok(chunk) if passthrough => {
                        out.clear();
                        out.extend(chunk.iter().map(|v| v.clamp(-1.0, 1.0)));
                        an.push(&out, &scope2);
                        produced += out.len() as u64;
                        if let Some(t) = &tl {
                            t.update(produced, tl_at.elapsed().as_secs_f64());
                            tl_at = Instant::now();
                        }
                        for q in &queues {
                            q.push(&out);
                        }
                        if let Some((st_dec, rds_dec, lr)) = demux.as_mut() {
                            lr.clear();
                            st_dec.push(&out, lr);
                            rds_dec.push(&out);
                            for q in &cqueues {
                                q.push(lr);
                            }
                            // what the receiver needs to rebuild this composite: the audio is already at the
                            // composite's scale (audio level 1), pilot and RDS at their measured levels
                            let i = &rds_dec.info;
                            let p = MpxParams { stereo: st_dec.stereo(), audio_level: 1.0, pilot_level: st_dec.pilot_level().clamp(0.0, 0.2), rds_level: if i.synced { rds_dec.level().clamp(0.0, 0.15) } else { 0.0 }, preemph_us: 0.0, limiter: false, output: 1.0 };
                            let r = RdsData { enabled: i.synced, pi: i.pi.unwrap_or(0), ps: i.ps.clone(), rt: i.rt.clone(), pty: i.pty.unwrap_or(0), tp: i.tp, ta: i.ta, music: i.music };
                            if let Ok(mut g) = pa.lock() {
                                *g = (p, r);
                            }
                        }
                        let p = out.iter().fold(0f32, |m, v| m.max(v.abs()));
                        let old = f32::from_bits(pk.load(Ordering::Relaxed));
                        pk.store(p.max(old * 0.9).to_bits(), Ordering::Relaxed);
                        if secs_at.elapsed() > Duration::from_secs(1) {
                            secs_at = Instant::now();
                            set(&stt, "Running (MPX input)");
                        }
                    }
                    Ok(chunk) => {
                        if let Ok(g) = pa.lock() {
                            enc.set_params(g.0.clone());
                            enc.set_rds(g.1.clone());
                        }
                        out.clear();
                        enc.process(&chunk, &mut out);
                        an.push(&out, &scope2);
                        produced += out.len() as u64;
                        if let Some(t) = &tl {
                            t.update(produced, tl_at.elapsed().as_secs_f64());
                            tl_at = Instant::now();
                        }
                        for q in &queues {
                            q.push(&out);
                        }
                        if let Some(tap) = enc.tap.as_mut() {
                            for q in &cqueues {
                                q.push(tap);
                            }
                            tap.clear();
                        }
                        let p = enc.take_peak();
                        pk.store(p.to_bits(), Ordering::Relaxed);
                        if secs_at.elapsed() > Duration::from_secs(1) {
                            secs_at = Instant::now();
                            set(&stt, "Running");
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        set(&stt, "Waiting for audio...");
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        MpxHandle { stop, peak, status, card, links, clock, params, live_rt: Mutex::new(String::new()), scope }
    }

    /// Applies changed levels / RDS texts while running.
    pub fn update(&self, cfg: &MpxCfg) {
        let rt = self.live_rt.lock().map(|g| g.clone()).unwrap_or_default();
        let rt = if cfg.rt_from_playlist { rt } else { String::new() };
        if let Ok(mut g) = self.params.lock() {
            *g = (params_of(cfg), rds_of(cfg, &rt));
        }
    }

    /// Sets the RadioText from the playing song.
    pub fn set_radiotext(&self, cfg: &MpxCfg, text: &str) {
        if let Ok(mut g) = self.live_rt.lock() {
            *g = text.to_string();
        }
        self.update(cfg);
    }

    /// Highest composite level of the last block (1.0 = 75 kHz deviation).
    pub fn peak(&self) -> f32 {
        f32::from_bits(self.peak.load(Ordering::Relaxed))
    }

    /// State of the generator ("Running", "Waiting for audio...").
    pub fn status(&self) -> String {
        get(&self.status)
    }

    /// State of the 192 kHz sound card output (empty when it is off).
    pub fn card_status(&self) -> String {
        get(&self.card)
    }

    /// Every IP output with its live numbers.
    pub fn links(&self) -> Vec<(IpTarget, LinkStat)> {
        self.links.iter().map(|(t, l)| (t.clone(), l.lock().map(|g| g.clone()).unwrap_or_default())).collect()
    }

    /// The SFN reference clock (None when SFN is off).
    pub fn clock_state(&self) -> Option<ClockState> {
        self.clock.as_ref().map(|c| c.state())
    }

    /// Sum of the bandwidth of all IP outputs, Mbit/s.
    pub fn total_mbps(&self) -> f32 {
        self.links.iter().map(|(_, l)| l.lock().map(|g| g.mbps).unwrap_or(0.0)).sum()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(c) = &self.clock {
            c.stop();
        }
    }
}

