//! MPX output: runs the generator (mpxdsp) on the on-air audio and sends the composite signal to a
//! 192 kHz sound card and/or over IP: uncompressed 24-bit (RTP L24 over UDP, raw L24 over TCP) or lossless FLAC
//! (one FLAC frame per RTP packet, or length-prefixed frames over TCP).
use crate::model::MpxCfg;
use crate::mpxdsp::{to_l24, MpxEncoder, MpxParams, RdsData};
use crate::mpxflac::{encode_frame, to_i24, BLOCK, PT_FLAC, PT_L24, TCP_FLAC_MAGIC};
use crate::mpxio::{card_thread, get, set, Queue, Text, FS};
use std::io::Write;
use std::net::{TcpStream, UdpSocket};
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

// ---------------------------------------------------------------- IP sender

fn ip_thread(cfg: MpxCfg, q: Arc<Queue>, stop: Arc<AtomicBool>, status: Text) {
    std::thread::spawn(move || {
        let addr = format!("{}:{}", cfg.ip_host, cfg.ip_port);
        let tcp = cfg.ip_proto == "tcp";
        let flac = cfg.ip_codec == "flac";
        let pkt = if flac { BLOCK } else { PKT };
        let mut udp: Option<UdpSocket> = None;
        let mut tcps: Option<TcpStream> = None;
        let (mut seq, mut ts) = (0u16, 0u32);
        let mut frame_no: u32 = 0;
        let ssrc: u32 = 0x524D_4D58 ^ (std::process::id() << 8); // "RMMX", different at every start
        let mut credit = 0.0f64;
        let mut last = Instant::now();
        let mut samples = Vec::with_capacity(pkt);
        let mut ints: Vec<i32> = Vec::with_capacity(pkt);
        let mut payload = Vec::with_capacity(pkt * 6 + 16);
        let (mut sent, mut raw): (u64, u64) = (0, 0);
        let mut last_status = Instant::now();
        while !stop.load(Ordering::Relaxed) {
            if tcp && tcps.is_none() {
                set(&status, &format!("IP: connecting to {addr} ..."));
                match TcpStream::connect(&addr) {
                    Ok(mut s) => {
                        let _ = s.set_nodelay(true);
                        if flac && s.write_all(TCP_FLAC_MAGIC).is_err() {
                            continue;
                        }
                        tcps = Some(s);
                    }
                    Err(e) => {
                        set(&status, &format!("IP: connection failed ({e}), retrying"));
                        std::thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                }
            }
            if !tcp && udp.is_none() {
                match UdpSocket::bind("0.0.0.0:0").and_then(|s| s.connect(&addr).map(|_| s)) {
                    Ok(s) => udp = Some(s),
                    Err(e) => {
                        set(&status, &format!("IP: {e}"));
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
                if !q.pop_n(pkt, &mut samples) {
                    break;
                }
                credit -= pkt as f64;
                idle = false;
                payload.clear();
                raw += (pkt * 3) as u64;
                if flac {
                    ints.clear();
                    to_i24(&samples, &mut ints);
                    let frame = encode_frame(&ints, frame_no);
                    frame_no = (frame_no + 1) & 0x7FFF_FFFF;
                    if tcp {
                        payload.extend_from_slice(&(frame.len() as u16).to_be_bytes());
                        payload.extend_from_slice(&frame);
                    } else {
                        // one frame per packet (at most about 1170 bytes, below the internet MTU); flags byte 0
                        rtp_header(&mut payload, PT_FLAC, seq, ts, ssrc);
                        payload.push(0);
                        payload.extend_from_slice(&(frame.len() as u16).to_be_bytes());
                        payload.extend_from_slice(&frame);
                    }
                } else {
                    if !tcp {
                        rtp_header(&mut payload, PT_L24, seq, ts, ssrc);
                    }
                    to_l24(&samples, &mut payload);
                }
                if !tcp {
                    seq = seq.wrapping_add(1);
                    ts = ts.wrapping_add(pkt as u32);
                }
                sent += payload.len() as u64;
                if let Some(s) = &udp {
                    let _ = s.send(&payload);
                } else if let Some(s) = tcps.as_mut() {
                    if s.write_all(&payload).is_err() {
                        tcps = None;
                        break;
                    }
                }
            }
            if credit > (pkt * 40) as f64 {
                credit = 0.0; // the producer stalled: do not send a burst afterwards
            }
            if last_status.elapsed() > Duration::from_millis(500) {
                let secs = last_status.elapsed().as_secs_f64();
                last_status = Instant::now();
                let kind = match (tcp, flac) {
                    (false, false) => "UDP/RTP, L24".to_string(),
                    (true, false) => "TCP, L24".to_string(),
                    (false, true) => "UDP/RTP, FLAC lossless".to_string(),
                    (true, true) => "TCP, FLAC lossless".to_string(),
                };
                let ratio = if flac && sent > 0 { format!(" (L24 would be {:.2})", raw as f64 * 8.0 / secs / 1e6) } else { String::new() };
                set(&status, &format!("IP ({kind}): {addr}, {:.2} Mbit/s{ratio}, buffer {} ms, dropped {}", sent as f64 * 8.0 / secs / 1e6, q.ms(), q.drops.load(Ordering::Relaxed)));
                sent = 0;
                raw = 0;
            }
            if idle {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
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
    ip: Text,
    params: Arc<Mutex<(MpxParams, RdsData)>>,
    live_rt: Mutex<String>,
    pub scope: crate::mpxscope::Scope,
}

impl MpxHandle {
    /// `rx` delivers the on-air mix: interleaved stereo at 44.1 kHz.
    pub fn start(cfg: &MpxCfg, rx: Receiver<Vec<f32>>) -> MpxHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU32::new(0));
        let status: Text = Arc::new(Mutex::new("Starting...".into()));
        let (card, ip): (Text, Text) = (Arc::new(Mutex::new(String::new())), Arc::new(Mutex::new(String::new())));
        let params = Arc::new(Mutex::new((params_of(cfg), rds_of(cfg, ""))));
        let mut queues: Vec<Arc<Queue>> = Vec::new();
        if cfg.card_on {
            let q = Queue::new(120);
            card_thread(cfg.card_device.clone(), q.clone(), stop.clone(), card.clone(), "Sound card");
            queues.push(q);
        }
        if cfg.ip_on {
            let q = Queue::new(60);
            ip_thread(cfg.clone(), q.clone(), stop.clone(), ip.clone());
            queues.push(q);
        }
        if queues.is_empty() {
            set(&status, "No output selected (sound card or IP)");
        }
        let scope = crate::mpxscope::new_scope();
        let scope2 = scope.clone();
        let (st, pk, pa, stt) = (stop.clone(), peak.clone(), params.clone(), status.clone());
        let in_rate = 44_100;
        std::thread::spawn(move || {
            let (p0, r0) = pa.lock().map(|g| g.clone()).unwrap_or_default();
            let mut enc = MpxEncoder::new(in_rate, FS, p0, r0);
            let mut out: Vec<f32> = Vec::new();
            let mut an = crate::mpxscope::Analyzer::new();
            let mut secs_at = Instant::now();
            // wait for the first audio, then keep going until stopped
            while !st.load(Ordering::Relaxed) {
                match rx.recv_timeout(Duration::from_millis(250)) {
                    Ok(chunk) => {
                        if let Ok(g) = pa.lock() {
                            enc.set_params(g.0.clone());
                            enc.rds.data = g.1.clone();
                        }
                        out.clear();
                        enc.process(&chunk, &mut out);
                        an.push(&out, &scope2);
                        for q in &queues {
                            q.push(&out);
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
        MpxHandle { stop, peak, status, card, ip, params, live_rt: Mutex::new(String::new()), scope }
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

    pub fn status_lines(&self) -> Vec<String> {
        let mut v = vec![get(&self.status)];
        for t in [&self.card, &self.ip] {
            let s = get(t);
            if !s.is_empty() {
                v.push(s);
            }
        }
        v
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

