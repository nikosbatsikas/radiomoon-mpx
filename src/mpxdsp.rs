//! FM multiplex (MPX) generator: stereo audio in, composite baseband out (192 kHz by default).
//! Pure signal processing (standard library only):
//! resampler -> 16 kHz low-pass -> pre-emphasis -> peak limiter -> stereo matrix + 19 kHz pilot + 38 kHz DSB-SC + 57 kHz RDS.
#![allow(dead_code)]
use std::f64::consts::PI;

#[derive(Clone, Debug)]
pub struct MpxParams {
    pub stereo: bool,
    /// Level of the audio part (L+R and L-R) in the composite, 0..1.
    pub audio_level: f32,
    /// Level of the 19 kHz pilot, 0..0.2.
    pub pilot_level: f32,
    /// Level of the RDS subcarrier, 0..0.1.
    pub rds_level: f32,
    /// Pre-emphasis time constant in microseconds (50 Europe, 75 America, 0 = off).
    pub preemph_us: f32,
    pub limiter: bool,
    /// Final output gain (1.0 = the composite reaches +-1.0 at 75 kHz deviation).
    pub output: f32,
}

impl Default for MpxParams {
    fn default() -> Self {
        MpxParams { stereo: true, audio_level: 0.85, pilot_level: 0.09, rds_level: 0.04, preemph_us: 50.0, limiter: true, output: 1.0 }
    }
}

// ---------------------------------------------------------------- RDS

#[derive(Clone, Debug)]
pub struct RdsData {
    pub enabled: bool,
    pub pi: u16,
    pub ps: String,
    pub rt: String,
    pub pty: u8,
    pub tp: bool,
    pub ta: bool,
    pub music: bool,
}

impl Default for RdsData {
    fn default() -> Self {
        RdsData { enabled: true, pi: 0x1234, ps: "RADIOMON".into(), rt: "RadioMoon".into(), pty: 10, tp: false, ta: false, music: true }
    }
}

const OFFSETS: [u16; 5] = [0x0FC, 0x198, 0x168, 0x350, 0x1B4]; // A B C C' D

/// 10-bit checkword of a 16-bit information word, already combined with the offset word.
pub fn rds_check(word: u16, offset: u16) -> u16 {
    let mut reg: u32 = (word as u32) << 10;
    for bit in (10..26).rev() {
        if reg & (1 << bit) != 0 {
            reg ^= 0x5B9 << (bit - 10); // x^10+x^8+x^7+x^5+x^4+x^3+1
        }
    }
    ((reg & 0x3FF) as u16) ^ offset
}

/// 26-bit block (information word + checkword) as an integer.
fn block(word: u16, offset: u16) -> u32 {
    ((word as u32) << 10) | rds_check(word, offset) as u32
}

fn char_code(c: char) -> u8 {
    // RDS uses its own table; ASCII 0x20..0x7E is identical.
    let u = c as u32;
    if (0x20..0x7F).contains(&u) {
        u as u8
    } else {
        match c {
            'ά' | 'Ά' | 'α' | 'Α' => b'A',
            'έ' | 'Έ' | 'ε' | 'Ε' => b'E',
            'ή' | 'Ή' | 'η' | 'Η' => b'H',
            'ί' | 'ϊ' | 'ΐ' | 'Ί' | 'ι' | 'Ι' => b'I',
            'ό' | 'Ό' | 'ο' | 'Ο' => b'O',
            'ύ' | 'ϋ' | 'ΰ' | 'Ύ' | 'υ' | 'Υ' => b'Y',
            'ώ' | 'Ώ' | 'ω' | 'Ω' => b'W',
            'β' | 'Β' => b'B',
            'γ' | 'Γ' => b'G',
            'δ' | 'Δ' => b'D',
            'ζ' | 'Ζ' => b'Z',
            'θ' | 'Θ' => b'T',
            'κ' | 'Κ' => b'K',
            'λ' | 'Λ' => b'L',
            'μ' | 'Μ' => b'M',
            'ν' | 'Ν' => b'N',
            'ξ' | 'Ξ' => b'X',
            'π' | 'Π' => b'P',
            'ρ' | 'Ρ' => b'R',
            'σ' | 'ς' | 'Σ' => b'S',
            'τ' | 'Τ' => b'T',
            'φ' | 'Φ' => b'F',
            'χ' | 'Χ' => b'X',
            'ψ' | 'Ψ' => b'P',
            _ => b' ',
        }
    }
}

fn text_bytes(s: &str, len: usize, pad: bool) -> Vec<u8> {
    let mut v: Vec<u8> = s.chars().map(char_code).take(len).collect();
    if pad {
        while v.len() < len {
            v.push(b' ');
        }
    }
    v
}

/// Builds the endless RDS group sequence (0A for the station name, 2A for RadioText) bit by bit.
pub struct RdsEncoder {
    pub data: RdsData,
    bits: Vec<u8>,
    pos: usize,
    seq: usize,
    ps_seg: usize,
    rt_seg: usize,
    rt_ab: bool,
    last_rt: String,
    cur_diff: u8,
}

impl RdsEncoder {
    pub fn new(data: RdsData) -> Self {
        RdsEncoder { data, bits: Vec::new(), pos: 0, seq: 0, ps_seg: 0, rt_seg: 0, rt_ab: false, last_rt: String::new(), cur_diff: 0 }
    }

    fn group(&mut self) -> [u16; 4] {
        let d = &self.data;
        let b2_common = ((d.tp as u16) << 10) | ((d.pty as u16 & 31) << 5);
        // sequence: 0A x4 (all PS segments), then 2A segments, alternating so both keep repeating
        let use_ps = self.seq % 3 != 2 || d.rt.is_empty();
        self.seq += 1;
        if use_ps {
            let seg = self.ps_seg;
            self.ps_seg = (self.ps_seg + 1) % 4;
            let ps = text_bytes(&d.ps, 8, true);
            let b2 = b2_common | ((d.ta as u16) << 4) | ((d.music as u16) << 3) | (seg as u16);
            [d.pi, b2, 0xCDCD, ((ps[seg * 2] as u16) << 8) | ps[seg * 2 + 1] as u16]
        } else {
            if d.rt != self.last_rt {
                self.last_rt = d.rt.clone();
                self.rt_ab = !self.rt_ab; // toggling the A/B flag tells receivers to clear the display
                self.rt_seg = 0;
            }
            let mut t = text_bytes(&d.rt, 64, false);
            if t.len() < 64 {
                t.push(0x0D);
            }
            let segs = (t.len() + 3) / 4;
            while t.len() < segs * 4 {
                t.push(b' ');
            }
            let seg = self.rt_seg % segs;
            self.rt_seg = (self.rt_seg + 1) % segs;
            let b2 = (2 << 12) | b2_common | ((self.rt_ab as u16) << 4) | (seg as u16);
            let i = seg * 4;
            [d.pi, b2, ((t[i] as u16) << 8) | t[i + 1] as u16, ((t[i + 2] as u16) << 8) | t[i + 3] as u16]
        }
    }

    fn refill(&mut self) {
        let g = self.group();
        self.bits.clear();
        for (i, w) in g.iter().enumerate() {
            let blk = block(*w, OFFSETS[i]);
            for k in (0..26).rev() {
                self.bits.push(((blk >> k) & 1) as u8);
            }
        }
        self.pos = 0;
    }

    /// Next differentially encoded bit (called at 1187.5 Hz).
    pub fn next_bit(&mut self) -> u8 {
        if self.pos >= self.bits.len() {
            self.refill();
        }
        let b = self.bits[self.pos];
        self.pos += 1;
        self.cur_diff ^= b;
        self.cur_diff
    }
}

// ---------------------------------------------------------------- filters

struct Fir {
    h: Vec<f32>,
    buf: Vec<f32>,
    pos: usize,
}

impl Fir {
    /// Windowed-sinc low-pass (Blackman), `taps` should be odd.
    fn lowpass(cut_hz: f64, fs: f64, taps: usize) -> Fir {
        let m = (taps - 1) as f64 / 2.0;
        let fc = cut_hz / fs;
        let mut h: Vec<f64> = (0..taps)
            .map(|i| {
                let x = i as f64 - m;
                let s = if x == 0.0 { 2.0 * fc } else { (2.0 * PI * fc * x).sin() / (PI * x) };
                let w = 0.42 - 0.5 * (2.0 * PI * i as f64 / (taps - 1) as f64).cos() + 0.08 * (4.0 * PI * i as f64 / (taps - 1) as f64).cos();
                s * w
            })
            .collect();
        let sum: f64 = h.iter().sum();
        for v in h.iter_mut() {
            *v /= sum;
        }
        Fir { h: h.iter().map(|v| *v as f32).collect(), buf: vec![0.0; taps * 2], pos: 0 }
    }

    #[inline]
    fn push(&mut self, x: f32) -> f32 {
        let n = self.h.len();
        self.buf[self.pos] = x;
        self.buf[self.pos + n] = x;
        self.pos += 1;
        if self.pos == n {
            self.pos = 0;
        }
        let w = &self.buf[self.pos..self.pos + n];
        let mut acc = 0.0f32;
        for (a, b) in w.iter().zip(self.h.iter()) {
            acc += a * b;
        }
        acc
    }
}

/// First-order pre-emphasis (1 + s*tau) / (1 + s*tau2), bilinear transform, unity gain at DC.
struct PreEmph {
    b0: f32,
    b1: f32,
    a1: f32,
    x1: f32,
    y1: f32,
    on: bool,
}

impl PreEmph {
    fn new(tau_us: f32, fs: f64) -> PreEmph {
        if tau_us <= 0.0 {
            return PreEmph { b0: 1.0, b1: 0.0, a1: 0.0, x1: 0.0, y1: 0.0, on: false };
        }
        let tau = tau_us as f64 * 1e-6;
        let tau2 = 3.18e-6; // limits the boost above ~50 kHz
        let k = 2.0 * fs;
        let d = 1.0 + k * tau2;
        PreEmph { b0: ((1.0 + k * tau) / d) as f32, b1: ((1.0 - k * tau) / d) as f32, a1: ((1.0 - k * tau2) / d) as f32, x1: 0.0, y1: 0.0, on: true }
    }
    #[inline]
    fn push(&mut self, x: f32) -> f32 {
        if !self.on {
            return x;
        }
        let y = self.b0 * x + self.b1 * self.x1 - self.a1 * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

// ---------------------------------------------------------------- the generator

pub struct MpxEncoder {
    pub fs: u32,
    pub params: MpxParams,
    pub rds: RdsEncoder,
    in_rate: f64,
    hist: [[f32; 2]; 4],
    mu: f64,
    step: f64,
    fir_l: Fir,
    fir_r: Fir,
    pre_l: PreEmph,
    pre_r: PreEmph,
    pre_tau: f32,
    gain: f32,
    p: f64,        // pilot phase in turns, counted over 16 pilot cycles (= one RDS bit)
    rds_sym: f32,  // current RDS data polarity (+1/-1)
    /// Largest absolute composite value since the last `take_peak`.
    peak: f32,
}

impl MpxEncoder {
    pub fn new(in_rate: u32, fs: u32, params: MpxParams, rds: RdsData) -> MpxEncoder {
        let pre_tau = params.preemph_us;
        let mut e = MpxEncoder {
            fs,
            in_rate: in_rate as f64,
            hist: [[0.0; 2]; 4],
            mu: 1.0,
            step: in_rate as f64 / fs as f64,
            fir_l: Fir::lowpass(16_000.0, fs as f64, 255),
            fir_r: Fir::lowpass(16_000.0, fs as f64, 255),
            pre_l: PreEmph::new(pre_tau, fs as f64),
            pre_r: PreEmph::new(pre_tau, fs as f64),
            pre_tau,
            gain: 1.0,
            p: 0.0,
            rds_sym: 1.0,
            peak: 0.0,
            rds: RdsEncoder::new(rds),
            params,
        };
        e.rds_sym = if e.rds.next_bit() == 1 { 1.0 } else { -1.0 };
        e
    }

    pub fn set_params(&mut self, p: MpxParams) {
        if (p.preemph_us - self.pre_tau).abs() > f32::EPSILON {
            self.pre_l = PreEmph::new(p.preemph_us, self.fs as f64);
            self.pre_r = PreEmph::new(p.preemph_us, self.fs as f64);
            self.pre_tau = p.preemph_us;
        }
        self.params = p;
    }

    pub fn take_peak(&mut self) -> f32 {
        std::mem::replace(&mut self.peak, 0.0)
    }

    pub fn input_rate(&self) -> u32 {
        self.in_rate as u32
    }

    /// Converts interleaved stereo input samples to MPX samples (appended to `out`).
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let frames = input.len() / 2;
        out.reserve((frames as f64 / self.step) as usize + 2);
        let mut i = 0usize;
        let pr = self.params.clone();
        let pinc = 19_000.0 / self.fs as f64;
        loop {
            while self.mu >= 1.0 {
                if i >= frames {
                    return;
                }
                self.hist.rotate_left(1);
                self.hist[3] = [input[i * 2], input[i * 2 + 1]];
                i += 1;
                self.mu -= 1.0;
            }
            let mu = self.mu as f32;
            self.mu += self.step;
            let cr = |c: usize| {
                let (x0, x1, x2, x3) = (self.hist[0][c], self.hist[1][c], self.hist[2][c], self.hist[3][c]);
                0.5 * (2.0 * x1 + (x2 - x0) * mu + (2.0 * x0 - 5.0 * x1 + 4.0 * x2 - x3) * mu * mu + (3.0 * x1 - x0 - 3.0 * x2 + x3) * mu * mu * mu)
            };
            let (l0, r0) = (cr(0), cr(1));
            // band-limit to 16 kHz (also removes the images created by the up-sampling)
            let (mut l, mut r) = (self.fir_l.push(l0), self.fir_r.push(r0));
            l = self.pre_l.push(l);
            r = self.pre_r.push(r);
            if pr.limiter {
                let pk = l.abs().max(r.abs());
                let target = if pk > 1.0 { 1.0 / pk } else { 1.0 };
                if target < self.gain {
                    self.gain = target;
                } else {
                    self.gain += (target - self.gain) * 0.0004;
                }
                l *= self.gain;
                r *= self.gain;
            }
            // phase bookkeeping
            self.p += pinc;
            if self.p >= 16.0 {
                self.p -= 16.0;
            }
            if self.p < pinc {
                // a new RDS bit starts exactly every 16 pilot cycles
                self.rds_sym = if self.rds.next_bit() == 1 { 1.0 } else { -1.0 };
            }
            let th = 2.0 * PI * self.p; // pilot angle (wraps seamlessly: 16 cycles)
            let m = (l + r) * 0.5;
            let mut c;
            if pr.stereo {
                let s = (l - r) * 0.5;
                c = pr.audio_level * (m + s * (2.0 * th).sin() as f32) + pr.pilot_level * th.sin() as f32;
            } else {
                c = pr.audio_level * m;
            }
            if self.rds.data.enabled && pr.rds_level > 0.0 {
                // biphase symbol: one full sine period per bit, on the 57 kHz carrier
                let sym = (th / 16.0).sin() as f32 * self.rds_sym;
                c += pr.rds_level * sym * (3.0 * th).sin() as f32;
            }
            c *= pr.output;
            let c = c.clamp(-1.0, 1.0);
            let a = c.abs();
            if a > self.peak {
                self.peak = a;
            }
            out.push(c);
        }
    }
}

// ---------------------------------------------------------------- sample formats for the IP stream

pub fn to_l24(samples: &[f32], out: &mut Vec<u8>) {
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 8_388_607.0) as i32;
        out.extend_from_slice(&[(v >> 16) as u8, (v >> 8) as u8, v as u8]);
    }
}

pub fn from_l24(bytes: &[u8], out: &mut Vec<f32>) {
    for c in bytes.chunks_exact(3) {
        let v = (((c[0] as i32) << 24) | ((c[1] as i32) << 16) | ((c[2] as i32) << 8)) >> 8;
        out.push(v as f32 / 8_388_607.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rds_syndrome() {
        // the syndrome of a received block (information + checkword) equals the offset's own syndrome
        for (w, off) in [(0x1234u16, 0usize), (0xABCD, 3), (0x0001, 4)] {
            let blk = block(w, OFFSETS[off]);
            let s = rds_check((blk >> 10) as u16, 0) ^ (blk & 0x3FF) as u16;
            assert_eq!(s, OFFSETS[off]);
        }
    }

    #[test]
    fn l24_roundtrip() {
        let a = vec![0.0, 0.5, -0.5, 1.0, -1.0];
        let mut b = Vec::new();
        to_l24(&a, &mut b);
        let mut c = Vec::new();
        from_l24(&b, &mut c);
        for (x, y) in a.iter().zip(c.iter()) {
            assert!((x - y).abs() < 1e-6);
        }
    }
}
