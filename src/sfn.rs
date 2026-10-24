//! SFN (single frequency network): all transmitters of a network must put the same signal on air at the same
//! moment. The MPX Transmitter stamps every UDP packet with the UTC time (ns) of its first sample, taken from a
//! common reference clock (GPS / PTP). Every MPX Receiver plays each sample at exactly "time stamp + delay" by its
//! own reference clock, with the same delay at all sites, and follows the reference with a fine resampler, so the
//! sound card's own clock does not drift away.
//!
//! The time stamp is an RTP header extension (RFC 3550, profile 0x524D "RM", 2 words): older receivers skip it.
use crate::clock::RefClock;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub const EXT_PROFILE: u16 = 0x524D;
const FS: f64 = 192_000.0;

// ================================================================ transmitter: the time line

/// Maps the transmitter's sample counter to UTC: stamp(k) = t0 + k / fs + correction, where the correction follows
/// the reference clock slowly (time constant about 10 s), so the stamps are smooth and every output gets the same
/// stamp for the same sample.
pub struct Timeline {
    clock: Arc<RefClock>,
    g: Mutex<Option<(i128, f64)>>, // (t0 ns, correction ns)
}

impl Timeline {
    pub fn new(clock: Arc<RefClock>) -> Arc<Timeline> {
        Arc::new(Timeline { clock, g: Mutex::new(None) })
    }

    pub fn clock(&self) -> &Arc<RefClock> {
        &self.clock
    }

    /// Called by the generator after it produced samples up to (not including) `k_end`, `dt` seconds after the
    /// previous call.
    pub fn update(&self, k_end: u64, dt: f64) {
        let Some(now) = self.clock.now_ns() else { return };
        let Ok(mut g) = self.g.lock() else { return };
        let nominal = (k_end as f64 * 1e9 / FS) as i128;
        match g.as_mut() {
            None => *g = Some((now - nominal, 0.0)),
            Some((t0, corr)) => {
                let raw = (now - *t0 - nominal) as f64;
                if (raw - *corr).abs() > 0.5e9 {
                    // the generator stalled or the clock stepped: start a new time line
                    *t0 = now - nominal;
                    *corr = 0.0;
                } else {
                    *corr += (raw - *corr) * (dt / 10.0).clamp(0.0, 0.05);
                }
            }
        }
    }

    /// UTC (ns) of sample `k`, or None before the clock has locked.
    pub fn stamp(&self, k: u64) -> Option<i64> {
        let g = self.g.lock().ok()?;
        let (t0, corr) = (*g)?;
        Some((t0 + (k as f64 * 1e9 / FS) as i128 + corr as i128) as i64)
    }
}

/// Writes an RTP header with the SFN extension: 12 bytes + 4 extension header + 8 bytes time stamp.
pub fn rtp_header_sfn(out: &mut Vec<u8>, pt: u8, seq: u16, ts: u32, ssrc: u32, stamp: i64) {
    out.extend_from_slice(&[0x90, pt]); // version 2, X bit
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&ts.to_be_bytes());
    out.extend_from_slice(&ssrc.to_be_bytes());
    out.extend_from_slice(&EXT_PROFILE.to_be_bytes());
    out.extend_from_slice(&2u16.to_be_bytes());
    out.extend_from_slice(&stamp.to_be_bytes());
}

/// The SFN time stamp of an RTP packet, if it has one.
pub fn rtp_stamp(pkt: &[u8]) -> Option<i64> {
    crate::mpxflac::sfn_stamp(pkt)
}

// ================================================================ receiver: timed playout

/// Settings of the receiver's SFN playout.
#[derive(Clone, Debug)]
pub struct SfnCfg {
    /// Time from the stamp to the moment on air (the same at all sites), ms.
    pub delay_ms: f64,
    /// Correction of this site's output path (sound card, cables), µs; positive plays later.
    pub correction_us: f64,
}

/// What the window shows about the playout.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SfnStatus {
    pub active: bool,
    /// "Waiting for time stamps", "Waiting for the clock", "Locked", "Late: increase the delay" ...
    pub state: String,
    pub locked: bool,
    /// Playout timing error now (filtered), µs: positive = playing too early.
    pub error_us: f64,
    /// Resampling ratio deviation, ppm.
    pub ppm: f64,
    /// How early the packets arrive before their playout time (smallest value of the last seconds), ms.
    pub margin_ms: f64,
    /// Number of times the playout position was set anew (start, gaps, too late).
    pub resyncs: u64,
    /// Spread of the transmitter's time stamps around a straight line, µs.
    pub stamp_jitter_us: f64,
}

/// Windowed-sinc fractional interpolator (32 taps, 512 phases): keeps the 57 kHz RDS and 38 kHz stereo intact
/// while the playout rate is trimmed by a few ppm.
struct Interp {
    table: Vec<[f32; 32]>,
}

impl Interp {
    fn new() -> Interp {
        const P: usize = 512;
        let mut table = vec![[0f32; 32]; P + 1];
        for (p, row) in table.iter_mut().enumerate() {
            let mu = p as f64 / P as f64;
            let mut sum = 0.0;
            for (j, v) in row.iter_mut().enumerate() {
                let x = j as f64 - 15.0 - mu; // taps at -15..16 around the position
                let fc = 0.46; // cut-off relative to the sample rate: 88 kHz, flat to 60 kHz
                let s = if x.abs() < 1e-9 { 2.0 * fc } else { (2.0 * std::f64::consts::PI * fc * x).sin() / (std::f64::consts::PI * x) };
                let n = (x + 16.0) / 32.0; // 0..1 across the window
                let w = 0.42 - 0.5 * (2.0 * std::f64::consts::PI * n).cos() + 0.08 * (4.0 * std::f64::consts::PI * n).cos();
                *v = (s * w) as f32;
                sum += s * w;
            }
            for v in row.iter_mut() {
                *v = (*v as f64 / sum) as f32;
            }
        }
        Interp { table }
    }

    /// Value at position `i + mu` (0 <= mu < 1) from `d`, where `d[k]` is sample k; needs d[i-15 ..= i+16].
    #[inline]
    fn at(&self, d: &VecDeque<f32>, i: usize, mu: f64) -> f32 {
        let pf = mu * 512.0;
        let p = (pf as usize).min(511);
        let f = (pf - p as f64) as f32;
        let (a, b) = (&self.table[p], &self.table[p + 1]);
        let mut acc = 0f32;
        for j in 0..32 {
            let s = d[i + j - 15];
            acc += s * (a[j] + (b[j] - a[j]) * f);
        }
        acc
    }
}

struct Buf {
    data: VecDeque<f32>,
    /// Sample number of data[0].
    base: u64,
    /// (sample number, UTC ns) of recent packets.
    anchors: VecDeque<(u64, i64)>,
    // playout
    pos: Option<f64>,
    ratio_adj: f64,
    integ: f64,
    err_f: f64,
    min_margin: f64,
    margin_since: std::time::Instant,
    /// Blocks since the last (re)start.
    settle: u32,
}

pub struct SfnPlayout {
    cfg: SfnCfg,
    pub clock: Arc<RefClock>,
    b: Mutex<Buf>,
    interp: Interp,
    pub status: Mutex<SfnStatus>,
    cap: usize,
}

impl SfnPlayout {
    pub fn new(cfg: SfnCfg, clock: Arc<RefClock>) -> Arc<SfnPlayout> {
        let cap = ((cfg.delay_ms / 1000.0 + 2.0) * FS) as usize;
        Arc::new(SfnPlayout {
            cfg,
            clock,
            b: Mutex::new(Buf { data: VecDeque::with_capacity(cap), base: 0, anchors: VecDeque::new(), pos: None, ratio_adj: 0.0, integ: 0.0, err_f: 0.0, min_margin: f64::MAX, margin_since: std::time::Instant::now(), settle: 0 }),
            interp: Interp::new(),
            status: Mutex::new(SfnStatus { active: true, state: "Waiting for time stamps".into(), ..SfnStatus::default() }),
            cap,
        })
    }

    /// Adds received samples. `stamp`: (position of a packet's first sample inside `samples`, its UTC time).
    pub fn push(&self, samples: &[f32], stamp: Option<(usize, i64)>) {
        let Ok(mut b) = self.b.lock() else { return };
        let first = b.base + b.data.len() as u64;
        if let Some((at, s)) = stamp {
            b.anchors.push_back((first + at as u64, s));
            while b.anchors.len() > 64 {
                b.anchors.pop_front();
            }
            // arrival margin: how much earlier than its playout time this packet is here
            if let Some(now) = self.clock.now_ns() {
                let margin = (s as i128 + (self.cfg.delay_ms * 1e6) as i128 - now) as f64 / 1e6;
                if b.margin_since.elapsed().as_secs_f64() > 5.0 {
                    b.min_margin = margin;
                    b.margin_since = std::time::Instant::now();
                } else {
                    b.min_margin = b.min_margin.min(margin);
                }
                let m = b.min_margin;
                if let Ok(mut st) = self.status.lock() {
                    st.margin_ms = m;
                }
            }
        }
        b.data.extend(samples.iter().copied());
        // keep the buffer bounded (only happens when nothing is played)
        let over = b.data.len().saturating_sub(self.cap);
        if over > 0 {
            b.data.drain(..over);
            b.base += over as u64;
        }
    }

    /// Spread of the recent time stamps around a straight line (stamp jitter), µs: shows how evenly the
    /// transmitter's stamps arrive in the samples.
    pub fn stamp_jitter_us(&self) -> f64 {
        let Ok(b) = self.b.lock() else { return 0.0 };
        let a = &b.anchors;
        if a.len() < 4 {
            return 0.0;
        }
        let (k0, s0) = a[0];
        let (k1, s1) = a[a.len() - 1];
        if k1 <= k0 {
            return 0.0;
        }
        let slope = (s1 - s0) as f64 / (k1 - k0) as f64;
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for &(k, s) in a.iter() {
            let r = (s - s0) as f64 - slope * (k - k0) as f64;
            lo = lo.min(r);
            hi = hi.max(r);
        }
        (hi - lo) / 2000.0
    }

    /// Audio waiting before playout, ms.
    pub fn ms(&self) -> u32 {
        self.b.lock().map(|b| (b.data.len() as f64 * 1000.0 / FS) as u32).unwrap_or(0)
    }

    fn set_state(&self, f: impl FnOnce(&mut SfnStatus)) {
        if let Ok(mut s) = self.status.lock() {
            f(&mut s);
        }
    }

    /// Fills one output block. `latency_ns`: time from now until the first sample of this block is heard (from the
    /// sound driver). Returns the samples (mono, 192 kHz) in `out`.
    pub fn render(&self, out: &mut [f32], latency_ns: i128) {
        let n = out.len();
        let Ok(mut b) = self.b.lock() else {
            out.fill(0.0);
            return;
        };
        let Some(now) = self.clock.now_ns() else {
            out.fill(0.0);
            b.pos = None;
            drop(b);
            self.set_state(|s| {
                s.locked = false;
                s.state = "Waiting for the reference clock".into();
            });
            return;
        };
        let Some(&(ka, sa)) = b.anchors.back() else {
            out.fill(0.0);
            drop(b);
            self.set_state(|s| {
                s.locked = false;
                s.state = "Waiting for time stamps (SFN must be switched on in the Transmitter)".into();
            });
            return;
        };
        // the sample that must be heard at the time this block starts playing
        let t_play = now + latency_ns - (self.cfg.correction_us * 1000.0) as i128;
        let target = ka as f64 + (t_play - sa as i128 - (self.cfg.delay_ms * 1e6) as i128) as f64 * FS / 1e9;
        let lo = b.base as f64 + 16.0;
        let hi = (b.base + b.data.len() as u64) as f64 - 17.0 - n as f64 * 1.001;
        // (re)start when not running, or after a real jump (more than 20 ms off): single callbacks are often a few
        // ms early or late, which the filtered control below absorbs
        let resync = match b.pos {
            None => true,
            Some(p) => (p - target).abs() > 3840.0,
        };
        if resync {
            if target < lo {
                // the samples for now are gone: the delay is shorter than the way through the network
                out.fill(0.0);
                b.pos = None;
                drop(b);
                self.set_state(|s| {
                    s.locked = false;
                    s.state = "Late: the packets arrive after their playout time - increase the delay".into();
                });
                return;
            }
            if target > hi {
                // not yet time (normal at the start)
                out.fill(0.0);
                b.pos = None;
                drop(b);
                self.set_state(|s| {
                    s.locked = false;
                    s.state = "Waiting for the playout time".into();
                });
                return;
            }
            b.pos = Some(target);
            b.integ = 0.0;
            b.err_f = 0.0;
            b.ratio_adj = 0.0;
            b.settle = 0;
            drop(b);
            self.set_state(|s| s.resyncs += 1);
            b = match self.b.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
        }
        // fine control: error in samples, filtered over about 50 blocks (0.5 s); PI loop of about 4 s on the rate
        let pos = b.pos.unwrap_or(target);
        let err = pos - target; // > 0: ahead of time (playing too early)
        b.err_f += (err - b.err_f) * 0.02;
        b.settle = b.settle.saturating_add(1);
        let ef = b.err_f;
        b.integ = (b.integ + ef * 1.5e-9).clamp(-3e-4, 3e-4);
        b.ratio_adj = (-(ef * 1.3e-6) - b.integ).clamp(-3e-4, 3e-4);
        let ratio = 1.0 + b.ratio_adj;
        let end = (b.base + b.data.len() as u64) as f64;
        let mut p = pos;
        let missing = p + n as f64 * ratio + 17.0 > end;
        for o in out.iter_mut() {
            // a gap in the data (network): silence for the missing samples, the timing goes on
            *o = if p.floor() as u64 >= b.base + 15 && p + 17.0 <= end {
                let i = (p.floor() as u64 - b.base) as usize;
                self.interp.at(&b.data, i, p - p.floor())
            } else {
                0.0
            };
            p += ratio;
        }
        b.pos = Some(p);
        // drop what has been played (keep the interpolator's history)
        let keep_from = (p.floor() as u64).saturating_sub(64);
        if keep_from > b.base {
            let d = ((keep_from - b.base) as usize).min(b.data.len());
            b.data.drain(..d);
            b.base += d as u64;
        }
        while b.anchors.len() > 2 && b.anchors[0].0 < b.base {
            b.anchors.pop_front();
        }
        let (e_us, ppm, settled) = (ef / FS * 1e6, b.ratio_adj * 1e6, b.settle > 300);
        drop(b);
        self.set_state(|s| {
            s.error_us = e_us;
            s.ppm = ppm;
            s.locked = settled && e_us.abs() < 50.0;
            s.state = if missing {
                "Data missing (network gap)".into()
            } else if s.locked {
                "Locked to the reference time".into()
            } else {
                "Locking ...".into()
            };
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{ClockCfg, RefClock};

    #[test]
    fn rtp_extension_roundtrip() {
        let mut p = Vec::new();
        rtp_header_sfn(&mut p, 98, 7, 1234, 99, 1_793_065_812_123_456_789);
        p.extend_from_slice(&[1, 2, 3]);
        assert_eq!(rtp_stamp(&p), Some(1_793_065_812_123_456_789));
        assert_eq!(p.len(), 12 + 12 + 3);
        let plain = [0x80u8, 98, 0, 7, 0, 0, 4, 210, 0, 0, 0, 99, 1, 2, 3];
        assert_eq!(rtp_stamp(&plain), None);
    }

    #[test]
    fn interpolator_is_flat_to_60_khz() {
        let it = Interp::new();
        for f in [1_000.0, 19_000.0, 38_000.0, 57_000.0, 60_000.0] {
            let d: VecDeque<f32> = (0..2000).map(|k| (2.0 * std::f64::consts::PI * f * k as f64 / FS).sin() as f32).collect();
            let mut worst = 0f64;
            for k in 0..400 {
                let pos = 500.0 + k as f64 * 0.731;
                let i = pos.floor() as usize;
                let v = it.at(&d, i, pos - pos.floor()) as f64;
                let want = (2.0 * std::f64::consts::PI * f * pos / FS).sin();
                worst = worst.max((v - want).abs());
            }
            assert!(worst < 2e-3, "{f} Hz: error {worst}");
        }
    }

    /// Two receivers with different sound card clocks (+40 ppm and -25 ppm) and different network delays play the
    /// same stream: both must lock to the stamps, i.e. play the same sample at the same time.
    #[test]
    fn playout_locks_despite_card_clock_error() {
        let clock = RefClock::start(&ClockCfg::default());
        for (ppm, net_ms) in [(40.0f64, 30.0f64), (-25.0, 120.0)] {
            let pl = SfnPlayout::new(SfnCfg { delay_ms: 300.0, correction_us: 0.0 }, clock.clone());
            // simulate: stream time and card time advance in steps; we drive render() with a fake latency so that
            // its "now" corresponds to the simulated card time
            let mut fed: u64 = 0;
            let block = 1920usize; // 10 ms
            let mut out = vec![0f32; block];
            let mut card_t: f64 = 0.0; // seconds since start, by the card's clock
            let start = clock.now_ns().unwrap() as i64; // the stream starts now
            let real0 = start as f64 / 1e9;
            let mut last_err = f64::MAX;
            for step in 0..4000 {
                // feed samples that "arrived" by now (network delay)
                let t_sim = step as f64 * 0.01;
                while (fed as f64 / FS) < t_sim - net_ms / 1000.0 + 0.01 {
                    let pk: Vec<f32> = (0..384).map(|j| ((fed + j) as f32 * 0.001).sin()).collect();
                    pl.push(&pk, Some((0, start + (fed as f64 * 1e9 / FS) as i64)));
                    fed += 384;
                }
                // the card asks for a block every 10 ms of its own clock; express the simulated time as latency
                card_t += block as f64 / FS / (1.0 + ppm * 1e-6);
                let real_now = clock.now_ns().unwrap() as f64 / 1e9 - real0;
                let latency = ((card_t - real_now) * 1e9) as i128;
                pl.render(&mut out, latency);
                last_err = pl.status.lock().unwrap().error_us;
            }
            let st = pl.status.lock().unwrap().clone();
            eprintln!("card {ppm:+} ppm, network {net_ms} ms: {st:?}");
            assert!(st.locked, "{st:?}");
            assert!(last_err.abs() < 20.0);
            assert!((st.ppm + ppm).abs() < 8.0, "rate follows the card error: {}", st.ppm);
        }
    }
}
