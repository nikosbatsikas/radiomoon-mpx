//! Lossless compression of the MPX signal with FLAC.
//!
//! Every IP packet carries one complete, standard FLAC frame (24-bit, mono, 192 kHz, fixed block size), so a lost
//! packet never damages the next one. The frames are bit-exact: the decoded samples are exactly the 24-bit samples
//! that were encoded. No external library is needed; the frames follow the FLAC format specification (RFC 9639) and
//! can be decoded by any FLAC decoder once a stream header is put in front of them.
#![allow(dead_code)]

/// Samples per FLAC frame / IP packet: 2 ms. A multiple of 192 samples, so 19, 38 and 57 kHz complete a whole
/// number of cycles in one frame (used by the receiver to hide a lost packet without a phase jump).
pub const BLOCK: usize = 384;
pub const BPS: u32 = 24;
/// RTP payload types: 97 = L24 (uncompressed), 98 = FLAC.
/// FLAC payload: 1 byte flags, 16-bit length of the frame, the frame. Flag bit 0 = a copy of the previous frame
/// follows (optional redundancy; the receiver uses it to restore a single lost packet exactly).
pub const PT_L24: u8 = 97;
pub const PT_FLAC: u8 = 98;
/// Start of a FLAC stream over TCP; then every frame follows as a 16-bit length (big endian) and the frame.
pub const TCP_FLAC_MAGIC: &[u8; 4] = b"RMF1";
const MAX_LPC: usize = 32;
const QLP_PRECISION: u32 = 15;

// ---------------------------------------------------------------- CRCs

fn crc8(data: &[u8]) -> u8 {
    let mut c: u8 = 0;
    for &b in data {
        c ^= b;
        for _ in 0..8 {
            c = if c & 0x80 != 0 { (c << 1) ^ 0x07 } else { c << 1 };
        }
    }
    c
}

fn crc16(data: &[u8]) -> u16 {
    let mut c: u16 = 0;
    for &b in data {
        c ^= (b as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x8005 } else { c << 1 };
        }
    }
    c
}

// ---------------------------------------------------------------- bit writer / reader

struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    fn new(cap: usize) -> Self {
        BitWriter { out: Vec::with_capacity(cap), acc: 0, n: 0 }
    }
    /// Writes the low `bits` bits of `v` (bits <= 32).
    fn put(&mut self, v: u64, bits: u32) {
        if bits == 0 {
            return;
        }
        self.acc = (self.acc << bits) | (v & ((1u64 << bits) - 1));
        self.n += bits;
        while self.n >= 8 {
            self.n -= 8;
            self.out.push((self.acc >> self.n) as u8);
        }
        self.acc &= (1u64 << self.n) - 1;
    }
    fn put_signed(&mut self, v: i64, bits: u32) {
        self.put(v as u64, bits);
    }
    fn unary(&mut self, mut q: u64) {
        while q >= 32 {
            self.put(0, 32);
            q -= 32;
        }
        self.put(1, q as u32 + 1);
    }
    fn align(&mut self) {
        if self.n > 0 {
            self.put(0, 8 - self.n);
        }
    }
}

struct BitReader<'a> {
    d: &'a [u8],
    pos: usize, // in bits
}

impl<'a> BitReader<'a> {
    fn get(&mut self, bits: u32) -> Option<u64> {
        if bits == 0 {
            return Some(0);
        }
        if self.pos + bits as usize > self.d.len() * 8 {
            return None;
        }
        let mut v: u64 = 0;
        let mut left = bits;
        while left > 0 {
            let byte = self.d[self.pos / 8];
            let used = (self.pos % 8) as u32;
            let take = (8 - used).min(left);
            let part = (byte >> (8 - used - take)) & ((1u16 << take) - 1) as u8;
            v = (v << take) | part as u64;
            left -= take;
            self.pos += take as usize;
        }
        Some(v)
    }
    fn get_signed(&mut self, bits: u32) -> Option<i64> {
        let v = self.get(bits)?;
        if bits == 0 {
            return Some(0);
        }
        Some(((v << (64 - bits)) as i64) >> (64 - bits))
    }
    fn unary(&mut self) -> Option<u64> {
        let mut q = 0u64;
        loop {
            if self.pos >= self.d.len() * 8 {
                return None;
            }
            let bit = (self.d[self.pos / 8] >> (7 - self.pos % 8)) & 1;
            self.pos += 1;
            if bit == 1 {
                return Some(q);
            }
            q += 1;
            if q > (1 << 33) {
                return None;
            }
        }
    }
    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
}

// ---------------------------------------------------------------- residual coding (Rice)

fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

/// Best Rice parameter for one partition and its size in bits (without the parameter field).
fn best_param(res: &[i64]) -> (u32, u64) {
    let n = res.len() as u64;
    if n == 0 {
        return (0, 0);
    }
    let sum: u64 = res.iter().map(|&v| zigzag(v)).sum();
    let mean = sum / n;
    let guess = if mean == 0 { 0 } else { 64 - mean.leading_zeros() };
    let mut best = (0u32, u64::MAX);
    for k in guess.saturating_sub(2).min(30)..=(guess + 1).min(30) {
        let bits: u64 = res.iter().map(|&v| (zigzag(v) >> k) + 1 + k as u64).sum();
        if bits < best.1 {
            best = (k, bits);
        }
    }
    best
}

/// Chooses the partition order and parameters; returns (bits, partition order, parameters).
fn plan_residual(res: &[i64], pred_order: usize) -> (u64, u32, Vec<u32>) {
    let n = res.len() + pred_order;
    let mut best: (u64, u32, Vec<u32>) = (u64::MAX, 0, Vec::new());
    for po in 0..=8u32 {
        let parts = 1usize << po;
        if n % parts != 0 || n / parts < pred_order || n / parts == 0 {
            break;
        }
        let psize = n / parts;
        let mut bits = 2 + 4u64;
        let mut params = Vec::with_capacity(parts);
        let mut start = 0;
        for p in 0..parts {
            let len = if p == 0 { psize - pred_order } else { psize };
            let (k, b) = best_param(&res[start..start + len]);
            start += len;
            params.push(k);
            bits += 5 + b;
        }
        if bits < best.0 {
            best = (bits, po, params);
        }
    }
    best
}

fn write_residual(w: &mut BitWriter, res: &[i64], pred_order: usize, po: u32, params: &[u32]) {
    let rice2 = params.iter().any(|&k| k > 14);
    w.put(if rice2 { 1 } else { 0 }, 2);
    w.put(po as u64, 4);
    let n = res.len() + pred_order;
    let psize = n >> po;
    let pbits = if rice2 { 5 } else { 4 };
    let mut start = 0;
    for (p, &k) in params.iter().enumerate() {
        let len = if p == 0 { psize - pred_order } else { psize };
        w.put(k as u64, pbits);
        for &v in &res[start..start + len] {
            let u = zigzag(v);
            w.unary(u >> k);
            w.put(u & ((1u64 << k) - 1), k);
        }
        start += len;
    }
}

// ---------------------------------------------------------------- predictors

fn fixed_residual(x: &[i64], order: usize) -> Vec<i64> {
    (order..x.len())
        .map(|i| match order {
            0 => x[i],
            1 => x[i] - x[i - 1],
            2 => x[i] - 2 * x[i - 1] + x[i - 2],
            3 => x[i] - 3 * x[i - 1] + 3 * x[i - 2] - x[i - 3],
            _ => x[i] - 4 * x[i - 1] + 6 * x[i - 2] - 4 * x[i - 3] + x[i - 4],
        })
        .collect()
}

fn lpc_residual(x: &[i64], q: &[i32], shift: u32) -> Vec<i64> {
    let o = q.len();
    (o..x.len())
        .map(|i| {
            let mut s: i64 = 0;
            for (j, &c) in q.iter().enumerate() {
                s += c as i64 * x[i - j - 1];
            }
            x[i] - (s >> shift)
        })
        .collect()
}

/// Linear prediction coefficients for every order up to `max` (Levinson-Durbin on a windowed autocorrelation).
fn lpc_sets(x: &[i64], max: usize) -> Vec<Vec<f64>> {
    let n = x.len();
    // Tukey(0.5) window
    let w: Vec<f64> = (0..n)
        .map(|i| {
            let a = 0.25 * n as f64;
            let t = i as f64;
            if t < a {
                0.5 * (1.0 - (std::f64::consts::PI * t / a).cos())
            } else if t > n as f64 - 1.0 - a {
                0.5 * (1.0 - (std::f64::consts::PI * (n as f64 - 1.0 - t) / a).cos())
            } else {
                1.0
            }
        })
        .collect();
    let xs: Vec<f64> = x.iter().zip(&w).map(|(&v, &w)| v as f64 * w).collect();
    let mut r = vec![0.0f64; max + 1];
    for (lag, rl) in r.iter_mut().enumerate() {
        *rl = (lag..n).map(|i| xs[i] * xs[i - lag]).sum();
    }
    let mut sets = Vec::new();
    if r[0] <= 0.0 {
        return sets;
    }
    let mut a = vec![0.0f64; max];
    let mut err = r[0];
    for i in 0..max {
        let mut acc = r[i + 1];
        for j in 0..i {
            acc -= a[j] * r[i - j];
        }
        let k = acc / err;
        let prev = a.clone();
        a[i] = k;
        for j in 0..i {
            a[j] = prev[j] - k * prev[i - 1 - j];
        }
        err *= 1.0 - k * k;
        sets.push(a[..=i].to_vec());
        if err <= 0.0 {
            break;
        }
    }
    sets
}

/// Quantizes coefficients as libFLAC does; returns (coefficients, shift).
fn quantize(lp: &[f64], precision: u32) -> Option<(Vec<i32>, u32)> {
    let cmax = lp.iter().fold(0.0f64, |m, &c| m.max(c.abs()));
    if cmax <= 0.0 || !cmax.is_finite() {
        return None;
    }
    let log2cmax = cmax.log2().floor() as i32 + 1;
    let shift = (precision as i32 - 1 - log2cmax).clamp(0, 15) as u32;
    let qmax = (1i64 << (precision - 1)) - 1;
    let qmin = -(1i64 << (precision - 1));
    let mut e = 0.0f64;
    let mut q = Vec::with_capacity(lp.len());
    for &c in lp {
        e += c * (1u64 << shift) as f64;
        let v = (e.round() as i64).clamp(qmin, qmax);
        e -= v as f64;
        q.push(v as i32);
    }
    Some((q, shift))
}

// ---------------------------------------------------------------- encoder

enum Sub {
    Constant,
    Verbatim,
    Fixed(usize),
    Lpc(Vec<i32>, u32),
}

fn utf8_number(w: &mut BitWriter, v: u32) {
    let v = v & 0x7FFF_FFFF;
    if v < 0x80 {
        w.put(v as u64, 8);
        return;
    }
    let len = match v {
        0..=0x7FF => 2,
        0x800..=0xFFFF => 3,
        0x1_0000..=0x1F_FFFF => 4,
        0x20_0000..=0x3FF_FFFF => 5,
        _ => 6,
    };
    let lead_mark: u64 = (0xFF00u64 >> len) & 0xFF;
    w.put(lead_mark | (v as u64 >> (6 * (len - 1))), 8);
    for i in (0..len - 1).rev() {
        w.put(0x80 | ((v as u64 >> (6 * i)) & 0x3F), 8);
    }
}

/// Encodes one block of 24-bit samples (-8388608..=8388607) as a complete FLAC frame.
/// `frame_no` is the frame number (fixed block size stream); the block length may be 1..=65536.
pub fn encode_frame(samples: &[i32], frame_no: u32) -> Vec<u8> {
    let n = samples.len();
    let x: Vec<i64> = samples.iter().map(|&s| s.clamp(-8_388_608, 8_388_607) as i64).collect();

    // choose the smallest subframe
    let verbatim_bits = n as u64 * BPS as u64;
    let mut best: (u64, Sub, Vec<i64>, u32, Vec<u32>) = (verbatim_bits, Sub::Verbatim, Vec::new(), 0, Vec::new());
    if n > 0 && x.iter().all(|&v| v == x[0]) {
        best = (BPS as u64, Sub::Constant, Vec::new(), 0, Vec::new());
    } else {
        for order in 0..=4usize.min(n.saturating_sub(1)) {
            let res = fixed_residual(&x, order);
            let (bits, po, params) = plan_residual(&res, order);
            let total = bits.saturating_add(order as u64 * BPS as u64);
            if total < best.0 {
                best = (total, Sub::Fixed(order), res, po, params);
            }
        }
        if n > MAX_LPC * 2 {
            let sets = lpc_sets(&x, MAX_LPC);
            for &order in &[2usize, 4, 8, 12, 16, 20, 24, 32] {
                let Some(lp) = sets.get(order - 1) else { continue };
                let Some((q, shift)) = quantize(lp, QLP_PRECISION) else { continue };
                let res = lpc_residual(&x, &q, shift);
                if res.iter().any(|&r| r > i32::MAX as i64 || r < i32::MIN as i64 + 1) {
                    continue;
                }
                let (bits, po, params) = plan_residual(&res, order);
                let total = bits.saturating_add(order as u64 * (BPS as u64 + QLP_PRECISION as u64) + 4 + 5);
                if total < best.0 {
                    best = (total, Sub::Lpc(q, shift), res, po, params);
                }
            }
        }
    }

    let mut w = BitWriter::new(n * 3 + 32);
    // ---- frame header
    w.put(0b11111111111110, 14);
    w.put(0, 1); // reserved
    w.put(0, 1); // fixed block size
    w.put(0b0111, 4); // block size: 16 bits at end of header
    w.put(0b0011, 4); // 192 kHz
    w.put(0b0000, 4); // mono
    w.put(0b110, 3); // 24 bits
    w.put(0, 1);
    utf8_number(&mut w, frame_no);
    w.put((n - 1) as u64, 16);
    let c8 = crc8(&w.out);
    w.put(c8 as u64, 8);
    // ---- subframe
    match &best.1 {
        Sub::Constant => {
            w.put(0, 8);
            w.put_signed(x[0], BPS);
        }
        Sub::Verbatim => {
            w.put(0b00000010, 8);
            for &v in &x {
                w.put_signed(v, BPS);
            }
        }
        Sub::Fixed(order) => {
            w.put((0b001000 | *order as u64) << 1, 8);
            for &v in &x[..*order] {
                w.put_signed(v, BPS);
            }
            write_residual(&mut w, &best.2, *order, best.3, &best.4);
        }
        Sub::Lpc(q, shift) => {
            let order = q.len();
            w.put((0b100000 | (order as u64 - 1)) << 1, 8);
            for &v in &x[..order] {
                w.put_signed(v, BPS);
            }
            w.put(QLP_PRECISION as u64 - 1, 4);
            w.put(*shift as u64, 5);
            for &c in q {
                w.put_signed(c as i64, QLP_PRECISION);
            }
            write_residual(&mut w, &best.2, order, best.3, &best.4);
        }
    }
    // ---- footer
    w.align();
    let c16 = crc16(&w.out);
    w.put(c16 as u64, 16);
    w.out
}

// ---------------------------------------------------------------- decoder

/// Result of decoding: the frame number and the samples (24-bit range).
pub struct Decoded {
    pub frame_no: u32,
    pub samples: Vec<i32>,
}

fn read_utf8(r: &mut BitReader) -> Option<u32> {
    let first = r.get(8)? as u32;
    if first & 0x80 == 0 {
        return Some(first);
    }
    let len = (first as u8).leading_ones() as usize;
    if !(2..=6).contains(&len) {
        return None;
    }
    let mut v = first & (0x7F >> len);
    for _ in 1..len {
        let b = r.get(8)? as u32;
        if b & 0xC0 != 0x80 {
            return None;
        }
        v = (v << 6) | (b & 0x3F);
    }
    Some(v)
}

fn read_residual(r: &mut BitReader, n: usize, order: usize, out: &mut Vec<i64>) -> Option<()> {
    let method = r.get(2)?;
    if method > 1 {
        return None;
    }
    let pbits = if method == 1 { 5 } else { 4 };
    let escape = (1u64 << pbits) - 1;
    let po = r.get(4)? as u32;
    let parts = 1usize << po;
    if n % parts != 0 || n / parts < order {
        return None;
    }
    let psize = n / parts;
    for p in 0..parts {
        let len = if p == 0 { psize - order } else { psize };
        let k = r.get(pbits)?;
        if k == escape {
            let raw = r.get(5)? as u32;
            for _ in 0..len {
                out.push(r.get_signed(raw)?);
            }
        } else {
            let k = k as u32;
            for _ in 0..len {
                let q = r.unary()?;
                let u = (q << k) | r.get(k)?;
                out.push(((u >> 1) as i64) ^ -((u & 1) as i64));
            }
        }
    }
    Some(())
}

/// Decodes one complete mono FLAC frame. Returns None when the frame is damaged (CRC) or not supported.
pub fn decode_frame(d: &[u8]) -> Option<Decoded> {
    if d.len() < 8 || crc16(&d[..d.len() - 2]) != u16::from_be_bytes([d[d.len() - 2], d[d.len() - 1]]) {
        return None;
    }
    let mut r = BitReader { d, pos: 0 };
    if r.get(14)? != 0b11111111111110 || r.get(1)? != 0 {
        return None;
    }
    let _variable = r.get(1)?;
    let bs_code = r.get(4)?;
    let sr_code = r.get(4)?;
    if r.get(4)? != 0 {
        return None; // mono only
    }
    let bps = match r.get(3)? {
        0b001 => 8,
        0b010 => 12,
        0b100 => 16,
        0b101 => 20,
        0b110 => 24,
        0b111 => 32,
        0b000 => BPS, // "from the stream header": this stream is always 24-bit
        _ => return None,
    };
    r.get(1)?;
    let frame_no = read_utf8(&mut r)?;
    let n = match bs_code {
        0b0001 => 192,
        2..=5 => 576 << (bs_code - 2),
        0b0110 => r.get(8)? as usize + 1,
        0b0111 => r.get(16)? as usize + 1,
        8..=15 => 256 << (bs_code - 8),
        _ => return None,
    };
    match sr_code {
        0b1100 => {
            r.get(8)?;
        }
        0b1101 | 0b1110 => {
            r.get(16)?;
        }
        0b1111 => return None,
        _ => {}
    }
    let hdr_end = r.pos / 8;
    if crc8(&d[..hdr_end]) != r.get(8)? as u8 {
        return None;
    }
    // ---- subframe
    if r.get(1)? != 0 {
        return None;
    }
    let typ = r.get(6)?;
    let mut wasted = 0u32;
    if r.get(1)? == 1 {
        wasted = r.unary()? as u32 + 1;
    }
    let sbps = bps - wasted;
    let mut x: Vec<i64> = Vec::with_capacity(n);
    match typ {
        0 => {
            let v = r.get_signed(sbps)?;
            x.resize(n, v);
        }
        1 => {
            for _ in 0..n {
                x.push(r.get_signed(sbps)?);
            }
        }
        8..=12 => {
            let order = (typ - 8) as usize;
            if order > n {
                return None;
            }
            for _ in 0..order {
                x.push(r.get_signed(sbps)?);
            }
            let mut res = Vec::with_capacity(n);
            read_residual(&mut r, n, order, &mut res)?;
            for e in res {
                let i = x.len();
                let p = match order {
                    0 => 0,
                    1 => x[i - 1],
                    2 => 2 * x[i - 1] - x[i - 2],
                    3 => 3 * x[i - 1] - 3 * x[i - 2] + x[i - 3],
                    _ => 4 * x[i - 1] - 6 * x[i - 2] + 4 * x[i - 3] - x[i - 4],
                };
                x.push(p + e);
            }
        }
        32..=63 => {
            let order = (typ - 31) as usize;
            if order > n {
                return None;
            }
            for _ in 0..order {
                x.push(r.get_signed(sbps)?);
            }
            let prec = r.get(4)? as u32 + 1;
            if prec == 16 {
                return None;
            }
            let shift = r.get_signed(5)?;
            if shift < 0 {
                return None;
            }
            let mut q = Vec::with_capacity(order);
            for _ in 0..order {
                q.push(r.get_signed(prec)?);
            }
            let mut res = Vec::with_capacity(n);
            read_residual(&mut r, n, order, &mut res)?;
            for e in res {
                let i = x.len();
                let mut s: i64 = 0;
                for (j, &c) in q.iter().enumerate() {
                    s += c * x[i - j - 1];
                }
                x.push((s >> shift) + e);
            }
        }
        _ => return None,
    }
    r.align();
    if r.pos / 8 + 2 != d.len() {
        return None;
    }
    let samples = x.iter().map(|&v| (v << wasted) as i32).collect();
    Some(Decoded { frame_no, samples })
}

// ---------------------------------------------------------------- float helpers (same scaling as the L24 path)

pub fn to_i24(samples: &[f32], out: &mut Vec<i32>) {
    out.extend(samples.iter().map(|s| (s.clamp(-1.0, 1.0) * 8_388_607.0) as i32));
}

pub fn from_i24(samples: &[i32], out: &mut Vec<f32>) {
    out.extend(samples.iter().map(|&v| v as f32 / 8_388_607.0));
}

/// The FLAC stream header (marker + STREAMINFO) for a 24-bit mono 192 kHz stream; put in front of the frames it
/// makes a .flac file that any player can open (used for tests and recordings).
#[allow(dead_code)]
pub fn stream_header(total_samples: u64) -> Vec<u8> {
    let mut w = BitWriter::new(42);
    w.put(u32::from_be_bytes(*b"fLaC") as u64, 32);
    w.put(0x80, 8); // last metadata block, type 0 = STREAMINFO
    w.put(34, 24);
    w.put(BLOCK as u64, 16);
    w.put(BLOCK as u64, 16);
    w.put(0, 24);
    w.put(0, 24);
    w.put(192_000, 20);
    w.put(0, 3); // 1 channel
    w.put(BPS as u64 - 1, 5);
    w.put(total_samples >> 32, 4);
    w.put(total_samples & 0xFFFF_FFFF, 32);
    for _ in 0..4 {
        w.put(0, 32); // MD5 not computed
    }
    w.out
}

// ---------------------------------------------------------------- receiving side (pure logic, used by mpxrx.rs)

#[derive(Clone, Copy, Default, Debug)]
pub struct RxStats {
    pub packets: u64,
    /// packets that never arrived
    pub lost: u64,
    /// lost packets restored exactly from the redundant copy in the next packet
    pub recovered: u64,
    /// samples replaced by the concealment (repeat of the last 2 ms)
    pub concealed: u64,
    /// damaged or unreadable packets
    pub bad: u64,
    /// packets that came too late or twice (ignored)
    pub late: u64,
    pub flac: bool,
}

/// Keeps the last `BLOCK` output samples. Because BLOCK is a multiple of 192 samples, repeating them continues the
/// 19 kHz pilot, the 38 kHz subcarrier and the 57 kHz RDS carrier without a phase jump, so a lost packet does not
/// make the receivers switch to mono or lose RDS; the audio part only repeats for a few milliseconds.
struct History {
    buf: Vec<f32>,
    pos: usize,
    fill: usize,
}

impl History {
    fn new() -> Self {
        History { buf: vec![0.0; BLOCK], pos: 0, fill: 0 }
    }
    fn push(&mut self, s: &[f32], out: &mut Vec<f32>) {
        out.extend_from_slice(s);
        for &v in s {
            self.buf[self.pos] = v;
            self.pos = (self.pos + 1) % BLOCK;
        }
        self.fill = (self.fill + s.len()).min(BLOCK);
    }
    fn conceal(&mut self, n: usize, out: &mut Vec<f32>) {
        if self.fill < BLOCK {
            out.extend(std::iter::repeat(0.0).take(n));
            return;
        }
        // the ring keeps its content: the oldest sample sits at `pos`, so reading on from there repeats the last
        // BLOCK samples in order, as many times as needed
        out.extend((0..n).map(|i| self.buf[(self.pos + i) % BLOCK]));
    }
}

/// Turns RTP packets (L24 or FLAC) into samples in the right order: drops late copies, restores single losses from
/// the redundant frame and fills longer gaps (up to 200 ms) with the concealment, so the timing stays exact.
pub struct Depacketizer {
    expected: Option<u32>,
    ssrc: Option<u32>,
    hist: History,
    pub stats: RxStats,
}

impl Default for Depacketizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Depacketizer {
    pub fn new() -> Self {
        Depacketizer { expected: None, ssrc: None, hist: History::new(), stats: RxStats::default() }
    }

    /// Handles one UDP datagram; appends the samples to play to `out`. Returns false for a packet that is not RTP.
    pub fn packet(&mut self, pkt: &[u8], out: &mut Vec<f32>) -> bool {
        if pkt.len() < 12 || pkt[0] >> 6 != 2 {
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
        let pt = pkt[1] & 0x7F;
        let ts = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);
        let ssrc = u32::from_be_bytes([pkt[8], pkt[9], pkt[10], pkt[11]]);
        let payload = &pkt[off..end];
        if self.ssrc != Some(ssrc) {
            self.ssrc = Some(ssrc); // a new sender or a restart: start over
            self.expected = None;
        }
        let mut cur: Vec<f32> = Vec::with_capacity(BLOCK);
        let mut prev: Option<Vec<f32>> = None;
        if pt == PT_FLAC {
            self.stats.flac = true;
            if payload.len() < 3 {
                self.stats.bad += 1;
                return true;
            }
            let len = u16::from_be_bytes([payload[1], payload[2]]) as usize;
            let Some(frame) = payload.get(3..3 + len) else {
                self.stats.bad += 1;
                return true;
            };
            match decode_frame(frame) {
                Some(d) => from_i24(&d.samples, &mut cur),
                None => {
                    self.stats.bad += 1; // treated like a lost packet: the next one restores or conceals it
                    return true;
                }
            }
            if payload[0] & 1 != 0 {
                if let Some(d) = decode_frame(&payload[3 + len..]) {
                    let mut p = Vec::with_capacity(d.samples.len());
                    from_i24(&d.samples, &mut p);
                    prev = Some(p);
                }
            }
        } else {
            self.stats.flac = false;
            for c in payload.chunks_exact(3) {
                let v = (((c[0] as i32) << 24) | ((c[1] as i32) << 16) | ((c[2] as i32) << 8)) >> 8;
                cur.push(v as f32 / 8_388_607.0);
            }
        }
        if cur.is_empty() {
            return true;
        }
        self.stats.packets += 1;
        let n = cur.len();
        if let Some(e) = self.expected {
            let d = ts.wrapping_sub(e) as i32;
            if d < 0 && d > -192_000 {
                self.stats.late += 1; // already played (or concealed)
                return true;
            }
            if d > 0 && d <= 192_000 / 5 {
                let mut gap = d as usize;
                self.stats.lost += gap.div_ceil(n) as u64;
                if let Some(p) = prev.as_ref().filter(|p| p.len() <= gap) {
                    let before = gap - p.len();
                    if before > 0 {
                        self.hist.conceal(before, out);
                        self.stats.concealed += before as u64;
                    }
                    self.hist.push(p, out);
                    self.stats.recovered += 1;
                    gap = 0;
                }
                if gap > 0 {
                    self.hist.conceal(gap, out);
                    self.stats.concealed += gap as u64;
                }
            }
            // a jump of more than 200 ms (or back in time): continue from here without filling
        }
        self.hist.push(&cur, out);
        self.expected = Some(ts.wrapping_add(n as u32));
        true
    }
}

/// Reads a TCP stream: FLAC when it starts with TCP_FLAC_MAGIC, otherwise raw L24.
pub struct TcpDeframer {
    flac: Option<bool>,
    rest: Vec<u8>,
    pub stats: RxStats,
}

impl Default for TcpDeframer {
    fn default() -> Self {
        Self::new()
    }
}

impl TcpDeframer {
    pub fn new() -> Self {
        TcpDeframer { flac: None, rest: Vec::new(), stats: RxStats::default() }
    }

    pub fn feed(&mut self, data: &[u8], out: &mut Vec<f32>) {
        self.rest.extend_from_slice(data);
        if self.flac.is_none() {
            if self.rest.len() < 4 {
                return;
            }
            let f = &self.rest[..4] == TCP_FLAC_MAGIC;
            if f {
                self.rest.drain(..4);
            }
            self.flac = Some(f);
            self.stats.flac = f;
        }
        if self.flac == Some(true) {
            let mut at = 0;
            while self.rest.len() >= at + 2 {
                let len = u16::from_be_bytes([self.rest[at], self.rest[at + 1]]) as usize;
                if self.rest.len() < at + 2 + len {
                    break;
                }
                match decode_frame(&self.rest[at + 2..at + 2 + len]) {
                    Some(d) => {
                        from_i24(&d.samples, out);
                        self.stats.packets += 1;
                    }
                    None => self.stats.bad += 1,
                }
                at += 2 + len;
            }
            self.rest.drain(..at);
        } else {
            let usable = self.rest.len() / 3 * 3;
            for c in self.rest[..usable].chunks_exact(3) {
                let v = (((c[0] as i32) << 24) | ((c[1] as i32) << 16) | ((c[2] as i32) << 8)) >> 8;
                out.push(v as f32 / 8_388_607.0);
            }
            self.rest.drain(..usable);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rng(seed: &mut u64) -> f64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }

    /// A composite-like test signal: band-limited noise as L+R and L-R, 19 kHz pilot, 38 kHz DSB, 57 kHz carrier.
    fn mpx_like(n: usize, seed: u64) -> Vec<i32> {
        let mut s = seed;
        let (mut a, mut b, mut c, mut d) = (0.0, 0.0, 0.0, 0.0);
        (0..n)
            .map(|i| {
                let t = i as f64 / 192_000.0;
                a = 0.9 * a + 0.1 * rng(&mut s);
                b = 0.9 * b + 0.1 * a;
                c = 0.9 * c + 0.1 * rng(&mut s);
                d = 0.9 * d + 0.1 * c;
                let tau = std::f64::consts::TAU;
                let v = 1.2 * b + 0.6 * d * (tau * 38_000.0 * t).sin() + 0.09 * (tau * 19_000.0 * t).sin() + 0.03 * (tau * 57_000.0 * t).sin();
                (v.clamp(-1.0, 1.0) * 8_388_607.0) as i32
            })
            .collect()
    }

    fn roundtrip(x: &[i32]) -> usize {
        let mut total = 0;
        for (no, blk) in x.chunks(BLOCK).enumerate() {
            let f = encode_frame(blk, no as u32);
            total += f.len();
            let d = decode_frame(&f).expect("decodes");
            assert_eq!(d.frame_no, no as u32);
            assert_eq!(d.samples, blk, "bit-exact");
        }
        total
    }

    #[test]
    fn lossless_mpx() {
        let x = mpx_like(BLOCK * 500, 7);
        let bytes = roundtrip(&x);
        let ratio = (x.len() * 3) as f64 / bytes as f64;
        eprintln!("compression ratio {ratio:.2}");
        assert!(ratio > 1.1);
    }

    #[test]
    fn lossless_edge_cases() {
        roundtrip(&vec![0; BLOCK]);
        roundtrip(&vec![8_388_607; BLOCK]);
        roundtrip(&(0..BLOCK).map(|i| if i % 2 == 0 { 8_388_607 } else { -8_388_608 }).collect::<Vec<_>>());
        let mut s = 99u64;
        roundtrip(&(0..BLOCK * 4).map(|_| (rng(&mut s) * 8_388_607.0) as i32).collect::<Vec<_>>());
        roundtrip(&[1, 2, 3]);
        roundtrip(&[5]);
    }

    #[test]
    fn frame_numbers() {
        for no in [0u32, 127, 128, 2047, 2048, 65535, 65536, 0x1F_FFFF, 0x20_0000, 0x7FFF_FFFF] {
            let f = encode_frame(&[1, 2, 3, 4], no);
            assert_eq!(decode_frame(&f).unwrap().frame_no, no);
        }
    }

    #[test]
    fn damaged_frame_is_rejected() {
        let mut f = encode_frame(&mpx_like(BLOCK, 3), 1);
        let m = f.len() / 2;
        f[m] ^= 0x10;
        assert!(decode_frame(&f).is_none());
    }

    fn rtp(pt: u8, seq: u16, ts: u32, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![0x80, pt];
        p.extend_from_slice(&seq.to_be_bytes());
        p.extend_from_slice(&ts.to_be_bytes());
        p.extend_from_slice(&0x1234_5678u32.to_be_bytes());
        p.extend_from_slice(payload);
        p
    }

    /// Builds FLAC RTP packets the way the sender does (with the previous frame as redundancy).
    fn flac_packets(x: &[i32], redundancy: bool) -> Vec<Vec<u8>> {
        let mut prev: Vec<u8> = Vec::new();
        x.chunks(BLOCK)
            .enumerate()
            .map(|(i, b)| {
                let f = encode_frame(b, i as u32);
                let with_prev = redundancy && !prev.is_empty();
                let mut pl = vec![with_prev as u8];
                pl.extend_from_slice(&(f.len() as u16).to_be_bytes());
                pl.extend_from_slice(&f);
                if with_prev {
                    pl.extend_from_slice(&prev);
                }
                prev = f;
                rtp(PT_FLAC, i as u16, (i * BLOCK) as u32, &pl)
            })
            .collect()
    }

    fn as_f32(x: &[i32]) -> Vec<f32> {
        let mut v = Vec::new();
        from_i24(x, &mut v);
        v
    }

    #[test]
    fn rtp_flac_in_order_is_exact() {
        let x = mpx_like(BLOCK * 20, 5);
        let mut d = Depacketizer::new();
        let mut out = Vec::new();
        for p in flac_packets(&x, true) {
            d.packet(&p, &mut out);
        }
        assert_eq!(out, as_f32(&x));
        assert_eq!(d.stats.lost, 0);
    }

    #[test]
    fn single_loss_is_restored_exactly_and_duplicates_ignored() {
        let x = mpx_like(BLOCK * 20, 6);
        let pk = flac_packets(&x, true);
        let mut d = Depacketizer::new();
        let mut out = Vec::new();
        for (i, p) in pk.iter().enumerate() {
            if i == 7 || i == 12 {
                continue; // lost
            }
            d.packet(p, &mut out);
            if i == 9 {
                d.packet(p, &mut out); // duplicate
                d.packet(&pk[3], &mut out); // very late
            }
        }
        assert_eq!(out, as_f32(&x), "bit-exact despite two single losses");
        assert_eq!((d.stats.lost, d.stats.recovered, d.stats.concealed, d.stats.late), (2, 2, 0, 2));
    }

    #[test]
    fn double_loss_keeps_timing_and_pilot_phase() {
        // a pure 19 kHz pilot: the concealment must continue it exactly
        let x: Vec<i32> = (0..BLOCK * 20).map(|i| ((std::f64::consts::TAU * 19_000.0 * i as f64 / 192_000.0).sin() * 800_000.0) as i32).collect();
        let pk = flac_packets(&x, true);
        let mut d = Depacketizer::new();
        let mut out = Vec::new();
        for (i, p) in pk.iter().enumerate() {
            if (5..=7).contains(&i) {
                continue; // three in a row: 5 and 6 concealed, 7 restored from packet 8
            }
            d.packet(p, &mut out);
        }
        assert_eq!(out.len(), x.len(), "timing kept");
        let err = out.iter().zip(as_f32(&x)).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(err < 1e-6, "pilot continues without a phase jump (error {err})");
        assert_eq!((d.stats.lost, d.stats.recovered, d.stats.concealed), (3, 1, (BLOCK * 2) as u64));
    }

    #[test]
    fn l24_packets_still_work() {
        let x = mpx_like(240 * 10, 8);
        let mut d = Depacketizer::new();
        let mut out = Vec::new();
        for (i, b) in x.chunks(240).enumerate() {
            let mut pl = Vec::new();
            for &v in b {
                pl.extend_from_slice(&[(v >> 16) as u8, (v >> 8) as u8, v as u8]);
            }
            if i != 4 {
                d.packet(&rtp(PT_L24, i as u16, (i * 240) as u32, &pl), &mut out);
            }
        }
        assert_eq!(out.len(), x.len());
        assert_eq!(&out[..240 * 4], &as_f32(&x)[..240 * 4]);
        assert_eq!(&out[240 * 5..], &as_f32(&x)[240 * 5..]);
    }

    #[test]
    fn tcp_flac_and_l24() {
        let x = mpx_like(BLOCK * 10, 9);
        let mut stream = TCP_FLAC_MAGIC.to_vec();
        for (i, b) in x.chunks(BLOCK).enumerate() {
            let f = encode_frame(b, i as u32);
            stream.extend_from_slice(&(f.len() as u16).to_be_bytes());
            stream.extend_from_slice(&f);
        }
        let mut t = TcpDeframer::new();
        let mut out = Vec::new();
        for piece in stream.chunks(777) {
            t.feed(piece, &mut out);
        }
        assert!(t.stats.flac);
        assert_eq!(out, as_f32(&x));
        let mut raw = Vec::new();
        for &v in &x {
            raw.extend_from_slice(&[(v >> 16) as u8, (v >> 8) as u8, v as u8]);
        }
        let mut t = TcpDeframer::new();
        let mut out = Vec::new();
        for piece in raw.chunks(1001) {
            t.feed(piece, &mut out);
        }
        assert!(!t.stats.flac);
        assert_eq!(out, as_f32(&x));
    }
}
