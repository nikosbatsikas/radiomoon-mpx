//! RDS decoder: reads PI, PS (station name), RadioText, PTY, TP / TA / music from a 192 kHz MPX signal.
//! 57 kHz carrier recovery with a Costas loop (works with or without the stereo pilot), two-stage filtering,
//! biphase symbol detection locked to the carrier (48 carrier cycles per bit), differential decoding,
//! block synchronisation with the checkwords, and groups 0A/0B (PS) and 2A/2B (RadioText).
//! Standard library only.
use std::f64::consts::PI;

const OFFSET_A: u16 = 0x0FC;
const OFFSET_B: u16 = 0x198;
const OFFSET_C: u16 = 0x168;
const OFFSET_CP: u16 = 0x350;
const OFFSET_D: u16 = 0x1B4;

/// Checkword of a 16-bit information word (without offset).
fn check(word: u16) -> u16 {
    let mut reg: u32 = (word as u32) << 10;
    for bit in (10..26).rev() {
        if reg & (1 << bit) != 0 {
            reg ^= 0x5B9 << (bit - 10);
        }
    }
    (reg & 0x3FF) as u16
}

/// The offset word of a received 26-bit block (0 when the block is damaged or not a block).
fn offset_of(block: u32) -> u16 {
    check((block >> 10) as u16) ^ (block & 0x3FF) as u16
}

/// What has been received so far.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RdsInfo {
    /// RDS is being received (blocks decode correctly).
    pub synced: bool,
    pub pi: Option<u16>,
    /// Station name, 8 characters (unreceived characters are spaces).
    pub ps: String,
    pub rt: String,
    pub pty: Option<u8>,
    pub tp: bool,
    pub ta: bool,
    pub music: bool,
    /// Share of correct blocks recently, 0..1.
    pub quality: f32,
    pub groups: u64,
    /// Alternative frequencies (MHz), in the order received.
    pub af: Vec<f32>,
    /// Clock time (group 4A): local date and time as sent, e.g. "25.10.2026 02:31 (UTC+3)".
    pub ct: String,
    /// Local time offset of the clock time, half hours.
    pub ct_offset: Option<i8>,
}

/// Decimating low-pass (windowed sinc, Blackman), computes only every `m`-th output.
struct Decim {
    h: Vec<f32>,
    buf: Vec<f32>,
    pos: usize,
    m: usize,
    phase: usize,
}

impl Decim {
    fn new(cut: f64, fs: f64, taps: usize, m: usize) -> Decim {
        let mid = (taps - 1) as f64 / 2.0;
        let fc = cut / fs;
        let mut h: Vec<f64> = (0..taps)
            .map(|i| {
                let x = i as f64 - mid;
                let s = if x == 0.0 { 2.0 * fc } else { (2.0 * PI * fc * x).sin() / (PI * x) };
                let w = 0.42 - 0.5 * (2.0 * PI * i as f64 / (taps - 1) as f64).cos() + 0.08 * (4.0 * PI * i as f64 / (taps - 1) as f64).cos();
                s * w
            })
            .collect();
        let sum: f64 = h.iter().sum();
        h.iter_mut().for_each(|v| *v /= sum);
        Decim { h: h.iter().map(|v| *v as f32).collect(), buf: vec![0.0; taps * 2], pos: 0, m, phase: 0 }
    }
    /// Returns an output every `m` inputs.
    #[inline]
    fn push(&mut self, x: f32) -> Option<f32> {
        let n = self.h.len();
        self.buf[self.pos] = x;
        self.buf[self.pos + n] = x;
        self.pos += 1;
        if self.pos == n {
            self.pos = 0;
        }
        self.phase += 1;
        if self.phase < self.m {
            return None;
        }
        self.phase = 0;
        let w = &self.buf[self.pos..self.pos + n];
        Some(w.iter().zip(&self.h).map(|(a, b)| a * b).sum())
    }
}

pub struct RdsDecoder {
    fs: f64,
    // carrier (cycles)
    phase: f64,
    freq: f64,
    nominal: f64,
    i1: Decim,
    q1: Decim,
    i2: Decim,
    q2: Decim,
    // symbol sampling: 16 samples per bit, taken every 3 carrier cycles
    cyc: f64,
    prev_i: f32,
    samples: Vec<f32>,
    energy: [f32; 16],
    since_bit: usize,
    best: usize,
    last_sym: i8,
    // blocks
    reg: u32,
    nbits: u32,
    synced: bool,
    expect: usize,
    bad_run: u32,
    good: f32,
    group: [Option<u16>; 4],
    group_c_prime: bool,
    // contents
    ps: [u8; 8],
    ps_seen: u8,
    rt: [u8; 64],
    rt_seen: u64,
    rt_ab: Option<bool>,
    /// Average magnitude of the RDS signal after filtering (see `level`).
    mag: f64,
    pub info: RdsInfo,
}

impl RdsDecoder {
    pub fn new(fs: u32) -> RdsDecoder {
        let fs = fs as f64;
        let nominal = 57_000.0 / fs;
        RdsDecoder {
            fs,
            phase: 0.0,
            freq: nominal,
            nominal,
            // 192 kHz -> 24 kHz, then 2.4 kHz low-pass (keeps the RDS band, removes the stereo band's edge at 4 kHz)
            i1: Decim::new(6_000.0, fs, 129, 8),
            q1: Decim::new(6_000.0, fs, 129, 8),
            i2: Decim::new(2_400.0, fs / 8.0, 97, 1),
            q2: Decim::new(2_400.0, fs / 8.0, 97, 1),
            cyc: 0.0,
            prev_i: 0.0,
            samples: Vec::with_capacity(64),
            energy: [0.0; 16],
            since_bit: 0,
            best: 0,
            last_sym: 1,
            reg: 0,
            nbits: 0,
            synced: false,
            expect: 0,
            bad_run: 0,
            good: 0.0,
            group: [None; 4],
            group_c_prime: false,
            ps: [b' '; 8],
            ps_seen: 0,
            rt: [b' '; 64],
            rt_seen: 0,
            rt_ab: None,
            mag: 0.0,
            info: RdsInfo { ps: " ".repeat(8), ..RdsInfo::default() },
        }
    }

    /// Feeds MPX samples (192 kHz, 1.0 = 75 kHz deviation).
    pub fn push(&mut self, mpx: &[f32]) {
        for &x in mpx {
            let a = 2.0 * PI * self.phase;
            let (s, c) = a.sin_cos();
            let ii = self.i1.push(x * c as f32);
            let qq = self.q1.push(-x * s as f32);
            self.phase += self.freq;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            if let (Some(ii), Some(qq)) = (ii, qq) {
                let i = self.i2.push(ii).unwrap_or(0.0);
                let q = self.q2.push(qq).unwrap_or(0.0);
                self.costas(i, q);
                self.sample(i);
            }
        }
    }

    /// Costas loop: keeps the local 57 kHz exactly on the carrier (sign ambiguity is removed by the differential code).
    /// Level of the RDS subcarrier (1.0 = 75 kHz deviation), measured from the filtered signal.
    pub fn level(&self) -> f32 {
        // average magnitude of the filtered biphase signal -> subcarrier level (measured with the generator)
        (self.mag * 3.12) as f32
    }

    fn costas(&mut self, i: f32, q: f32) {
        self.mag += (((i * i + q * q) as f64).sqrt() - self.mag) * 0.0005;
        let p = (i * i + q * q) as f64 + 1e-12;
        let err = (i * q) as f64 / p; // ~ sin(2 * phase error) / 2
        // loop about 15 Hz wide at 24 kHz
        let wn = 2.0 * PI * 15.0 / 24_000.0;
        let (kp, ki) = (2.0 * 0.707 * wn, wn * wn);
        self.freq += ki * err / (2.0 * PI);
        let lim = 20.0 / self.fs;
        self.freq = self.freq.clamp(self.nominal - lim, self.nominal + lim);
        self.phase += kp * err / (2.0 * PI);
        self.phase = self.phase.rem_euclid(1.0);
    }

    /// Takes the in-phase signal every 3 carrier cycles (16 samples per bit) and detects the bits.
    fn sample(&mut self, i: f32) {
        // carrier cycles that passed during this decimated step
        let step = self.freq * 8.0;
        let start = self.cyc;
        self.cyc += step;
        while self.cyc >= 3.0 {
            self.cyc -= 3.0;
            // linear interpolation at the exact moment
            let frac = ((3.0 - start) / step).clamp(0.0, 1.0) as f32;
            let v = self.prev_i + (i - self.prev_i) * frac;
            self.symbol_sample(v);
        }
        self.prev_i = i;
    }

    fn symbol_sample(&mut self, v: f32) {
        self.samples.push(v);
        if self.samples.len() < 32 {
            return;
        }
        self.since_bit += 1;
        // biphase: one sine period per bit. Compare the 16 possible bit starts.
        if self.since_bit >= 16 {
            self.since_bit = 0;
            let base = self.samples.len() - 32;
            let mut corr = [0f32; 16];
            for (o, c) in corr.iter_mut().enumerate() {
                let mut acc = 0.0;
                for j in 0..16 {
                    acc += self.samples[base + o + j] * ((2.0 * std::f32::consts::PI * (j as f32 + 0.5) / 16.0).sin());
                }
                *c = acc;
                self.energy[o] = self.energy[o] * 0.97 + acc.abs();
            }
            let best = (0..16).max_by(|a, b| self.energy[*a].total_cmp(&self.energy[*b])).unwrap_or(0);
            self.best = best;
            let sym: i8 = if corr[best] >= 0.0 { 1 } else { -1 };
            let bit = (sym != self.last_sym) as u32; // differential decoding
            self.last_sym = sym;
            self.bit(bit);
            self.samples.drain(..16);
        }
    }

    fn bit(&mut self, b: u32) {
        self.reg = ((self.reg << 1) | b) & 0x3FF_FFFF;
        self.nbits += 1;
        if !self.synced {
            // look for a correct block of any type at every bit
            let off = offset_of(self.reg);
            let idx = match off {
                OFFSET_A => Some(0),
                OFFSET_B => Some(1),
                OFFSET_C | OFFSET_CP => Some(2),
                OFFSET_D => Some(3),
                _ => None,
            };
            if let Some(k) = idx {
                self.synced = true;
                self.nbits = 0;
                self.bad_run = 0;
                self.block(k, true);
                self.expect = (k + 1) % 4;
            }
            return;
        }
        if self.nbits < 26 {
            return;
        }
        self.nbits = 0;
        let off = offset_of(self.reg);
        let k = self.expect;
        let ok = match k {
            0 => off == OFFSET_A,
            1 => off == OFFSET_B,
            2 => {
                self.group_c_prime = off == OFFSET_CP;
                off == OFFSET_C || off == OFFSET_CP
            }
            _ => off == OFFSET_D,
        };
        self.block(k, ok);
        self.expect = (k + 1) % 4;
        if ok {
            self.bad_run = 0;
        } else {
            self.bad_run += 1;
            if self.bad_run > 10 {
                self.synced = false; // lost: search again
                self.info.synced = false;
            }
        }
    }

    fn block(&mut self, k: usize, ok: bool) {
        self.good = self.good * 0.98 + if ok { 0.02 } else { 0.0 };
        self.info.quality = self.good.min(1.0);
        if k == 0 {
            self.group = [None; 4];
        }
        self.group[k] = if ok { Some((self.reg >> 10) as u16) } else { None };
        if k == 3 {
            self.group_done();
        }
    }

    fn group_done(&mut self) {
        let [a, b, c, d] = self.group;
        if let Some(a) = a {
            self.info.pi = Some(a);
        }
        let Some(b) = b else { return };
        self.info.synced = true;
        self.info.groups += 1;
        let gtype = b >> 12;
        let version_b = (b >> 11) & 1 == 1;
        self.info.tp = (b >> 10) & 1 == 1;
        self.info.pty = Some(((b >> 5) & 31) as u8);
        match gtype {
            0 => {
                self.info.ta = (b >> 4) & 1 == 1;
                self.info.music = (b >> 3) & 1 == 1;
                if let (false, Some(c)) = (version_b, c) {
                    // AF method A: two codes per group; 224 + n announces the count
                    for code in [(c >> 8) as u8, c as u8] {
                        if (1..=204).contains(&code) {
                            let f = 87.5 + code as f32 / 10.0;
                            if !self.info.af.iter().any(|x| (x - f).abs() < 0.05) && self.info.af.len() < 25 {
                                self.info.af.push(f);
                            }
                        } else if code == 224 {
                            self.info.af.clear(); // "no AF"
                        }
                    }
                }
                if let Some(d) = d {
                    // the name is shown only when its four segments came complete and in order (0, 1, 2, 3):
                    // a changing PS (pages) then never shows a mix of two pages
                    let seg = (b & 3) as usize;
                    if seg == 0 {
                        self.ps_seen = 0;
                    }
                    if seg == 0 || self.ps_seen == (1 << seg) - 1 {
                        self.ps[seg * 2] = (d >> 8) as u8;
                        self.ps[seg * 2 + 1] = d as u8;
                        self.ps_seen |= 1 << seg;
                    } else {
                        self.ps_seen = 0;
                    }
                    if self.ps_seen == 0b1111 {
                        self.info.ps = self.ps.iter().map(|&c| rds_char(c)).collect();
                        self.ps_seen = 0;
                    }
                }
            }
            2 => {
                let ab = (b >> 4) & 1 == 1;
                if self.rt_ab != Some(ab) {
                    if self.rt_ab.is_some() {
                        self.rt = [b' '; 64]; // A/B flag changed: a new text
                        self.rt_seen = 0;
                    }
                    self.rt_ab = Some(ab);
                }
                let seg = (b & 15) as usize;
                if !version_b {
                    if let (Some(c), Some(d)) = (c, d) {
                        if !self.group_c_prime {
                            for (j, w) in [c, d].iter().enumerate() {
                                self.rt[seg * 4 + j * 2] = (w >> 8) as u8;
                                self.rt[seg * 4 + j * 2 + 1] = *w as u8;
                            }
                            self.rt_seen |= 1 << seg;
                        }
                    }
                } else if let Some(d) = d {
                    self.rt[seg * 2] = (d >> 8) as u8;
                    self.rt[seg * 2 + 1] = d as u8;
                    self.rt_seen |= 1 << seg;
                }
                let end = self.rt.iter().position(|&c| c == 0x0D).unwrap_or(64);
                self.info.rt = self.rt[..end].iter().map(|&c| rds_char(c)).collect::<String>().trim_end().to_string();
            }
            4 if !version_b => {
                // clock time: MJD, UTC hour and minute, local offset in half hours
                if let (Some(c), Some(d)) = (c, d) {
                    let mjd = ((b as u32 & 3) << 15) | (c as u32 >> 1);
                    let hour = ((c as u32 & 1) << 4) | (d as u32 >> 12);
                    let min = (d as u32 >> 6) & 63;
                    let off = (d as i32 & 31) * if d & 0x20 != 0 { -1 } else { 1 };
                    if hour < 24 && min < 60 && mjd > 50_000 {
                        let local = (mjd as i64 - 40_587) * 86_400 + hour as i64 * 3600 + min as i64 * 60 + off as i64 * 1800;
                        let (days, rem) = (local.div_euclid(86_400), local.rem_euclid(86_400));
                        let (y, m, dd) = civil(days);
                        let tz = if off % 2 == 0 { format!("{:+}", off / 2) } else { format!("{:+.1}", off as f32 / 2.0) };
                        self.info.ct_offset = Some(off as i8);
                        self.info.ct = format!("{dd:02}.{m:02}.{y} {:02}:{:02} (UTC{tz})", rem / 3600, rem % 3600 / 60);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Days since 1970-01-01 to (year, month, day).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// RDS character set: ASCII is identical; the rest is shown as a dot.
fn rds_char(c: u8) -> char {
    if (0x20..0x7F).contains(&c) {
        c as char
    } else {
        '·'
    }
}

/// Programme type names (Europe, RDS).
pub fn pty_name(p: u8) -> &'static str {
    const N: [&str; 32] = [
        "None", "News", "Current affairs", "Information", "Sport", "Education", "Drama", "Culture", "Science", "Varied", "Pop music", "Rock music", "Easy listening", "Light classical", "Serious classical", "Other music",
        "Weather", "Finance", "Children", "Social affairs", "Religion", "Phone-in", "Travel", "Leisure", "Jazz music", "Country music", "National music", "Oldies music", "Folk music", "Documentary", "Alarm test", "Alarm",
    ];
    N[(p & 31) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mpxdsp::{MpxEncoder, MpxParams, RdsData};

    fn run(stereo: bool, rds_level: f32, noise: f32, seconds: usize) -> RdsInfo {
        let params = MpxParams { stereo, rds_level, ..MpxParams::default() };
        let rds = RdsData { enabled: true, pi: 0x2C1D, ps: "MOON FM".into(), rt: "Artist - Title of the song".into(), pty: 10, tp: true, ta: false, music: true, ..RdsData::default() };
        let mut gen = MpxEncoder::new(44_100, 192_000, params, rds);
        let mut dec = RdsDecoder::new(192_000);
        let mut seed = 5u32;
        let mut out = Vec::new();
        for blk in 0..seconds * 100 {
            // music-like stereo input: tones + noise
            let inp: Vec<f32> = (0..441)
                .flat_map(|i| {
                    let t = (blk * 441 + i) as f32 / 44_100.0;
                    [0.5 * (6.283 * 220.0 * t).sin(), 0.5 * (6.283 * 1760.0 * t).sin()]
                })
                .collect();
            out.clear();
            gen.process(&inp, &mut out);
            if noise > 0.0 {
                for v in out.iter_mut() {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    *v += ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 2.0 * noise;
                }
            }
            dec.push(&out);
        }
        dec.info.clone()
    }

    #[test]
    fn decodes_stereo_mpx() {
        let i = run(true, 0.04, 0.0, 8);
        eprintln!("{i:?}");
        assert!(i.synced);
        assert_eq!(i.pi, Some(0x2C1D));
        assert_eq!(i.ps, "MOON FM ");
        assert_eq!(i.rt, "Artist - Title of the song");
        assert_eq!((i.pty, i.tp, i.ta, i.music), (Some(10), true, false, true));
        assert!(i.quality > 0.9);
    }

    #[test]
    fn decodes_af_ct_and_long_ps() {
        use crate::mpxdsp::{parse_af, PsPager};
        let params = MpxParams::default();
        let rds = RdsData { af: parse_af("94.3, 101.5 105.0"), ct: true, ct_offset: 6, ..RdsData::default() };
        let mut gen = MpxEncoder::new(44_100, 192_000, params, rds);
        let mut dec = RdsDecoder::new(192_000);
        let mut out = Vec::new();
        let pages = PsPager::pages("RADIO MOON FM ATHENS", "pages");
        assert_eq!(pages, vec![" RADIO  ".to_string(), "MOON FM ".to_string(), " ATHENS ".to_string()]);
        let mut seen = std::collections::HashSet::new();
        for blk in 0..1200 {
            // a new page every 2 s
            let page = pages[(blk / 200) % pages.len()].clone();
            gen.set_rds(RdsData { ps: page, ..gen.comp.rds.data.clone() });
            let inp: Vec<f32> = (0..441).flat_map(|i| [0.3 * (6.283 * 440.0 * (blk * 441 + i) as f32 / 44_100.0).sin(); 2]).collect();
            out.clear();
            gen.process(&inp, &mut out);
            dec.push(&out);
            seen.insert(dec.info.ps.clone());
        }
        let i = &dec.info;
        eprintln!("{i:?} pages seen {seen:?}");
        assert_eq!(i.af, vec![94.3, 101.5, 105.0]);
        assert!(i.ct.contains("(UTC+3)"), "{}", i.ct);
        for p in &pages {
            assert!(seen.contains(p), "page {p:?} never shown complete");
        }
        // never a mix of two pages
        assert!(seen.iter().all(|s| s.trim().is_empty() || pages.contains(s)), "{seen:?}");
    }

    #[test]
    fn decodes_mono_mpx_without_pilot() {
        let i = run(false, 0.04, 0.0, 8);
        assert_eq!(i.ps, "MOON FM ");
        assert_eq!(i.pi, Some(0x2C1D));
    }

    #[test]
    fn decodes_weak_rds_with_noise() {
        let i = run(true, 0.02, 0.05, 12);
        eprintln!("weak: {i:?}");
        assert_eq!(i.ps, "MOON FM ");
    }
}
