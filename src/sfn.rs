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
    /// How unevenly the sound card's callbacks come (RMS, µs); the playout averages it out.
    pub card_jitter_us: f64,
    /// Spread of the transmitter's time stamps around a straight line, µs.
    pub stamp_jitter_us: f64,
    /// Steps of the reference clock (e.g. the system clock set by the time service), and the last one, ms.
    pub clock_steps: u64,
    pub last_clock_step_ms: f64,
    /// Why the playout started anew the last time.
    pub last_restart: String,
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
    /// (unused, kept for the layout)
    dll: Option<(i128, f64, f64)>,
    dll_age: u32,
    /// Spread of the timing error around its average (the callback jitter), samples².
    dll_jitter: f64,
    /// The shown (averaged) error, samples, and rate.
    err_show: f64,
    rate_show: f64,
    /// Blocks in a row far off the target (a busy computer: callbacks late), and single outliers skipped.
    far: u32,
    /// The reference clock against the monotonic clock: a jump of this offset is a step of the clock.
    mono: Option<(std::time::Instant, i128, f64)>,
    /// Blocks in a row with the shown error beyond the unlock limit (then the loop is made fast again).
    off_blocks: u32,
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
            b: Mutex::new(Buf { data: VecDeque::with_capacity(cap), base: 0, anchors: VecDeque::new(), pos: None, ratio_adj: 0.0, integ: 0.0, err_f: 0.0, min_margin: f64::MAX, margin_since: std::time::Instant::now(), settle: 0, dll: None, dll_age: 0, dll_jitter: 0.0, err_show: 0.0, rate_show: 0.0, far: 0, mono: None, off_blocks: 0 }),
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
        // steps of the reference clock (a time service setting the system clock): the playout follows them, but they
        // are counted and shown, as they move this site against the others
        let mut step_ms = None;
        match b.mono {
            None => b.mono = Some((std::time::Instant::now(), now, 0.0)),
            Some((i0, n0, last)) => {
                let off = (now - n0) as f64 - i0.elapsed().as_nanos() as f64;
                if (off - last).abs() > 1e6 {
                    step_ms = Some((off - last) / 1e6);
                }
                b.mono = Some((i0, n0, off));
            }
        }
        if let Some(ms) = step_ms {
            self.set_state(|s| {
                s.clock_steps += 1;
                s.last_clock_step_ms = ms;
            });
        }
        let Some(&(ka, sa)) = b.anchors.back() else {
            out.fill(0.0);
            drop(b);
            self.set_state(|s| {
                s.locked = false;
                s.state = "Waiting for time stamps (SFN must be switched on in the Transmitter)".into();
            });
            return;
        };
        // when this block is really heard (the callback time jitters; the control loop below averages it out)
        let heard = now + latency_ns;
        // the sample that must be heard at the time this block starts playing
        let t_play = heard - (self.cfg.correction_us * 1000.0) as i128;
        let target = ka as f64 + (t_play - sa as i128 - (self.cfg.delay_ms * 1e6) as i128) as f64 * FS / 1e9;
        let lo = b.base as f64 + 16.0;
        let hi = (b.base + b.data.len() as u64) as f64 - 17.0 - n as f64 * 1.001;
        // (re)start when not running, or after a real jump (more than 20 ms off): single callbacks are often a few
        // ms early or late, which the filtered control below absorbs
        // one late or early callback (the computer was busy) must not move the playout: start anew only when the
        // position stays more than 20 ms off for a quarter of a second
        let resync = match b.pos.map(|p| (p - target).abs() > 3840.0) {
            None => true,
            Some(true) => {
                b.far += 1;
                b.far * n as u32 > FS as u32 / 4
            }
            Some(false) => {
                b.far = 0;
                false
            }
        };
        if resync && std::env::var_os("RM_SFN_DEBUG").is_some() {
            let tail: Vec<(u64, i64)> = b.anchors.iter().rev().take(4).copied().collect();
            eprintln!("SFN resync: pos {:?} target {target:.0} diff {:?} anchors {:?} base {} len {}", b.pos, b.pos.map(|p| p - target), tail, b.base, b.data.len());
        }
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
            let why = b.pos.map(|p| format!("the playout was {:+.1} ms off its time", (p - target) / FS * 1e3));
            b.pos = Some(target);
            b.far = 0;
            b.off_blocks = 0;
            b.integ = 0.0;
            b.err_f = 0.0;
            b.ratio_adj = 0.0;
            b.settle = 0;
            b.err_show = 0.0;
            b.rate_show = 0.0;
            drop(b);
            self.set_state(|s| {
                s.resyncs += 1;
                if let Some(w) = why {
                    s.last_restart = w;
                }
            });
            b = match self.b.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
        }
        // fine control: a second-order loop (PI on the resampling rate) on the timing error. Its bandwidth narrows
        // step by step: fast to lock (0.5 Hz), then slow to average out the jitter of the card's callbacks
        // (0.006 Hz at the end: ±1 ms of callback jitter gives about ±20 µs). A short pre-filter takes the
        // block-to-block noise off the rate.
        let pos = b.pos.unwrap_or(target);
        let err = pos - target; // > 0: ahead of time (playing too early)
        b.settle = b.settle.saturating_add(1);
        let bt = n as f64 / FS; // block length, s
        let age_s = b.settle as f64 * bt;
        let fnat = if age_s < 3.0 { 0.5 } else if age_s < 15.0 { 0.1 } else if age_s < 45.0 { 0.02 } else { 0.006 };
        // a measurement far off the others (a callback delayed by a busy computer) is not used
        let outlier = b.far > 0 || (age_s > 3.0 && (err - b.err_show).abs() > (6.0 * b.dll_jitter.sqrt()).max(FS * 0.002));
        if !outlier {
            let pre = (bt * 2.0 * std::f64::consts::PI * 10.0 * fnat).min(1.0);
            b.err_f += (err - b.err_f) * pre;
            let ef = b.err_f;
            let w = 2.0 * std::f64::consts::PI * fnat * bt; // per block
            let (kp, ki) = (2.0 * 0.707 * w / n as f64, w * w / n as f64);
            b.integ = (b.integ + ef * ki).clamp(-3e-4, 3e-4);
            b.ratio_adj = (-(ef * kp) - b.integ).clamp(-3e-4, 3e-4);
            // what the window shows: the error averaged over 1 s (10 s once the loop is slow: the callback jitter
            // is averaged out the same way), the rate over 2 s, the callback jitter
            let a3 = (bt / if age_s < 15.0 { 1.0 } else { 10.0 }).min(1.0);
            b.err_show += (err - b.err_show) * a3;
            b.dll_jitter += ((err - b.err_show).powi(2) - b.dll_jitter) * (bt / 3.0).min(1.0);
        }
        b.rate_show += (b.ratio_adj - b.rate_show) * (bt / 2.0).min(1.0);
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
        let (e_us, ppm, settled, jit) = (b.err_show / FS * 1e6, b.rate_show * 1e6, age_s > 3.0, b.dll_jitter.sqrt() / FS * 1e6);
        // how exactly the shown error can be measured with this callback jitter (its averaging time)
        let noise_us = jit * (bt / if age_s < 15.0 { 1.0 } else { 10.0 }).sqrt();
        let (lock_at, unlock_at) = ((3.0 * noise_us).max(50.0), (6.0 * noise_us).max(200.0));
        // far off for 2 s although the loop is slow (a step of the clock or of the card's latency): make the loop fast
        // again, so it is back in a few seconds instead of minutes (no restart: the sound goes on)
        if age_s > 15.0 && e_us.abs() > unlock_at {
            b.off_blocks += 1;
            if b.off_blocks as f64 * bt > 2.0 {
                b.settle = (3.0 / bt) as u32; // back to 0.1 Hz, the error shown over 1 s
                b.off_blocks = 0;
            }
        } else {
            b.off_blocks = 0;
        }
        drop(b);
        self.set_state(|s| {
            s.error_us = e_us;
            s.ppm = ppm;
            s.card_jitter_us = jit;
            // locked within 50 µs (or within the measuring accuracy, when the callbacks jitter a lot), unlocked again
            // only beyond 200 µs: no flicker around the limit
            s.locked = settled && if s.locked { e_us.abs() < unlock_at } else { e_us.abs() < lock_at };
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
    /// A real sound card calls back with a jitter of up to a millisecond or more (WASAPI, PipeWire, PulseAudio,
    /// USB): the playout must lock within seconds, never unlock again, and follow the card's rate without swinging.
    #[test]
    fn playout_stays_locked_with_callback_jitter() {
        let clock = RefClock::start(&ClockCfg::default());
        // (block size, callback jitter ± ms, card clock error ppm)
        // (the first runs 12 minutes: longer than the 5 minutes after which 1.12.2 could drift)
        for (block, jit_ms, ppm, mins) in [(1920usize, 1.0f64, 30.0f64, 12.0f64), (1024, 1.0, 30.0, 3.0), (1920, 2.0, -45.0, 3.0), (480, 0.5, 10.0, 3.0)] {
            let pl = SfnPlayout::new(SfnCfg { delay_ms: 300.0, correction_us: 0.0 }, clock.clone());
            let mut out = vec![0f32; block];
            let (mut fed, mut card_t) = (0u64, 0.0f64);
            let start = clock.now_ns().unwrap() as i64;
            let real0 = start as f64 / 1e9;
            let mut x: u64 = 12345;
            let (mut first_lock, mut unlocks, mut was_locked, mut max_err, mut max_ppm_dev) = (None, 0, false, 0f64, 0f64);
            let steps = (mins * 60.0 * FS / block as f64) as usize;
            for step in 0..steps {
                let t_sim = step as f64 * block as f64 / FS;
                while (fed as f64 / FS) < t_sim - 0.05 + 0.01 {
                    let pk: Vec<f32> = (0..384).map(|j| ((fed + j) as f32 * 0.001).sin()).collect();
                    pl.push(&pk, Some((0, start + (fed as f64 * 1e9 / FS) as i64)));
                    fed += 384;
                }
                card_t += block as f64 / FS / (1.0 + ppm * 1e-6);
                let real_now = clock.now_ns().unwrap() as f64 / 1e9 - real0;
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let jitter = ((x % 20001) as f64 / 10000.0 - 1.0) * jit_ms * 1e-3;
                let latency = ((card_t - real_now + jitter) * 1e9) as i128;
                pl.render(&mut out, latency);
                let st = pl.status.lock().unwrap().clone();
                if st.locked && first_lock.is_none() {
                    first_lock = Some(t_sim);
                }
                if was_locked && !st.locked {
                    unlocks += 1;
                }
                was_locked = st.locked;
                if t_sim > 60.0 {
                    max_err = max_err.max(st.error_us.abs());
                    max_ppm_dev = max_ppm_dev.max((st.ppm + ppm).abs());
                }
            }
            let st = pl.status.lock().unwrap().clone();
            eprintln!("{mins} min, block {block}, jitter ±{jit_ms} ms, card {ppm:+} ppm: locked after {first_lock:?} s, unlocks {unlocks}, max error {max_err:.1} µs, rate off by up to {max_ppm_dev:.2} ppm, jitter {:.0} µs", st.card_jitter_us);
            assert!(first_lock.map(|t| t < 10.0).unwrap_or(false), "lock time {first_lock:?}");
            assert_eq!(unlocks, 0);
            assert_eq!(st.resyncs, 1, "no restarts");
            // the shown error can only be as exact as the callback jitter allows (averaged over 10 s)
            // (callback jitter RMS / sqrt(callbacks in 10 s); the largest value over minutes is about 4 times that)
            let noise = jit_ms * 1000.0 / 3f64.sqrt() * (block as f64 / FS / 10.0).sqrt();
            assert!(max_err < 4.0 * noise + 10.0, "{max_err} (noise {noise:.1})");
            assert!(max_ppm_dev < 4.0 * jit_ms + 1.0, "rate swings: {max_ppm_dev}");
        }
    }

    /// A busy computer: now and then a callback comes 15 - 40 ms late (its measured time jumps). The playout must
    /// neither restart nor unlock.
    #[test]
    fn playout_ignores_late_callbacks() {
        let clock = RefClock::start(&ClockCfg::default());
        let pl = SfnPlayout::new(SfnCfg { delay_ms: 300.0, correction_us: 0.0 }, clock.clone());
        let block = 1024usize;
        let mut out = vec![0f32; block];
        let (mut fed, mut card_t) = (0u64, 0.0f64);
        let start = clock.now_ns().unwrap() as i64;
        let real0 = start as f64 / 1e9;
        let (mut unlocks, mut was_locked) = (0, false);
        let steps = (120.0 * FS / block as f64) as usize;
        for step in 0..steps {
            let t_sim = step as f64 * block as f64 / FS;
            while (fed as f64 / FS) < t_sim - 0.05 + 0.01 {
                let pk: Vec<f32> = (0..384).map(|j| ((fed + j) as f32 * 0.001).sin()).collect();
                pl.push(&pk, Some((0, start + (fed as f64 * 1e9 / FS) as i64)));
                fed += 384;
            }
            card_t += block as f64 / FS;
            let real_now = clock.now_ns().unwrap() as f64 / 1e9 - real0;
            // every ~7 s two callbacks 40 ms and 15 ms late
            let late = match step % 1300 { 700 => 0.040, 701 => 0.015, _ => 0.0 };
            let latency = ((card_t - real_now + late) * 1e9) as i128;
            pl.render(&mut out, latency);
            let st = pl.status.lock().unwrap().clone();
            if t_sim > 10.0 && was_locked && !st.locked {
                unlocks += 1;
            }
            was_locked = st.locked;
        }
        let st = pl.status.lock().unwrap().clone();
        eprintln!("late callbacks: {st:?}, unlocks {unlocks}");
        assert_eq!(st.resyncs, 1, "restarted on a late callback");
        assert_eq!(unlocks, 0);
        assert!(st.locked && st.error_us.abs() < 10.0);
    }

    /// A step of 3 ms (the system clock set by a time service, or the card's latency changing): the playout follows
    /// it without a restart and is locked again within 15 s.
    #[test]
    fn playout_follows_a_step() {
        let clock = RefClock::start(&ClockCfg::default());
        let pl = SfnPlayout::new(SfnCfg { delay_ms: 300.0, correction_us: 0.0 }, clock.clone());
        let block = 1024usize;
        let mut out = vec![0f32; block];
        let (mut fed, mut card_t) = (0u64, 0.0f64);
        let start = clock.now_ns().unwrap() as i64;
        let real0 = start as f64 / 1e9;
        let mut x: u64 = 777;
        let mut relock: Option<f64> = None;
        let steps = (150.0 * FS / block as f64) as usize;
        for step in 0..steps {
            let t_sim = step as f64 * block as f64 / FS;
            while (fed as f64 / FS) < t_sim - 0.05 + 0.01 {
                let pk: Vec<f32> = (0..384).map(|j| ((fed + j) as f32 * 0.001).sin()).collect();
                pl.push(&pk, Some((0, start + (fed as f64 * 1e9 / FS) as i64)));
                fed += 384;
            }
            card_t += block as f64 / FS / (1.0 + 20e-6);
            let real_now = clock.now_ns().unwrap() as f64 / 1e9 - real0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let jitter = ((x % 2001) as f64 - 1000.0) * 1e-6;
            let shift = if t_sim > 80.0 { 0.003 } else { 0.0 };
            pl.render(&mut out, ((card_t - real_now + jitter + shift) * 1e9) as i128);
            let st = pl.status.lock().unwrap().clone();
            if t_sim > 82.0 && relock.is_none() && st.locked && st.error_us.abs() < 60.0 {
                relock = Some(t_sim - 80.0);
            }
        }
        let st = pl.status.lock().unwrap().clone();
        eprintln!("3 ms step: locked again after {relock:?} s, {st:?}");
        assert_eq!(st.resyncs, 1, "no restart");
        assert!(relock.map(|t| t < 15.0).unwrap_or(false), "{relock:?}");
        assert!(st.locked && st.error_us.abs() < 60.0);
    }

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
