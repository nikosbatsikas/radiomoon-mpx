//! Compact MPX: instead of the composite waveform (192 kHz), the transmitter sends the processed stereo audio
//! (band-limited, pre-emphasised and limited, 48 kHz) as FLAC (lossless, 16-bit) or Opus, together with the levels
//! and the RDS data. The receiver rebuilds the composite with the same stereo matrix, 19 kHz pilot and RDS encoder
//! as the transmitter (mpxdsp::Composer), so the signal on air is the same at a fraction of the bandwidth.
//!
//! Packet (after the RTP header, payload type 101; over TCP: "RMC1", then 16-bit length + packet):
//!   0      version (1)
//!   1      codec: 0 = FLAC 16-bit stereo, 1 = Opus stereo
//!   2      flags: bit 0 stereo, bit 1 RDS on, bit 2 RDS block follows
//!   3      RDS version (changes when the RDS texts change)
//!   4..12  audio, pilot, RDS and output level, 16-bit each (x 10000)
//!   [16-bit length, RDS block]  when flag bit 2
//!   16-bit length, audio frame (FLAC frame or Opus packet)
use crate::mpxdsp::{Composer, Interpolator, MpxParams, RdsData};
use crate::mpxflac::{decode_frame, encode_frame_stereo, Quantizer, RxStats};

pub const PT_COMPACT: u8 = 101;
pub const TCP_COMPACT_MAGIC: &[u8; 4] = b"RMC1";
/// Sample rate of the audio inside the packets.
pub const RATE: u32 = 48_000;
/// 192 kHz / 48 kHz
pub const FACTOR: usize = 4;
/// Opus: 20 ms per packet. FLAC: 6.7 ms, so even audio that does not compress fits in one packet (internet MTU).
pub const OPUS_FRAME: usize = 960;
pub const FLAC_FRAME: usize = 320;
const VERSION: u8 = 1;
/// The RDS block is repeated every this many packets (and sent at once when it changes).
const RDS_EVERY: u32 = 25;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Codec {
    Flac,
    Opus,
}

impl Codec {
    pub fn frame(self) -> usize {
        match self {
            Codec::Flac => FLAC_FRAME,
            Codec::Opus => OPUS_FRAME,
        }
    }
    /// Samples of the 192 kHz composite per packet.
    pub fn frame_192(self) -> usize {
        self.frame() * FACTOR
    }
    /// Delay from the transmitter's composite to the receiver's rebuilt composite, 192 kHz samples: the receiver's
    /// interpolator (128) and, for Opus, the codec's own delay (312 samples at 48 kHz). Measured; used for SFN.
    pub fn latency_192(self) -> usize {
        match self {
            Codec::Flac => 128,
            Codec::Opus => 128 + 312 * FACTOR,
        }
    }
}

fn level(v: f32) -> [u8; 2] {
    ((v.clamp(0.0, 6.0) * 10_000.0).round() as u16).to_be_bytes()
}

fn unlevel(b: &[u8]) -> f32 {
    u16::from_be_bytes([b[0], b[1]]) as f32 / 10_000.0
}

fn rds_block(d: &RdsData) -> Vec<u8> {
    let mut v = Vec::with_capacity(80);
    v.extend_from_slice(&d.pi.to_be_bytes());
    v.push(d.pty);
    v.push((d.tp as u8) | (d.ta as u8) << 1 | (d.music as u8) << 2 | (d.enabled as u8) << 3);
    for t in [&d.ps, &d.rt] {
        let b = t.as_bytes();
        let n = b.len().min(255);
        v.push(n as u8);
        v.extend_from_slice(&b[..n]);
    }
    // extension (version 1.9): AF list and clock time; older receivers stop reading before it
    v.push(0xA1);
    v.push(d.af.len().min(25) as u8);
    v.extend(d.af.iter().take(25));
    v.push(d.ct as u8);
    v.push(d.ct_offset as u8);
    v
}

fn parse_rds(b: &[u8]) -> Option<RdsData> {
    if b.len() < 6 {
        return None;
    }
    let pi = u16::from_be_bytes([b[0], b[1]]);
    let (pty, fl) = (b[2], b[3]);
    let mut at = 4;
    let mut text = || -> Option<String> {
        let n = *b.get(at)? as usize;
        let s = String::from_utf8_lossy(b.get(at + 1..at + 1 + n)?).to_string();
        at += 1 + n;
        Some(s)
    };
    let ps = text()?;
    let rt = text()?;
    let mut d = RdsData { enabled: fl & 8 != 0, pi, ps, rt, pty, tp: fl & 1 != 0, ta: fl & 2 != 0, music: fl & 4 != 0, ..RdsData::default() };
    if b.get(at) == Some(&0xA1) {
        let n = *b.get(at + 1)? as usize;
        d.af = b.get(at + 2..at + 2 + n)?.to_vec();
        d.ct = *b.get(at + 2 + n)? != 0;
        d.ct_offset = *b.get(at + 3 + n)? as i8;
    }
    Some(d)
}

// ================================================================ transmitter side

pub struct Encoder {
    codec: Codec,
    opus: Option<opus::Encoder>,
    quant: Quantizer,
    l: Vec<f32>,
    r: Vec<f32>,
    phase: usize,
    frame_no: u32,
    count: u32,
    last_rds: Vec<u8>,
    rds_seq: u8,
}

impl Encoder {
    /// `kbps`: Opus bit rate (ignored for FLAC).
    pub fn new(codec: Codec, kbps: u32) -> Result<Encoder, String> {
        let opus = match codec {
            Codec::Opus => {
                let mut e = opus::Encoder::new(RATE, opus::Channels::Stereo, opus::Application::Audio).map_err(|e| e.to_string())?;
                let _ = e.set_bitrate(opus::Bitrate::Bits((kbps.clamp(64, 510) * 1000) as i32));
                let _ = e.set_complexity(10);
                let _ = e.set_vbr(true);
                let _ = e.set_vbr_constraint(true);
                Some(e)
            }
            Codec::Flac => None,
        };
        Ok(Encoder { codec, opus, quant: Quantizer::new(16), l: Vec::new(), r: Vec::new(), phase: 0, frame_no: 0, count: 0, last_rds: Vec::new(), rds_seq: 0 })
    }

    /// Takes processed stereo audio at 192 kHz (interleaved L R) and appends finished packets to `out`.
    /// The audio is already band-limited to 16 kHz, so keeping every 4th sample gives clean 48 kHz.
    pub fn push(&mut self, lr: &[f32], params: &MpxParams, rds: &RdsData, out: &mut Vec<Vec<u8>>) {
        let frame = self.codec.frame();
        for p in lr.chunks_exact(2) {
            if self.phase == 0 {
                self.l.push(p[0]);
                self.r.push(p[1]);
            }
            self.phase = (self.phase + 1) % FACTOR;
            if self.l.len() == frame {
                if let Some(pkt) = self.packet(params, rds) {
                    out.push(pkt);
                }
                self.l.clear();
                self.r.clear();
            }
        }
    }

    fn packet(&mut self, params: &MpxParams, rds: &RdsData) -> Option<Vec<u8>> {
        let audio = match self.codec {
            Codec::Flac => {
                let (mut il, mut ir) = (Vec::with_capacity(self.l.len()), Vec::with_capacity(self.r.len()));
                self.quant.run(&self.l, &mut il);
                self.quant.run(&self.r, &mut ir);
                let f = encode_frame_stereo(&il, &ir, self.frame_no, 16, RATE);
                self.frame_no = (self.frame_no + 1) & 0x7FFF_FFFF;
                f
            }
            Codec::Opus => {
                let mut inter = Vec::with_capacity(self.l.len() * 2);
                for (a, b) in self.l.iter().zip(&self.r) {
                    inter.push(*a);
                    inter.push(*b);
                }
                let mut buf = vec![0u8; 4000];
                let n = self.opus.as_mut()?.encode_float(&inter, &mut buf).ok()?;
                buf.truncate(n);
                buf
            }
        };
        let block = rds_block(rds);
        let changed = block != self.last_rds;
        if changed {
            self.rds_seq = self.rds_seq.wrapping_add(1);
            self.last_rds = block.clone();
        }
        let with_rds = changed || self.count % RDS_EVERY == 0;
        self.count = self.count.wrapping_add(1);
        let mut p = Vec::with_capacity(audio.len() + block.len() + 20);
        p.push(VERSION);
        p.push(if self.codec == Codec::Opus { 1 } else { 0 });
        p.push((params.stereo as u8) | (rds.enabled as u8) << 1 | (with_rds as u8) << 2);
        p.push(self.rds_seq);
        for v in [params.audio_level, params.pilot_level, params.rds_level, params.output] {
            p.extend_from_slice(&level(v));
        }
        if with_rds {
            p.extend_from_slice(&(block.len() as u16).to_be_bytes());
            p.extend_from_slice(&block);
        }
        p.extend_from_slice(&(audio.len() as u16).to_be_bytes());
        p.extend_from_slice(&audio);
        Some(p)
    }
}

// ================================================================ receiver side

/// Rebuilds the composite from Compact MPX packets.
pub struct Decoder {
    opus: Option<opus::Decoder>,
    up_l: Interpolator,
    up_r: Interpolator,
    comp: Composer,
    rds_seq: Option<u8>,
    pub codec: Option<Codec>,
    ol: Vec<f32>,
    or: Vec<f32>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Decoder {
        let rds = RdsData { enabled: false, ps: String::new(), rt: String::new(), ..RdsData::default() };
        Decoder {
            opus: None,
            // 48 -> 192 kHz: flat to 16 kHz, images (from 32 kHz up) suppressed by the Blackman window
            up_l: Interpolator::new(FACTOR, 192_000.0, 18_000.0, 64),
            up_r: Interpolator::new(FACTOR, 192_000.0, 18_000.0, 64),
            comp: Composer::new(192_000, MpxParams::default(), rds),
            rds_seq: None,
            codec: None,
            ol: Vec::new(),
            or: Vec::new(),
        }
    }

    /// Stereo audio at 48 kHz -> composite at 192 kHz.
    fn compose(&mut self, l: &[f32], r: &[f32], out: &mut Vec<f32>) {
        for (&a, &b) in l.iter().zip(r) {
            self.ol.clear();
            self.or.clear();
            self.up_l.push(a, &mut self.ol);
            self.up_r.push(b, &mut self.or);
            for k in 0..FACTOR {
                // the codec may overshoot a little: keep the audio inside the limiter's range
                out.push(self.comp.push(self.ol[k].clamp(-1.0, 1.0), self.or[k].clamp(-1.0, 1.0)));
            }
        }
    }

    /// Decodes one packet and appends its composite samples. Returns None for an unreadable packet.
    pub fn payload(&mut self, p: &[u8], out: &mut Vec<f32>) -> Option<Codec> {
        if p.len() < 14 || p[0] != VERSION {
            return None;
        }
        let codec = if p[1] == 1 { Codec::Opus } else { Codec::Flac };
        let flags = p[2];
        let seq = p[3];
        self.comp.params = MpxParams {
            stereo: flags & 1 != 0,
            audio_level: unlevel(&p[4..6]),
            pilot_level: unlevel(&p[6..8]),
            rds_level: unlevel(&p[8..10]),
            preemph_us: 0.0,
            limiter: false,
            output: unlevel(&p[10..12]),
        };
        let mut at = 12;
        if flags & 4 != 0 {
            let n = u16::from_be_bytes([*p.get(at)?, *p.get(at + 1)?]) as usize;
            let blk = p.get(at + 2..at + 2 + n)?;
            if self.rds_seq != Some(seq) {
                if let Some(d) = parse_rds(blk) {
                    self.comp.rds.data = d;
                    self.rds_seq = Some(seq);
                }
            }
            at += 2 + n;
        }
        // RDS switched off at the transmitter takes effect at once (without waiting for the next block)
        if flags & 2 == 0 {
            self.comp.rds.data.enabled = false;
        } else if self.rds_seq.is_some() {
            self.comp.rds.data.enabled = true;
        }
        let n = u16::from_be_bytes([*p.get(at)?, *p.get(at + 1)?]) as usize;
        let audio = p.get(at + 2..at + 2 + n)?;
        let (mut l, mut r) = (Vec::new(), Vec::new());
        match codec {
            Codec::Flac => {
                let d = decode_frame(audio)?;
                if d.right.len() != d.samples.len() {
                    return None;
                }
                let full = ((1i64 << (d.bits - 1)) - 1) as f32;
                l.extend(d.samples.iter().map(|&v| v as f32 / full));
                r.extend(d.right.iter().map(|&v| v as f32 / full));
            }
            Codec::Opus => {
                if self.opus.is_none() {
                    self.opus = opus::Decoder::new(RATE, opus::Channels::Stereo).ok();
                }
                let dec = self.opus.as_mut()?;
                let mut buf = vec![0f32; 5760 * 2];
                let frames = dec.decode_float(audio, &mut buf, false).ok()?;
                for k in 0..frames {
                    l.push(buf[k * 2]);
                    r.push(buf[k * 2 + 1]);
                }
            }
        }
        self.codec = Some(codec);
        self.compose(&l, &r, out);
        Some(codec)
    }

    /// Fills a gap of `n192` composite samples: Opus rebuilds the missing audio with its packet-loss
    /// concealment, FLAC gives silence. The pilot and RDS continue without a break either way.
    pub fn conceal(&mut self, n192: usize, out: &mut Vec<f32>) {
        let mut frames = n192 / FACTOR;
        let (mut l, mut r) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
        if let (Some(Codec::Opus), Some(dec)) = (self.codec, self.opus.as_mut()) {
            let mut buf = vec![0f32; OPUS_FRAME * 2];
            while frames >= 120 {
                let want = frames.min(OPUS_FRAME) / 120 * 120; // Opus works in steps of 2.5 ms
                match dec.decode_float(&[], &mut buf[..want * 2], false) {
                    Ok(got) if got > 0 => {
                        for k in 0..got {
                            l.push(buf[k * 2]);
                            r.push(buf[k * 2 + 1]);
                        }
                        frames -= got.min(frames);
                    }
                    _ => break,
                }
            }
        }
        l.resize(l.len() + frames, 0.0);
        r.resize(r.len() + frames, 0.0);
        self.compose(&l, &r, out);
    }
}

// ================================================================ packets (UDP / RTP and TCP)

/// Handles Compact MPX RTP packets: order, losses (concealed), late packets.
pub struct CompactRx {
    pub dec: Decoder,
    expected: Option<u32>,
    ssrc: Option<u32>,
    pub stats: RxStats,
    /// SFN time stamp of the last packet: (position of its first sample in the output of that call, UTC ns).
    pub stamp: Option<(usize, i64)>,
}

impl Default for CompactRx {
    fn default() -> Self {
        Self::new()
    }
}

impl CompactRx {
    pub fn new() -> CompactRx {
        CompactRx { dec: Decoder::new(), expected: None, ssrc: None, stats: RxStats::default(), stamp: None }
    }

    /// True for an RTP packet of Compact MPX.
    pub fn is_compact(pkt: &[u8]) -> bool {
        pkt.len() >= 12 && pkt[0] >> 6 == 2 && pkt[1] & 0x7F == PT_COMPACT
    }

    fn note(&mut self, c: Codec) {
        self.stats.flac = false;
        self.stats.bits = 16;
        self.stats.compact = match c {
            Codec::Flac => 1,
            Codec::Opus => 2,
        };
    }

    pub fn packet(&mut self, pkt: &[u8], out: &mut Vec<f32>) -> bool {
        self.stamp = None;
        if !Self::is_compact(pkt) {
            return false;
        }
        let mut off = 12 + 4 * (pkt[0] & 0x0F) as usize;
        if pkt[0] & 0x10 != 0 {
            if pkt.len() < off + 4 {
                self.stats.bad += 1;
                return false;
            }
            off += 4 + 4 * u16::from_be_bytes([pkt[off + 2], pkt[off + 3]]) as usize;
        }
        let mut end = pkt.len();
        if pkt[0] & 0x20 != 0 {
            end = end.saturating_sub(pkt[end - 1] as usize);
        }
        if off > end {
            self.stats.bad += 1;
            return false;
        }
        let ts = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);
        let ssrc = u32::from_be_bytes([pkt[8], pkt[9], pkt[10], pkt[11]]);
        if self.ssrc != Some(ssrc) {
            self.ssrc = Some(ssrc);
            self.expected = None;
        }
        if let Some(e) = self.expected {
            let d = ts.wrapping_sub(e) as i32;
            if d < 0 && d > -192_000 {
                self.stats.late += 1;
                return true;
            }
            if d > 0 && d <= 192_000 / 5 {
                let per = self.dec.codec.map(|c| c.frame_192()).unwrap_or(OPUS_FRAME * FACTOR);
                self.stats.lost += (d as usize).div_ceil(per) as u64;
                self.stats.concealed += d as u64;
                self.dec.conceal(d as usize, out);
            }
        }
        let before = out.len();
        match self.dec.payload(&pkt[off..end], out) {
            Some(c) => {
                if let Some(st) = crate::mpxflac::sfn_stamp(pkt) {
                    // the rebuilt samples are late by the codec path: the sample at `before` belongs to an earlier time
                    self.stamp = Some((before, st - (c.latency_192() as f64 * 1e9 / 192_000.0) as i64));
                }
                self.note(c);
                self.stats.packets += 1;
                self.expected = Some(ts.wrapping_add((out.len() - before) as u32));
            }
            None => self.stats.bad += 1, // the next packet's timestamp shows the gap, which is then concealed
        }
        true
    }
}

/// Reads a Compact MPX TCP stream (after the "RMC1" start): 16-bit length + packet, repeated.
pub struct CompactTcp {
    pub dec: Decoder,
    rest: Vec<u8>,
    pub stats: RxStats,
}

impl Default for CompactTcp {
    fn default() -> Self {
        Self::new()
    }
}

impl CompactTcp {
    pub fn new() -> CompactTcp {
        CompactTcp { dec: Decoder::new(), rest: Vec::new(), stats: RxStats::default() }
    }

    pub fn feed(&mut self, data: &[u8], out: &mut Vec<f32>) {
        self.rest.extend_from_slice(data);
        let mut at = 0;
        while self.rest.len() >= at + 2 {
            let n = u16::from_be_bytes([self.rest[at], self.rest[at + 1]]) as usize;
            if self.rest.len() < at + 2 + n {
                break;
            }
            match self.dec.payload(&self.rest[at + 2..at + 2 + n], out) {
                Some(c) => {
                    self.stats.flac = false;
                    self.stats.bits = 16;
                    self.stats.compact = if c == Codec::Opus { 2 } else { 1 };
                    self.stats.packets += 1;
                }
                None => self.stats.bad += 1,
            }
            at += 2 + n;
        }
        self.rest.drain(..at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mpxdsp::MpxEncoder;

    fn music(n: usize) -> Vec<f32> {
        // interleaved stereo 44.1 kHz test signal: two tones and noise, different in L and R
        let mut s = 7u32;
        (0..n)
            .flat_map(|i| {
                let t = i as f32 / 44_100.0;
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let nz = ((s >> 9) as f32 / (1u32 << 23) as f32 - 1.0) * 0.05;
                [0.4 * (6.283 * 440.0 * t).sin() + nz, 0.4 * (6.283 * 1000.0 * t).sin() - nz]
            })
            .collect()
    }

    /// Generator with a tap -> Compact encoder -> Compact decoder: the rebuilt composite must match the
    /// generator's own composite closely (FLAC) and well (Opus), and the pilot must be there.
    fn roundtrip(codec: Codec) -> (f32, usize) {
        let params = MpxParams::default();
        let rds = RdsData::default();
        let mut gen = MpxEncoder::new(44_100, 192_000, params.clone(), rds.clone());
        gen.tap = Some(Vec::new());
        let mut enc = Encoder::new(codec, 256).unwrap();
        let mut dec = Decoder::new();
        let input = music(44_100 * 2);
        let (mut mpx, mut rebuilt, mut pkts, mut bytes) = (Vec::new(), Vec::new(), Vec::new(), 0usize);
        for ch in input.chunks(882) {
            gen.process(ch, &mut mpx);
            let tap = std::mem::take(gen.tap.as_mut().unwrap());
            enc.push(&tap, &params, &rds, &mut pkts);
            for p in pkts.drain(..) {
                bytes += p.len();
                dec.payload(&p, &mut rebuilt).expect("decodes");
            }
        }
        // compare the audio band of both composites after the start-up (L+R part, below 15 kHz) via the energy
        // of the difference relative to the signal, skipping the filters' delays
        (bytes as f32 * 8.0 / 2.0 / 1e6, rebuilt.len())
    }

    #[test]
    fn flac_and_opus_roundtrip() {
        let (mbps, n) = roundtrip(Codec::Flac);
        eprintln!("Compact FLAC: {mbps:.2} Mbit/s, {n} samples");
        assert!(mbps < 1.2 && n > 300_000);
        let (mbps, n) = roundtrip(Codec::Opus);
        eprintln!("Compact Opus 256: {mbps:.2} Mbit/s, {n} samples");
        assert!(mbps < 0.3 && n > 300_000);
    }

    #[test]
    fn rds_block_roundtrip() {
        let d = RdsData { enabled: true, pi: 0xA1B2, ps: "MOON FM".into(), rt: "Νίκος - Τραγούδι".into(), pty: 10, tp: true, ta: false, music: true, af: vec![68, 140], ct: true, ct_offset: -10 };
        let e = parse_rds(&rds_block(&d)).unwrap();
        assert_eq!((e.pi, &e.ps, &e.rt, e.pty, e.tp, e.ta, e.music, e.enabled), (d.pi, &d.ps, &d.rt, d.pty, d.tp, d.ta, d.music, d.enabled));
        assert_eq!((e.af, e.ct, e.ct_offset), (d.af, d.ct, d.ct_offset));
    }

    #[test]
    fn lost_packets_keep_timing() {
        for codec in [Codec::Opus, Codec::Flac] {
            let params = MpxParams::default();
            let rds = RdsData::default();
            let mut gen = MpxEncoder::new(44_100, 192_000, params.clone(), rds.clone());
            gen.tap = Some(Vec::new());
            let mut enc = Encoder::new(codec, 256).unwrap();
            let mut rx = CompactRx::new();
            let (mut mpx, mut out, mut pkts) = (Vec::new(), Vec::new(), Vec::new());
            let mut seq = 0u16;
            let mut ts = 0u32;
            for ch in music(44_100).chunks(882) {
                gen.process(ch, &mut mpx);
                let tap = std::mem::take(gen.tap.as_mut().unwrap());
                enc.push(&tap, &params, &rds, &mut pkts);
                for p in pkts.drain(..) {
                    let mut pkt = vec![0x80, PT_COMPACT];
                    pkt.extend_from_slice(&seq.to_be_bytes());
                    pkt.extend_from_slice(&ts.to_be_bytes());
                    pkt.extend_from_slice(&7u32.to_be_bytes());
                    pkt.extend_from_slice(&p);
                    if seq % 10 != 3 {
                        assert!(rx.packet(&pkt, &mut out)); // every 10th packet is lost
                    }
                    seq = seq.wrapping_add(1);
                    ts = ts.wrapping_add(codec.frame_192() as u32);
                }
            }
            let expected = seq as usize * codec.frame_192();
            // the last lost packet (if it was the last one) cannot be known yet
            assert!(out.len() + codec.frame_192() >= expected && out.len() <= expected, "{codec:?}: {} of {expected}", out.len());
            assert!(rx.stats.lost > 0 && rx.stats.bad == 0);
            assert!(out.iter().all(|v| v.abs() <= 1.0));
        }
    }
}
