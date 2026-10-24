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
    /// The 8 characters of the station name sent now (one page of a long PS text, see PsPager).
    pub ps: String,
    pub rt: String,
    pub pty: u8,
    pub tp: bool,
    pub ta: bool,
    pub music: bool,
    /// Alternative frequencies (AF, method A), as RDS codes: 1 = 87.6 MHz ... 204 = 107.9 MHz.
    pub af: Vec<u8>,
    /// Send the clock time (group 4A) once a minute.
    pub ct: bool,
    /// Local time offset of the clock time, half hours (e.g. +6 = UTC+3).
    pub ct_offset: i8,
}

impl Default for RdsData {
    fn default() -> Self {
        RdsData { enabled: true, pi: 0x1234, ps: "RADIOMON".into(), rt: "RadioMoon".into(), pty: 10, tp: false, ta: false, music: true, af: Vec::new(), ct: false, ct_offset: 0 }
    }
}

/// RDS code of an FM frequency (87.6 - 107.9 MHz in 0.1 MHz steps), method A.
pub fn af_code(mhz: f32) -> Option<u8> {
    let c = ((mhz - 87.5) * 10.0).round();
    (1.0..=204.0).contains(&c).then_some(c as u8)
}

/// Frequency of an AF code, MHz.
pub fn af_mhz(code: u8) -> f32 {
    87.5 + code as f32 / 10.0
}

/// Parses "94.3, 101.5 105.0" into AF codes (at most 25, no doubles).
pub fn parse_af(text: &str) -> Vec<u8> {
    let mut v: Vec<u8> = Vec::new();
    for t in text.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).filter(|t| !t.is_empty()) {
        if let Some(c) = t.replace(',', ".").parse::<f32>().ok().and_then(af_code) {
            if !v.contains(&c) && v.len() < 25 {
                v.push(c);
            }
        }
    }
    v
}

/// Long station names shown 8 characters at a time ("dynamic PS"): by words (pages) or scrolling one character
/// at a time, every `interval` seconds. Note: the RDS standard wants a fixed PS; many stations still use this.
pub struct PsPager {
    start: std::time::Instant,
}

impl Default for PsPager {
    fn default() -> Self {
        PsPager { start: std::time::Instant::now() }
    }
}

impl PsPager {
    /// The pages of a text: "static" = the first 8 characters, "pages" = words packed into pages of 8 characters
    /// (a longer word is cut), "scroll" = the text moving one character per step.
    pub fn pages(text: &str, mode: &str) -> Vec<String> {
        let t: Vec<char> = text.trim().chars().collect();
        let pad = |s: String| -> String { format!("{s:<8}").chars().take(8).collect() };
        if t.len() <= 8 || mode == "static" {
            return vec![pad(t.iter().take(8).collect())];
        }
        if mode == "scroll" {
            let mut lane: Vec<char> = t.clone();
            lane.extend("    ".chars()); // a short gap before the text starts again
            return (0..lane.len()).map(|i| (0..8).map(|j| lane[(i + j) % lane.len()]).collect()).collect();
        }
        let mut pages = Vec::new();
        let mut cur = String::new();
        for w in text.split_whitespace() {
            let w: String = w.chars().take(8).collect();
            if cur.is_empty() {
                cur = w;
            } else if cur.chars().count() + 1 + w.chars().count() <= 8 {
                cur = format!("{cur} {w}");
            } else {
                pages.push(pad(cur));
                cur = w;
            }
        }
        if !cur.is_empty() {
            pages.push(pad(cur));
        }
        // centre the words of each page, as on most radios' displays
        pages.into_iter().map(|p| {
            let w = p.trim_end().chars().count();
            let left = (8 - w) / 2;
            format!("{}{}", " ".repeat(left), p.trim_end()).chars().chain(std::iter::repeat(' ')).take(8).collect()
        }).collect()
    }

    /// The page to send now.
    pub fn now(&self, text: &str, mode: &str, interval_s: f32) -> String {
        let pages = Self::pages(text, mode);
        let i = (self.start.elapsed().as_secs_f32() / interval_s.max(0.3)) as usize % pages.len();
        pages[i].clone()
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
    /// The station name being sent: taken from `data.ps` only at the start of a complete PS (segment 0), so a
    /// changing name never mixes two pages on a radio.
    ps_now: String,
    af_pair: usize,
    last_ct_minute: i64,
}

impl RdsEncoder {
    pub fn new(data: RdsData) -> Self {
        RdsEncoder { ps_now: data.ps.clone(), data, bits: Vec::new(), pos: 0, seq: 0, ps_seg: 0, rt_seg: 0, rt_ab: false, last_rt: String::new(), cur_diff: 0, af_pair: 0, last_ct_minute: -1 }
    }

    /// Block C of group 0A: the AF list (method A), one pair per group; the first pair gives the count.
    fn af_block(&mut self) -> u16 {
        const FILLER: u8 = 205;
        let af = &self.data.af;
        if af.is_empty() {
            return 0xE0CD; // "no AF" (224) + filler
        }
        // pairs: (224 + n, f1), (f2, f3), (f4, f5) ...
        let mut list: Vec<u8> = vec![224 + af.len() as u8];
        list.extend_from_slice(af);
        if list.len() % 2 == 1 {
            list.push(FILLER);
        }
        let pairs = list.len() / 2;
        let i = self.af_pair % pairs;
        self.af_pair = (self.af_pair + 1) % pairs;
        ((list[i * 2] as u16) << 8) | list[i * 2 + 1] as u16
    }

    /// Group 4A (clock time) at the start of every minute, when CT is on.
    fn ct_group(&mut self) -> Option<[u16; 4]> {
        if !self.data.ct {
            return None;
        }
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
        let minute = secs / 60;
        if minute == self.last_ct_minute {
            return None;
        }
        self.last_ct_minute = minute;
        let d = &self.data;
        let mjd = (secs.div_euclid(86_400) + 40_587) as u32;
        let (hour, min) = (((secs % 86_400) / 3600) as u32, ((secs % 3600) / 60) as u32);
        let off = d.ct_offset.clamp(-31, 31);
        let b2 = (4 << 12) | ((d.tp as u16) << 10) | ((d.pty as u16 & 31) << 5) | ((mjd >> 15) & 3) as u16;
        let b3 = (((mjd & 0x7FFF) << 1) | (hour >> 4)) as u16;
        let b4 = (((hour & 15) << 12) | (min << 6) | (if off < 0 { 1 << 5 } else { 0 }) | off.unsigned_abs() as u32) as u16;
        Some([d.pi, b2, b3, b4])
    }

    fn group(&mut self) -> [u16; 4] {
        if let Some(g) = self.ct_group() {
            return g;
        }
        if self.ps_seg == 0 {
            self.ps_now = self.data.ps.clone();
        }
        let af = self.af_block();
        let d = &self.data;
        let b2_common = ((d.tp as u16) << 10) | ((d.pty as u16 & 31) << 5);
        // sequence: 0A x4 (all PS segments), then 2A segments, alternating so both keep repeating
        let use_ps = self.seq % 3 != 2 || d.rt.is_empty();
        self.seq += 1;
        if use_ps {
            let seg = self.ps_seg;
            self.ps_seg = (self.ps_seg + 1) % 4;
            let ps = text_bytes(&self.ps_now, 8, true);
            let b2 = b2_common | ((d.ta as u16) << 4) | ((d.music as u16) << 3) | (seg as u16);
            [d.pi, b2, af, ((ps[seg * 2] as u16) << 8) | ps[seg * 2 + 1] as u16]
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
        // offset words of blocks A, B, C and D (C' is only for version B groups, which are not sent)
        let offs = [OFFSETS[0], OFFSETS[1], OFFSETS[2], OFFSETS[4]];
        for (w, off) in g.iter().zip(offs) {
            let blk = block(*w, off);
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

// ---------------------------------------------------------------- the composite (stereo matrix, pilot, RDS)

/// Builds the composite signal from processed stereo audio (band-limited, pre-emphasised, limited):
/// L+R, L-R on 38 kHz, the 19 kHz pilot and RDS on 57 kHz, all locked to the same phase.
/// Used by the generator and by the receiver of Compact MPX, so both produce the same signal.
pub struct Composer {
    pub fs: u32,
    pub params: MpxParams,
    pub rds: RdsEncoder,
    p: f64,       // pilot phase in turns, counted over 16 pilot cycles (= one RDS bit)
    rds_sym: f32, // current RDS data polarity (+1/-1)
    /// Largest absolute composite value since the last `take_peak`.
    peak: f32,
}

impl Composer {
    pub fn new(fs: u32, params: MpxParams, rds: RdsData) -> Composer {
        let mut c = Composer { fs, params, rds: RdsEncoder::new(rds), p: 0.0, rds_sym: 1.0, peak: 0.0 };
        c.rds_sym = if c.rds.next_bit() == 1 { 1.0 } else { -1.0 };
        c
    }

    pub fn take_peak(&mut self) -> f32 {
        std::mem::replace(&mut self.peak, 0.0)
    }

    /// One composite sample from one stereo sample.
    #[inline]
    pub fn push(&mut self, l: f32, r: f32) -> f32 {
        let pr = &self.params;
        let pinc = 19_000.0 / self.fs as f64;
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
        c
    }
}

// ---------------------------------------------------------------- the generator

pub struct MpxEncoder {
    pub fs: u32,
    pub comp: Composer,
    /// When set, the processed stereo audio (after filter, pre-emphasis and limiter) is also appended here,
    /// interleaved L R at `fs` (used for Compact MPX).
    pub tap: Option<Vec<f32>>,
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
}

impl MpxEncoder {
    pub fn new(in_rate: u32, fs: u32, params: MpxParams, rds: RdsData) -> MpxEncoder {
        let pre_tau = params.preemph_us;
        MpxEncoder {
            fs,
            tap: None,
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
            comp: Composer::new(fs, params, rds),
        }
    }

    pub fn set_params(&mut self, p: MpxParams) {
        if (p.preemph_us - self.pre_tau).abs() > f32::EPSILON {
            self.pre_l = PreEmph::new(p.preemph_us, self.fs as f64);
            self.pre_r = PreEmph::new(p.preemph_us, self.fs as f64);
            self.pre_tau = p.preemph_us;
        }
        self.comp.params = p;
    }

    pub fn set_rds(&mut self, d: RdsData) {
        self.comp.rds.data = d;
    }

    pub fn take_peak(&mut self) -> f32 {
        self.comp.take_peak()
    }

    pub fn input_rate(&self) -> u32 {
        self.in_rate as u32
    }

    /// Converts interleaved stereo input samples to MPX samples (appended to `out`).
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let frames = input.len() / 2;
        out.reserve((frames as f64 / self.step) as usize + 2);
        let mut i = 0usize;
        let limiter = self.comp.params.limiter;
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
            if limiter {
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
            if let Some(t) = self.tap.as_mut() {
                t.push(l);
                t.push(r);
            }
            out.push(self.comp.push(l, r));
        }
    }
}

// ---------------------------------------------------------------- polyphase interpolator (Compact MPX receiver)

/// Raises the sample rate by an integer factor with a windowed-sinc low-pass, so the images of the audio stay
/// out of the stereo and RDS bands.
pub struct Interpolator {
    factor: usize,
    /// phases[k][j]: tap j of phase k
    phases: Vec<Vec<f32>>,
    hist: Vec<f32>,
    pos: usize,
}

impl Interpolator {
    pub fn new(factor: usize, out_fs: f64, cut_hz: f64, taps_per_phase: usize) -> Interpolator {
        let taps = factor * taps_per_phase;
        let proto = Fir::lowpass(cut_hz, out_fs, taps + 1);
        let mut phases = vec![vec![0.0f32; taps_per_phase]; factor];
        for (i, &h) in proto.h.iter().take(taps).enumerate() {
            phases[i % factor][i / factor] = h * factor as f32;
        }
        Interpolator { factor, phases, hist: vec![0.0; taps_per_phase * 2], pos: 0 }
    }

    /// One input sample in, `factor` output samples appended.
    pub fn push(&mut self, x: f32, out: &mut Vec<f32>) {
        let n = self.hist.len() / 2;
        self.pos = if self.pos == 0 { n - 1 } else { self.pos - 1 };
        self.hist[self.pos] = x;
        self.hist[self.pos + n] = x;
        let w = &self.hist[self.pos..self.pos + n];
        for k in 0..self.factor {
            let mut acc = 0.0f32;
            for (a, b) in w.iter().zip(self.phases[k].iter()) {
                acc += a * b;
            }
            out.push(acc);
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
