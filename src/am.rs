//! AM mode: the audio for an AM (medium wave / long wave / short wave) transmitter instead of the FM multiplex.
//! An AM exciter takes plain mono audio. The Transmitter makes it from the same processed audio as the FM outputs:
//!   - mono (L + R) / 2,
//!   - the FM pre-emphasis taken out again (the shared processing pre-emphasises for FM),
//!   - a high-pass at 50 Hz (no DC or rumble on the carrier),
//!   - a steep low-pass (8th-order Butterworth) at the channel's audio bandwidth: 4.5 kHz (9 kHz channels, Europe /
//!     Asia / Africa), 5 kHz, 9 kHz or 10 kHz (NRSC, 10 kHz channels, America),
//!   - an asymmetric peak limiter: negative peaks never below -100 % modulation (the carrier would cut off and
//!     splatter), positive peaks up to the set limit (usually +125 %),
//!   - a final clipper at the same limits.
//! The result is sent as Compact audio (FLAC or Opus); the Receiver plays it on its sound card for the exciter's
//! audio input. Output scale: -100 % modulation = -0.8 of full scale, +125 % = +1.0 (the exciter's input level is set
//! once, as with any audio source).
use std::f64::consts::PI;

/// Full-scale value of -100 % modulation: leaves room for the positive peaks up to +125 % below 1.0.
pub const NEG_FS: f32 = 0.8;

/// Settings of one AM output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmCfg {
    /// Audio bandwidth (low-pass), Hz.
    pub bandwidth_hz: f32,
    /// Positive peak limit, % modulation (100 - 150; 125 is usual).
    pub pos_peak: f32,
    /// The FM pre-emphasis in the audio that comes in, µs (0 = none), taken out again.
    pub fm_preemph_us: f32,
}

impl Default for AmCfg {
    fn default() -> Self {
        AmCfg { bandwidth_hz: 4500.0, pos_peak: 125.0, fm_preemph_us: 50.0 }
    }
}

/// Second-order section (transposed direct form II).
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    fn new(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Biquad {
        Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, z1: 0.0, z2: 0.0 }
    }
    /// Low-pass with quality factor q (RBJ cookbook).
    fn lowpass(f: f64, q: f64, fs: f64) -> Biquad {
        let w = 2.0 * PI * f / fs;
        let (c, a) = (w.cos(), w.sin() / (2.0 * q));
        Biquad::new((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }
    fn highpass(f: f64, q: f64, fs: f64) -> Biquad {
        let w = 2.0 * PI * f / fs;
        let (c, a) = (w.cos(), w.sin() / (2.0 * q));
        Biquad::new((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + a, -2.0 * c, 1.0 - a)
    }
    #[inline]
    fn push(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// First-order de-emphasis 1 / (1 + s*tau) (bilinear), the inverse of the FM pre-emphasis.
#[derive(Clone, Copy, Debug)]
struct DeEmph {
    b: f64,
    a1: f64,
    x1: f64,
    y1: f64,
    on: bool,
}

impl DeEmph {
    fn new(tau_us: f32, fs: f64) -> DeEmph {
        if tau_us <= 0.0 {
            return DeEmph { b: 1.0, a1: 0.0, x1: 0.0, y1: 0.0, on: false };
        }
        let k = 2.0 * fs * tau_us as f64 * 1e-6;
        DeEmph { b: 1.0 / (1.0 + k), a1: (1.0 - k) / (1.0 + k), x1: 0.0, y1: 0.0, on: true }
    }
    #[inline]
    fn push(&mut self, x: f64) -> f64 {
        if !self.on {
            return x;
        }
        let y = self.b * (x + self.x1) - self.a1 * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// The AM processing at `fs` (48 kHz in the Transmitter).
pub struct AmProcessor {
    pub cfg: AmCfg,
    deemph: DeEmph,
    hp: [Biquad; 1],
    lp: [Biquad; 4],
    /// Gain of the limiter (1 = no limiting) and its release per sample.
    gain: f64,
    release: f64,
    /// Program peak of the last block, for the window.
    pub peak_pos: f32,
    pub peak_neg: f32,
}

impl AmProcessor {
    pub fn new(cfg: AmCfg, fs: u32) -> AmProcessor {
        let fs = fs as f64;
        let bw = (cfg.bandwidth_hz as f64).clamp(2000.0, fs * 0.45);
        // 8th-order Butterworth: Q of the four sections
        let qs = [0.50979558, 0.60134489, 0.89997622, 2.5629154];
        AmProcessor {
            cfg,
            deemph: DeEmph::new(cfg.fm_preemph_us, fs),
            hp: [Biquad::highpass(50.0, std::f64::consts::FRAC_1_SQRT_2, fs)],
            lp: qs.map(|q| Biquad::lowpass(bw, q, fs)),
            gain: 1.0,
            // back to full gain in about 300 ms
            release: 1.0 - (-1.0 / (0.3 * fs)).exp(),
            peak_pos: 0.0,
            peak_neg: 0.0,
        }
    }

    /// One stereo sample (the shared processed audio, ±1) -> the AM audio, scaled so -100 % = -NEG_FS.
    #[inline]
    pub fn push(&mut self, l: f32, r: f32) -> f32 {
        let mut x = (l as f64 + r as f64) * 0.5;
        x = self.deemph.push(x);
        for s in self.hp.iter_mut() {
            x = s.push(x);
        }
        for s in self.lp.iter_mut() {
            x = s.push(x);
        }
        // the program at full level reaches ±1 (= 100 % modulation)
        let pos = (self.cfg.pos_peak.clamp(100.0, 150.0) / 100.0) as f64;
        let limit = if x >= 0.0 { pos } else { 1.0 };
        let need = if x.abs() * self.gain > limit { limit / x.abs() } else { 1.0 };
        if need < self.gain {
            self.gain = need;
        } else {
            self.gain += (1.0 - self.gain) * self.release;
        }
        let y = (x * self.gain).clamp(-1.0, pos);
        let out = (y * NEG_FS as f64) as f32;
        self.peak_pos = self.peak_pos.max(out);
        self.peak_neg = self.peak_neg.min(out);
        out
    }
}

/// The usual audio bandwidths: (Hz, label).
pub const BANDWIDTHS: [(u32, &str); 4] = [(4500, "4.5 kHz (9 kHz channels: Europe, Africa, Asia)"), (5000, "5 kHz"), (9000, "9 kHz"), (10000, "10 kHz (NRSC, 10 kHz channels: America)")];

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(p: &mut AmProcessor, f: f64, amp: f32, n: usize) -> (f32, f32) {
        let mut lo = 0f32;
        let mut hi = 0f32;
        for i in 0..n {
            let v = amp * (2.0 * PI * f * i as f64 / 48_000.0).sin() as f32;
            let y = p.push(v, v);
            if i > n / 2 {
                lo = lo.min(y);
                hi = hi.max(y);
            }
        }
        (lo, hi)
    }

    #[test]
    fn negative_peaks_never_below_minus_100_percent() {
        // a loud, asymmetric signal (a sawtooth with a big negative swing)
        let mut p = AmProcessor::new(AmCfg { fm_preemph_us: 0.0, ..Default::default() }, 48_000);
        let mut lo = 0f32;
        let mut hi = 0f32;
        for i in 0..96_000 {
            let ph = (i % 160) as f32 / 160.0;
            let v = 3.0 * (0.3 - ph);
            let y = p.push(v, v);
            lo = lo.min(y);
            hi = hi.max(y);
        }
        assert!(lo >= -NEG_FS - 1e-6, "lowest {lo}");
        assert!(hi <= NEG_FS * 1.25 + 1e-6, "highest {hi}");
    }

    #[test]
    fn bandwidth_is_kept() {
        let cfg = AmCfg { bandwidth_hz: 4500.0, pos_peak: 125.0, fm_preemph_us: 0.0 };
        // 1 kHz passes at its level (well below the limiter), 9 kHz is gone (more than 40 dB down)
        let (lo, hi) = tone(&mut AmProcessor::new(cfg, 48_000), 1000.0, 0.3, 48_000);
        assert!((hi - 0.3 * NEG_FS).abs() < 0.02 && (lo + 0.3 * NEG_FS).abs() < 0.02, "1 kHz {lo} {hi}");
        let (_, hi9) = tone(&mut AmProcessor::new(cfg, 48_000), 9000.0, 0.3, 48_000);
        assert!(hi9 < 0.3 * NEG_FS * 0.01, "9 kHz {hi9}");
        // 50 Hz high-pass: 20 Hz well down
        let (_, hi20) = tone(&mut AmProcessor::new(cfg, 48_000), 20.0, 0.3, 96_000);
        assert!(hi20 < 0.3 * NEG_FS * 0.3, "20 Hz {hi20}");
    }

    #[test]
    fn fm_preemphasis_is_taken_out() {
        // a 50 µs pre-emphasised 3 kHz tone (+3.5 dB) comes out at about its original level
        let pre = (1.0 + (2.0 * PI * 3000.0 * 50e-6).powi(2)).sqrt() as f32;
        let (_, hi) = tone(&mut AmProcessor::new(AmCfg { bandwidth_hz: 10_000.0, ..Default::default() }, 48_000), 3000.0, 0.2 * pre, 48_000);
        assert!((hi / NEG_FS - 0.2).abs() < 0.015, "{}", hi / NEG_FS);
    }
}
