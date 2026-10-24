//! Stereo decoder for a ready composite signal (MPX input): recovers the stereo audio and measures the pilot, so
//! the Transmitter can send an MPX input as Compact MPX. The audio stays pre-emphasised (as in the composite),
//! which is what Compact MPX carries. Standard library only.
use std::f64::consts::PI;

/// Low-pass (windowed sinc, Blackman) that computes an output only every `m`-th input.
struct Lowpass {
    h: Vec<f32>,
    buf: Vec<f32>,
    pos: usize,
}

impl Lowpass {
    fn new(cut: f64, fs: f64, taps: usize) -> Lowpass {
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
        Lowpass { h: h.iter().map(|v| *v as f32).collect(), buf: vec![0.0; taps * 2], pos: 0 }
    }
    #[inline]
    fn put(&mut self, x: f32) {
        let n = self.h.len();
        self.buf[self.pos] = x;
        self.buf[self.pos + n] = x;
        self.pos += 1;
        if self.pos == n {
            self.pos = 0;
        }
    }
    #[inline]
    fn out(&self) -> f32 {
        let n = self.h.len();
        self.buf[self.pos..self.pos + n].iter().zip(&self.h).map(|(a, b)| a * b).sum()
    }
}

pub struct StereoDecoder {
    fs: f64,
    phase: f64, // pilot phase in cycles
    freq: f64,
    nominal: f64,
    err_lp: f64,
    amp_lp: f64,
    integ: f64,
    m: Lowpass,
    s: Lowpass,
    count: usize,
    hold: (f32, f32),
}

impl StereoDecoder {
    pub fn new(fs: u32) -> StereoDecoder {
        let fs = fs as f64;
        StereoDecoder {
            fs,
            phase: 0.0,
            freq: 19_000.0 / fs,
            nominal: 19_000.0 / fs,
            err_lp: 0.0,
            amp_lp: 0.0,
            integ: 0.0,
            m: Lowpass::new(15_500.0, fs, 255),
            s: Lowpass::new(15_500.0, fs, 255),
            count: 0,
            hold: (0.0, 0.0),
        }
    }

    /// Level of the 19 kHz pilot (1.0 = 75 kHz deviation).
    pub fn pilot_level(&self) -> f32 {
        self.amp_lp as f32
    }

    /// A stereo signal (pilot present).
    pub fn stereo(&self) -> bool {
        self.amp_lp > 0.01
    }

    /// Composite in (192 kHz), stereo audio out (interleaved L R, 192 kHz, at the composite's scale: the audio part
    /// of the composite is (L+R)/2 + (L-R)/2 * sin(2*pilot)). The filters are computed every 4th sample (all that
    /// Compact MPX uses) and held in between.
    pub fn push(&mut self, mpx: &[f32], out: &mut Vec<f32>) {
        // PLL of about 8 Hz: locks the local oscillator onto the pilot
        let wn = 2.0 * PI * 8.0 / self.fs;
        let (kp, ki) = (2.0 * 0.707 * wn, wn * wn);
        let stereo = self.stereo();
        for &x in mpx {
            let a = 2.0 * PI * self.phase;
            let (sn, cs) = a.sin_cos();
            // phase error and pilot amplitude (low-passed products)
            self.err_lp += (x as f64 * cs - self.err_lp) * 0.002;
            self.amp_lp += (2.0 * x as f64 * sn - self.amp_lp) * 0.0005;
            let e = if self.amp_lp.abs() > 1e-4 { self.err_lp / self.amp_lp.abs() } else { 0.0 };
            self.integ += ki * e;
            self.integ = self.integ.clamp(-2.0 * PI * 20.0 / self.fs, 2.0 * PI * 20.0 / self.fs);
            self.phase += self.freq + (kp * e + self.integ) / (2.0 * PI);
            self.phase -= self.phase.floor();
            // L+R and L-R (the 38 kHz subcarrier is twice the pilot phase)
            let sub = (2.0 * a).sin() as f32;
            self.m.put(x);
            self.s.put(if stereo { 2.0 * x * sub } else { 0.0 });
            if self.count == 0 {
                let (m, s) = (self.m.out(), self.s.out());
                self.hold = (m + s, m - s);
            }
            self.count = (self.count + 1) % 4;
            out.push(self.hold.0);
            out.push(self.hold.1);
        }
        let _ = self.nominal;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mpxdsp::{MpxEncoder, MpxParams, RdsData};

    #[test]
    fn recovers_pilot_and_stereo_audio() {
        let params = MpxParams { pilot_level: 0.09, audio_level: 0.85, ..MpxParams::default() };
        let mut gen = MpxEncoder::new(44_100, 192_000, params, RdsData::default());
        gen.tap = Some(Vec::new());
        let mut dec = StereoDecoder::new(192_000);
        let (mut mpx, mut lr) = (Vec::new(), Vec::new());
        // left: 1 kHz tone only, right: silence
        for blk in 0..400 {
            let inp: Vec<f32> = (0..441).flat_map(|i| [0.5 * (6.283 * 1000.0 * (blk * 441 + i) as f32 / 44_100.0).sin(), 0.0]).collect();
            mpx.clear();
            gen.process(&inp, &mut mpx);
            dec.push(&mpx, &mut lr);
        }
        assert!(dec.stereo());
        assert!((dec.pilot_level() - 0.09).abs() < 0.005, "pilot {}", dec.pilot_level());
        // last second: left carries the tone, right almost nothing (stereo separation)
        let tail = &lr[lr.len() - 192_000 * 2..];
        let rms = |c: usize| (tail.iter().skip(c).step_by(2).map(|v| v * v).sum::<f32>() / 192_000.0).sqrt();
        let (l, r) = (rms(0), rms(1));
        let sep = 20.0 * (l / r.max(1e-9)).log10();
        eprintln!("left {l:.4} right {r:.5} separation {sep:.1} dB");
        assert!(sep > 30.0);
    }
}
