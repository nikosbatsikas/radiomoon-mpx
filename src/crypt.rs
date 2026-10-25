//! Cryptography for the protected MPX stream, without external crates:
//! ChaCha20-Poly1305 (RFC 8439, the AEAD of TLS 1.3 and WireGuard) and PBKDF2-HMAC-SHA256 (RFC 8018) to turn the
//! password into the key. Checked against the RFC test vectors and against the Python `cryptography` package.

// ---------------------------------------------------------------- SHA-256, HMAC, PBKDF2

const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

#[derive(Clone)]
pub struct Sha256 {
    h: [u32; 8],
    buf: [u8; 64],
    n: usize,
    len: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Sha256 {
        Sha256 { h: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19], buf: [0; 64], n: 0, len: 0 }
    }

    fn block(h: &mut [u32; 8], b: &[u8]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut bb, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K256[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & bb) ^ (a & c) ^ (bb & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = bb;
            bb = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, bb, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.len += data.len() as u64;
        if self.n > 0 {
            let k = (64 - self.n).min(data.len());
            self.buf[self.n..self.n + k].copy_from_slice(&data[..k]);
            self.n += k;
            data = &data[k..];
            if self.n < 64 {
                return;
            }
            let b = self.buf;
            Self::block(&mut self.h, &b);
            self.n = 0;
        }
        while data.len() >= 64 {
            Self::block(&mut self.h, &data[..64]);
            data = &data[64..];
        }
        self.buf[..data.len()].copy_from_slice(data);
        self.n = data.len();
    }

    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.n != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        let mut out = [0u8; 32];
        for (i, v) in self.h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
        }
        out
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut s = Sha256::new();
    s.update(data);
    s.finish()
}

/// HMAC-SHA256 with the key's inner and outer states prepared once (for PBKDF2's many rounds).
struct Hmac {
    inner: Sha256,
    outer: Sha256,
}

impl Hmac {
    fn new(key: &[u8]) -> Hmac {
        let mut k = [0u8; 64];
        if key.len() > 64 {
            k[..32].copy_from_slice(&sha256(key));
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        let (mut inner, mut outer) = (Sha256::new(), Sha256::new());
        inner.update(&k.map(|b| b ^ 0x36));
        outer.update(&k.map(|b| b ^ 0x5c));
        Hmac { inner, outer }
    }
    fn mac(&self, parts: &[&[u8]]) -> [u8; 32] {
        let mut i = self.inner.clone();
        for p in parts {
            i.update(p);
        }
        let mut o = self.outer.clone();
        o.update(&i.finish());
        o.finish()
    }
}

pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    Hmac::new(key).mac(&[data])
}

/// PBKDF2-HMAC-SHA256, one 32-byte block.
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], rounds: u32) -> [u8; 32] {
    let h = Hmac::new(password);
    let mut u = h.mac(&[salt, &1u32.to_be_bytes()]);
    let mut out = u;
    for _ in 1..rounds.max(1) {
        u = h.mac(&[&u]);
        for (o, x) in out.iter_mut().zip(u) {
            *o ^= x;
        }
    }
    out
}

/// The stream key of a password (the same in the Transmitter and the Receiver). Made once per password and
/// kept, as it takes some milliseconds.
pub fn stream_key(password: &str) -> [u8; 32] {
    static CACHE: std::sync::Mutex<Vec<([u8; 32], [u8; 32])>> = std::sync::Mutex::new(Vec::new());
    let id = sha256(password.as_bytes());
    if let Some(k) = CACHE.lock().ok().and_then(|c| c.iter().find(|e| e.0 == id).map(|e| e.1)) {
        return k;
    }
    let k = pbkdf2_sha256(password.as_bytes(), b"RadioMoon MPX stream key v1", 100_000);
    if let Ok(mut c) = CACHE.lock() {
        if c.len() > 16 {
            c.remove(0);
        }
        c.push((id, k));
    }
    k
}

// ---------------------------------------------------------------- ChaCha20

fn quarter(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = (s[d] ^ s[a]).rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_left(7);
}

fn chacha_block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let mut st = [0u32; 16];
    st[..4].copy_from_slice(&[0x61707865, 0x3320646e, 0x79622d32, 0x6b206574]);
    for i in 0..8 {
        st[4 + i] = u32::from_le_bytes([key[i * 4], key[i * 4 + 1], key[i * 4 + 2], key[i * 4 + 3]]);
    }
    st[12] = counter;
    for i in 0..3 {
        st[13 + i] = u32::from_le_bytes([nonce[i * 4], nonce[i * 4 + 1], nonce[i * 4 + 2], nonce[i * 4 + 3]]);
    }
    let mut w = st;
    for _ in 0..10 {
        quarter(&mut w, 0, 4, 8, 12);
        quarter(&mut w, 1, 5, 9, 13);
        quarter(&mut w, 2, 6, 10, 14);
        quarter(&mut w, 3, 7, 11, 15);
        quarter(&mut w, 0, 5, 10, 15);
        quarter(&mut w, 1, 6, 11, 12);
        quarter(&mut w, 2, 7, 8, 13);
        quarter(&mut w, 3, 4, 9, 14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        out[i * 4..i * 4 + 4].copy_from_slice(&w[i].wrapping_add(st[i]).to_le_bytes());
    }
    out
}

/// XORs `data` with the ChaCha20 key stream, starting at block `counter`.
pub fn chacha20_xor(key: &[u8; 32], counter: u32, nonce: &[u8; 12], data: &mut [u8]) {
    for (i, chunk) in data.chunks_mut(64).enumerate() {
        let ks = chacha_block(key, counter.wrapping_add(i as u32), nonce);
        for (d, k) in chunk.iter_mut().zip(ks.iter()) {
            *d ^= k;
        }
    }
}

// ---------------------------------------------------------------- Poly1305 (44-bit limbs)

struct Poly1305 {
    r: [u64; 3],
    s: [u64; 2],
    h: [u64; 3],
    buf: [u8; 16],
    n: usize,
}

const M44: u64 = (1 << 44) - 1;
const M42: u64 = (1 << 42) - 1;

fn le64(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

impl Poly1305 {
    fn new(key: &[u8; 32]) -> Poly1305 {
        let (t0, t1) = (le64(&key[0..8]), le64(&key[8..16]));
        let r = [t0 & 0xffc0fffffff, ((t0 >> 44) | (t1 << 20)) & 0xfffffc0ffff, (t1 >> 24) & 0x00ffffffc0f];
        Poly1305 { r, s: [le64(&key[16..24]), le64(&key[24..32])], h: [0; 3], buf: [0; 16], n: 0 }
    }

    fn block(&mut self, m: &[u8; 16], hibit: u64) {
        let [r0, r1, r2] = self.r;
        let (s1, s2) = (r1 * 20, r2 * 20);
        let (t0, t1) = (le64(&m[0..8]), le64(&m[8..16]));
        let [mut h0, mut h1, mut h2] = self.h;
        h0 += t0 & M44;
        h1 += ((t0 >> 44) | (t1 << 20)) & M44;
        h2 += ((t1 >> 24) & M42) | hibit;
        let m = |a: u64, b: u64| a as u128 * b as u128;
        let d0 = m(h0, r0) + m(h1, s2) + m(h2, s1);
        let mut d1 = m(h0, r1) + m(h1, r0) + m(h2, s2);
        let mut d2 = m(h0, r2) + m(h1, r1) + m(h2, r0);
        let mut c = (d0 >> 44) as u64;
        h0 = d0 as u64 & M44;
        d1 += c as u128;
        c = (d1 >> 44) as u64;
        h1 = d1 as u64 & M44;
        d2 += c as u128;
        c = (d2 >> 42) as u64;
        h2 = d2 as u64 & M42;
        h0 += c * 5;
        c = h0 >> 44;
        h0 &= M44;
        h1 += c;
        self.h = [h0, h1, h2];
    }

    fn update(&mut self, mut data: &[u8]) {
        if self.n > 0 {
            let k = (16 - self.n).min(data.len());
            self.buf[self.n..self.n + k].copy_from_slice(&data[..k]);
            self.n += k;
            data = &data[k..];
            if self.n < 16 {
                return;
            }
            let b = self.buf;
            self.block(&b, 1 << 40);
            self.n = 0;
        }
        while data.len() >= 16 {
            let mut b = [0u8; 16];
            b.copy_from_slice(&data[..16]);
            self.block(&b, 1 << 40);
            data = &data[16..];
        }
        self.buf[..data.len()].copy_from_slice(data);
        self.n = data.len();
    }

    fn finish(mut self) -> [u8; 16] {
        if self.n > 0 {
            let mut b = [0u8; 16];
            b[..self.n].copy_from_slice(&self.buf[..self.n]);
            b[self.n] = 1;
            self.block(&b, 0);
        }
        let [mut h0, mut h1, mut h2] = self.h;
        let mut c = h1 >> 44;
        h1 &= M44;
        h2 += c;
        c = h2 >> 42;
        h2 &= M42;
        h0 += c * 5;
        c = h0 >> 44;
        h0 &= M44;
        h1 += c;
        c = h1 >> 44;
        h1 &= M44;
        h2 += c;
        c = h2 >> 42;
        h2 &= M42;
        h0 += c * 5;
        c = h0 >> 44;
        h0 &= M44;
        h1 += c;
        // h - p
        let mut g0 = h0 + 5;
        c = g0 >> 44;
        g0 &= M44;
        let mut g1 = h1 + c;
        c = g1 >> 44;
        g1 &= M44;
        let g2 = (h2 + c).wrapping_sub(1 << 42);
        let mask = (g2 >> 63).wrapping_sub(1); // all ones when h >= p
        h0 = (h0 & !mask) | (g0 & mask);
        h1 = (h1 & !mask) | (g1 & mask);
        h2 = (h2 & !mask) | (g2 & mask);
        // + s
        let (t0, t1) = (self.s[0], self.s[1]);
        h0 += t0 & M44;
        c = h0 >> 44;
        h0 &= M44;
        h1 += (((t0 >> 44) | (t1 << 20)) & M44) + c;
        c = h1 >> 44;
        h1 &= M44;
        h2 += ((t1 >> 24) & M42) + c;
        h2 &= M42;
        let lo = h0 | (h1 << 44);
        let hi = (h1 >> 20) | (h2 << 24);
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&lo.to_le_bytes());
        out[8..].copy_from_slice(&hi.to_le_bytes());
        out
    }
}

pub fn poly1305(key: &[u8; 32], msg: &[u8]) -> [u8; 16] {
    let mut p = Poly1305::new(key);
    p.update(msg);
    p.finish()
}

// ---------------------------------------------------------------- AEAD

pub const TAG: usize = 16;

fn aead_tag(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], ct: &[u8]) -> [u8; 16] {
    let block = chacha_block(key, 0, nonce);
    let mut pk = [0u8; 32];
    pk.copy_from_slice(&block[..32]);
    let mut p = Poly1305::new(&pk);
    let zeros = [0u8; 16];
    p.update(aad);
    p.update(&zeros[..(16 - aad.len() % 16) % 16]);
    p.update(ct);
    p.update(&zeros[..(16 - ct.len() % 16) % 16]);
    p.update(&(aad.len() as u64).to_le_bytes());
    p.update(&(ct.len() as u64).to_le_bytes());
    p.finish()
}

/// Encrypts `data` in place and appends the 16-byte tag.
pub fn seal(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], data: &mut Vec<u8>) {
    chacha20_xor(key, 1, nonce, data);
    let tag = aead_tag(key, nonce, aad, data);
    data.extend_from_slice(&tag);
}

/// Checks the tag and decrypts. `data` = ciphertext + tag; returns the plain text, or None when the packet was not
/// made with this key (wrong password) or was changed on the way.
pub fn open(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < TAG {
        return None;
    }
    let (ct, tag) = data.split_at(data.len() - TAG);
    let want = aead_tag(key, nonce, aad, ct);
    // constant time comparison
    if want.iter().zip(tag).fold(0u8, |a, (x, y)| a | (x ^ y)) != 0 {
        return None;
    }
    let mut pt = ct.to_vec();
    chacha20_xor(key, 1, nonce, &mut pt);
    Some(pt)
}

/// Bytes that are different at every call (for nonces): from the operating system's random hash keys, the time,
/// the process and a counter.
pub fn random_bytes() -> [u8; 32] {
    use std::hash::{BuildHasher, Hasher};
    static COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut s = Sha256::new();
    for _ in 0..4 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        s.update(&h.finish().to_le_bytes());
    }
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    s.update(&t.to_le_bytes());
    s.update(&std::process::id().to_le_bytes());
    #[cfg(unix)]
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let mut b = [0u8; 32];
        if f.read_exact(&mut b).is_ok() {
            s.update(&b);
        }
    }
    s.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap()).collect()
    }

    #[test]
    fn sha256_vectors() {
        assert_eq!(sha256(b"abc").to_vec(), hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        assert_eq!(sha256(b"").to_vec(), hex("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"));
        assert_eq!(sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq").to_vec(), hex("248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"));
        // RFC 4231 test case 2
        assert_eq!(hmac_sha256(b"Jefe", b"what do ya want for nothing?").to_vec(), hex("5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"));
    }

    #[test]
    fn chacha20_poly1305_rfc8439() {
        // RFC 8439 2.5.2: Poly1305
        let k: [u8; 32] = hex("85d6be7857556d337f4452fe42d506a80103808afb0db2fd4abff6af4149f51b").try_into().unwrap();
        assert_eq!(poly1305(&k, b"Cryptographic Forum Research Group").to_vec(), hex("a8061dc1305136c6c22b8baf0c0127a9"));
        // RFC 8439 2.8.2: AEAD
        let key: [u8; 32] = hex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f").try_into().unwrap();
        let nonce: [u8; 12] = hex("070000004041424344454647").try_into().unwrap();
        let aad = hex("50515253c0c1c2c3c4c5c6c7");
        let pt = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let mut d = pt.to_vec();
        seal(&key, &nonce, &aad, &mut d);
        assert_eq!(&d[..16], &hex("d31a8d34648e60db7b86afbc53ef7ec2")[..]);
        assert_eq!(&d[d.len() - 16..], &hex("1ae10b594f09e26a7e902ecbd0600691")[..]);
        assert_eq!(open(&key, &nonce, &aad, &d).unwrap(), pt.to_vec());
        d[5] ^= 1;
        assert!(open(&key, &nonce, &aad, &d).is_none());
    }
}
