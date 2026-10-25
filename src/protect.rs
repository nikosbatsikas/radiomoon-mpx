//! Protection of the MPX stream: encryption with a password, forward error correction (FEC) against lost UDP
//! packets, and the list of allowed senders.
//!
//! A protected UDP stream wraps every packet of the normal stream (RTP: L24 / L16 / L20 / FLAC / Compact MPX) in an
//! envelope, so the formats themselves stay as they are:
//!
//! ```text
//! data:  "RMSD" flags fec_n depth seq(4)  body                 body = packet, or nonce(12) + encrypted packet + tag(16)
//! FEC:   "RMSF" flags fec_n depth base(4) len_xor(2) parity    parity = XOR of the bodies of packets
//!                                                               base, base + depth, ... base + (fec_n - 1) * depth
//! ```
//! flags bit 0 = encrypted (ChaCha20-Poly1305, key = PBKDF2-HMAC-SHA256 of the password; the 11 header bytes are
//! authenticated with the packet). Every group of `fec_n` data packets gets one FEC packet, from which the receiver
//! restores one lost packet of the group. With `depth` > 1 (burst protection) the groups are interleaved: blocks
//! of fec_n x depth packets, each group takes every depth-th packet, so up to `depth` packets lost in a row are
//! all restored. TCP needs no FEC; encrypted TCP is "RMS1" flags, then
//! frames of length(4) + nonce(12) + encrypted bytes of the normal TCP stream + tag(16).
use crate::crypt::{self, open, seal, stream_key, TAG};
use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;

pub const DATA: &[u8; 4] = b"RMSD";
pub const FEC: &[u8; 4] = b"RMSF";
pub const TCP_MAGIC: &[u8; 4] = b"RMS1";
const HDR: usize = 11;
const FEC_HDR: usize = 13;
const ENCRYPTED: u8 = 1;
const TCP_AAD: &[u8] = b"RMST";

/// FEC group sizes offered in the settings (one FEC packet for every n data packets).
pub const FEC_CHOICES: [u8; 4] = [0, 20, 10, 5];
/// Burst protection offered in the settings: how many packets lost in a row can be restored.
pub const DEPTH_CHOICES: [u8; 4] = [1, 2, 4, 8];

pub fn fec_name(n: u8) -> String {
    match n {
        0 => "Off".into(),
        n => format!("1 per {n} packets (+{}%)", 100 / n as u32),
    }
}

/// Unique nonces: 8 random bytes (new at every start) + a 32-bit counter (new random bytes when it wraps).
struct Nonces {
    prefix: [u8; 8],
    ctr: u32,
}

impl Nonces {
    fn new() -> Nonces {
        let mut prefix = [0u8; 8];
        prefix.copy_from_slice(&crypt::random_bytes()[..8]);
        Nonces { prefix, ctr: 0 }
    }
    fn next(&mut self) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[..8].copy_from_slice(&self.prefix);
        n[8..].copy_from_slice(&self.ctr.to_be_bytes());
        self.ctr = self.ctr.wrapping_add(1);
        if self.ctr == 0 {
            *self = Nonces::new();
        }
        n
    }
}

fn data_header(flags: u8, fec_n: u8, depth: u8, seq: u32) -> [u8; HDR] {
    let mut h = [0u8; HDR];
    h[..4].copy_from_slice(DATA);
    h[4] = flags;
    h[5] = fec_n;
    h[6] = depth;
    h[7..].copy_from_slice(&seq.to_be_bytes());
    h
}

// ================================================================ sender

/// The protection of one output (one per IP target).
pub struct Protector {
    key: Option<[u8; 32]>,
    fec_n: u8,
    depth: u8,
    nonces: Nonces,
    seq: u32,
    /// Bodies of the current FEC groups (one per column of the block).
    groups: Vec<Vec<Vec<u8>>>,
    base: u32,
}

impl Protector {
    /// `password` empty = not encrypted; `fec_n` 0 = no FEC (UDP only); `depth` = burst protection (1 = off).
    pub fn new(password: &str, fec_n: u8, depth: u8) -> Protector {
        let key = (!password.is_empty()).then(|| stream_key(password));
        let depth = if fec_n == 0 { 1 } else { depth.clamp(1, 16) };
        Protector { key, fec_n: if fec_n == 1 { 2 } else { fec_n }, depth, nonces: Nonces::new(), seq: 0, groups: vec![Vec::new(); depth as usize], base: 0 }
    }

    pub fn encrypted(&self) -> bool {
        self.key.is_some()
    }

    /// Whether UDP packets get the envelope at all.
    pub fn udp_active(&self) -> bool {
        self.key.is_some() || self.fec_n > 0
    }

    fn flags(&self) -> u8 {
        if self.key.is_some() {
            ENCRYPTED
        } else {
            0
        }
    }

    /// One packet of the normal stream -> the datagrams to send (the data packet, and a FEC packet after each group).
    pub fn udp(&mut self, pkt: &[u8], out: &mut Vec<Vec<u8>>) {
        if !self.udp_active() {
            out.push(pkt.to_vec());
            return;
        }
        let (flags, n, depth, seq) = (self.flags(), self.fec_n, self.depth, self.seq);
        self.seq = self.seq.wrapping_add(1);
        let hdr = data_header(flags, n, depth, seq);
        let body = match self.key {
            Some(k) => {
                let nonce = self.nonces.next();
                let mut b = Vec::with_capacity(12 + pkt.len() + TAG);
                b.extend_from_slice(&nonce);
                let mut ct = pkt.to_vec();
                seal(&k, &nonce, &hdr, &mut ct);
                b.extend_from_slice(&ct);
                b
            }
            None => pkt.to_vec(),
        };
        let mut d = Vec::with_capacity(HDR + body.len());
        d.extend_from_slice(&hdr);
        d.extend_from_slice(&body);
        out.push(d);
        if n == 0 {
            return;
        }
        // blocks of n x depth packets start at multiples of that (a block cut by the wrap of the counter simply
        // gets no FEC packets); packet i of a block belongs to group i % depth
        let block = n as u32 * depth as u32;
        if seq % block == 0 {
            self.groups.iter_mut().for_each(|g| g.clear());
            self.base = seq;
        }
        let i = seq.wrapping_sub(self.base);
        if i >= block {
            return;
        }
        let (col, k) = ((i % depth as u32) as usize, (i / depth as u32) as usize);
        if self.groups[col].len() != k {
            return;
        }
        self.groups[col].push(body);
        if self.groups[col].len() == n as usize {
            let group = std::mem::take(&mut self.groups[col]);
            let max = group.iter().map(|b| b.len()).max().unwrap_or(0);
            let mut parity = vec![0u8; max];
            let mut len_xor = 0u16;
            for b in &group {
                len_xor ^= b.len() as u16;
                for (p, x) in parity.iter_mut().zip(b) {
                    *p ^= x;
                }
            }
            let mut f = Vec::with_capacity(FEC_HDR + max);
            f.extend_from_slice(FEC);
            f.push(flags);
            f.push(n);
            f.push(depth);
            f.extend_from_slice(&self.base.wrapping_add(col as u32).to_be_bytes());
            f.extend_from_slice(&len_xor.to_be_bytes());
            f.extend_from_slice(&parity);
            out.push(f);
        }
    }

    /// TCP: the bytes that start an encrypted stream (empty when not encrypted).
    pub fn tcp_start(&self) -> Vec<u8> {
        match self.key {
            Some(_) => [&TCP_MAGIC[..], &[ENCRYPTED]].concat(),
            None => Vec::new(),
        }
    }

    /// TCP: bytes of the normal stream -> the bytes to write.
    pub fn tcp(&mut self, data: &[u8]) -> Vec<u8> {
        let Some(k) = self.key else { return data.to_vec() };
        let nonce = self.nonces.next();
        let mut ct = data.to_vec();
        seal(&k, &nonce, TCP_AAD, &mut ct);
        let mut f = Vec::with_capacity(4 + 12 + ct.len());
        f.extend_from_slice(&((12 + ct.len()) as u32).to_be_bytes());
        f.extend_from_slice(&nonce);
        f.extend_from_slice(&ct);
        f
    }
}

// ================================================================ receiver

/// What the protection layer of the receiver saw (shown in the Receiver window).
#[derive(Clone, Default, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ProtStats {
    /// The stream is encrypted (and the password fits).
    pub encrypted: bool,
    /// FEC group size of the stream (0 = no FEC) and its burst protection (packets in a row).
    pub fec_n: u8,
    pub fec_depth: u8,
    /// Packets restored from FEC packets.
    pub restored: u64,
    /// Packets lost that FEC could not restore.
    pub lost: u64,
    /// Packets that failed the check: wrong password, or changed on the way.
    pub rejected: u64,
    /// Unencrypted packets ignored because a password is set.
    pub plain_rejected: u64,
    /// Encrypted packets arrived but no password is set.
    pub need_password: bool,
    /// Packets / connections refused because the sender is not in the list of allowed senders.
    pub blocked: u64,
    pub blocked_from: String,
}

/// Opens protected UDP packets: checks and decrypts, restores lost packets with FEC and returns the packets of
/// the normal stream in order.
pub struct Unprotector {
    key: Option<[u8; 32]>,
    pub stats: ProtStats,
    /// Bodies by sequence number (also the delivered ones of the last groups, for FEC).
    have: BTreeMap<u32, Vec<u8>>,
    fecs: HashMap<u32, (u16, Vec<u8>)>,
    /// Next sequence number to deliver.
    next: Option<u32>,
    /// Highest sequence number received.
    top: u32,
    flags: u8,
    fec_n: u8,
    depth: u8,
}

fn ahead(a: u32, b: u32) -> i64 {
    a.wrapping_sub(b) as i32 as i64
}

impl Unprotector {
    pub fn new(password: &str) -> Unprotector {
        let key = (!password.is_empty()).then(|| stream_key(password));
        Unprotector { key, stats: ProtStats::default(), have: BTreeMap::new(), fecs: HashMap::new(), next: None, top: 0, flags: 0, fec_n: 0, depth: 1 }
    }

    pub fn has_password(&self) -> bool {
        self.key.is_some()
    }

    pub fn is_protected(d: &[u8]) -> bool {
        d.len() >= HDR && (&d[..4] == DATA || &d[..4] == FEC)
    }

    /// Handles one datagram of a protected stream; appends the packets of the normal stream to play to `out`.
    pub fn udp(&mut self, d: &[u8], out: &mut Vec<Vec<u8>>) {
        if !Self::is_protected(d) {
            return;
        }
        let (flags, n, depth) = (d[4], d[5], d[6].max(1));
        if flags & ENCRYPTED == 0 && self.key.is_some() {
            self.stats.plain_rejected += 1;
            return;
        }
        if flags & ENCRYPTED != 0 && self.key.is_none() {
            self.stats.need_password = true;
            return;
        }
        self.stats.need_password = false;
        let seq = u32::from_be_bytes([d[7], d[8], d[9], d[10]]);
        let block = n as u32 * depth as u32;
        let fec = &d[..4] == FEC;
        if fec && d.len() < FEC_HDR {
            return;
        }
        // a new start of the sender (or a long break): begin again
        if let Some(nx) = self.next {
            let a = ahead(seq, nx);
            if !(-(4 * block.max(1) as i64 + 64)..=20_000).contains(&a) {
                self.have.clear();
                self.fecs.clear();
                self.next = None;
            }
        }
        if self.next.is_none() {
            // start at the beginning of a group, so the first group can be restored too
            self.next = Some(if n > 0 && !fec { seq - seq % block } else { seq });
            self.top = seq;
        }
        self.flags = flags;
        self.fec_n = n;
        self.depth = depth;
        self.stats.fec_n = n;
        self.stats.fec_depth = depth;
        let nx = self.next.unwrap_or(seq);
        if fec {
            if ahead(seq, nx) > -(block as i64) {
                let len_xor = u16::from_be_bytes([d[11], d[12]]);
                self.fecs.insert(seq, (len_xor, d[FEC_HDR..].to_vec()));
            }
        } else if ahead(seq, nx) >= 0 {
            self.have.entry(seq).or_insert_with(|| d[HDR..].to_vec());
            if ahead(seq, self.top) > 0 {
                self.top = seq;
            }
        } else if n > 0 && ahead(seq, nx) > -(block as i64) {
            // late, but still useful for FEC of its group
            self.have.entry(seq).or_insert_with(|| d[HDR..].to_vec());
        }
        self.flush(out);
    }

    /// Restores the missing packet `seq` from the FEC packet of its group, when exactly that one is missing.
    fn restore(&mut self, seq: u32) -> bool {
        let (n, depth) = (self.fec_n as u32, self.depth as u32);
        if n == 0 {
            return false;
        }
        let block = n * depth;
        let start = seq - seq % block;
        let base = start + (seq - start) % depth;
        let Some((len_xor, parity)) = self.fecs.get(&base) else { return false };
        let mut body = parity.clone();
        let mut len = *len_xor;
        for k in 0..n {
            let s = base.wrapping_add(k * depth);
            if s == seq {
                continue;
            }
            let Some(b) = self.have.get(&s) else { return false };
            len ^= b.len() as u16;
            for (p, x) in body.iter_mut().zip(b) {
                *p ^= x;
            }
        }
        if len as usize > body.len() {
            return false;
        }
        body.truncate(len as usize);
        self.have.insert(seq, body);
        self.stats.restored += 1;
        true
    }

    fn deliver(&mut self, seq: u32, out: &mut Vec<Vec<u8>>) {
        let Some(body) = self.have.get(&seq) else { return };
        if self.flags & ENCRYPTED != 0 {
            let Some(k) = self.key else { return };
            if body.len() < 12 + TAG {
                self.stats.rejected += 1;
                return;
            }
            let mut nonce = [0u8; 12];
            nonce.copy_from_slice(&body[..12]);
            match open(&k, &nonce, &data_header(self.flags, self.fec_n, self.depth, seq), &body[12..]) {
                Some(p) => {
                    self.stats.encrypted = true;
                    out.push(p);
                }
                None => {
                    self.stats.rejected += 1;
                    self.stats.encrypted = false;
                }
            }
        } else {
            self.stats.encrypted = false;
            out.push(body.clone());
        }
    }

    fn flush(&mut self, out: &mut Vec<Vec<u8>>) {
        let n = self.fec_n as u32;
        let block = n * self.depth as u32;
        while let Some(nx) = self.next {
            if ahead(self.top, nx) < 0 {
                break; // all delivered
            }
            if self.have.contains_key(&nx) || self.restore(nx) {
                self.deliver(nx, out);
                self.next = Some(nx.wrapping_add(1));
                continue;
            }
            // missing: wait for the rest of its group and its FEC packet, unless that cannot come any more
            let give_up = n == 0 || {
                let start = nx - nx % block;
                let base = start + (nx - start) % self.depth as u32;
                self.fecs.contains_key(&base) || ahead(self.top, start) >= 2 * block as i64 + 1
            };
            if !give_up {
                break;
            }
            self.stats.lost += 1;
            self.next = Some(nx.wrapping_add(1));
        }
        // forget what is no longer needed for FEC
        if let Some(nx) = self.next {
            let keep = 2 * block.max(1) as i64;
            self.have.retain(|s, _| ahead(*s, nx) >= -keep);
            self.fecs.retain(|s, _| ahead(*s, nx) >= -keep);
        }
    }
}

/// Opens an encrypted TCP stream: the bytes after "RMS1" + flags -> the bytes of the normal stream.
pub struct TcpUnwrap {
    key: [u8; 32],
    buf: Vec<u8>,
    pub rejected: u64,
}

impl TcpUnwrap {
    pub fn new(key: [u8; 32]) -> TcpUnwrap {
        TcpUnwrap { key, buf: Vec::new(), rejected: 0 }
    }
    /// Returns false when a frame failed the check (wrong password): the connection should be closed.
    pub fn feed(&mut self, data: &[u8], out: &mut Vec<u8>) -> bool {
        self.buf.extend_from_slice(data);
        let mut at = 0;
        while self.buf.len() >= at + 4 {
            let len = u32::from_be_bytes([self.buf[at], self.buf[at + 1], self.buf[at + 2], self.buf[at + 3]]) as usize;
            if !(12 + TAG..=1 << 20).contains(&len) {
                self.rejected += 1;
                return false;
            }
            if self.buf.len() < at + 4 + len {
                break;
            }
            let f = &self.buf[at + 4..at + 4 + len];
            let mut nonce = [0u8; 12];
            nonce.copy_from_slice(&f[..12]);
            match open(&self.key, &nonce, TCP_AAD, &f[12..]) {
                Some(p) => out.extend_from_slice(&p),
                None => {
                    self.rejected += 1;
                    return false;
                }
            }
            at += 4 + len;
        }
        self.buf.drain(..at);
        true
    }
}

/// The start of a TCP connection at the receiver: decides from the first bytes whether the stream is encrypted.
pub enum TcpGate {
    Start(Vec<u8>),
    Plain,
    Secure(TcpUnwrap),
}

/// What the gate did with the received bytes.
pub enum Gate {
    /// Bytes of the normal stream (maybe none yet).
    Data,
    /// Encrypted, but no password is set here.
    NeedPassword,
    /// Not encrypted, but a password is set here.
    PlainRejected,
    /// Wrong password or changed data.
    Rejected,
}

impl TcpGate {
    pub fn new() -> TcpGate {
        TcpGate::Start(Vec::new())
    }

    pub fn feed(&mut self, key: Option<&[u8; 32]>, data: &[u8], out: &mut Vec<u8>) -> Gate {
        if let TcpGate::Start(b) = self {
            b.extend_from_slice(data);
            if b.len() < 5 {
                return Gate::Data;
            }
            let first = std::mem::take(b);
            if &first[..4] == TCP_MAGIC {
                let Some(k) = key else { return Gate::NeedPassword };
                let mut u = TcpUnwrap::new(*k);
                let ok = u.feed(&first[5..], out);
                *self = TcpGate::Secure(u);
                return if ok { Gate::Data } else { Gate::Rejected };
            }
            if key.is_some() {
                return Gate::PlainRejected;
            }
            out.extend_from_slice(&first);
            *self = TcpGate::Plain;
            return Gate::Data;
        }
        match self {
            TcpGate::Plain => {
                out.extend_from_slice(data);
                Gate::Data
            }
            TcpGate::Secure(u) => {
                if u.feed(data, out) {
                    Gate::Data
                } else {
                    Gate::Rejected
                }
            }
            TcpGate::Start(_) => Gate::Data,
        }
    }
}

impl Default for TcpGate {
    fn default() -> Self {
        Self::new()
    }
}

// ================================================================ allowed senders

/// The list of allowed senders: addresses and networks, e.g. "192.168.1.10, 10.0.0.0/8, 2001:db8::/32".
/// Empty = everyone.
#[derive(Clone, Debug, Default)]
pub struct AllowList {
    nets: Vec<(IpAddr, u8)>,
}

impl AllowList {
    pub fn parse(text: &str) -> AllowList {
        let mut nets = Vec::new();
        for t in text.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).filter(|t| !t.is_empty()) {
            let (a, bits) = match t.split_once('/') {
                Some((a, b)) => (a, b.parse::<u8>().ok()),
                None => (t, None),
            };
            if let Ok(ip) = a.parse::<IpAddr>() {
                let max = if ip.is_ipv4() { 32 } else { 128 };
                nets.push((ip, bits.unwrap_or(max).min(max)));
            }
        }
        AllowList { nets }
    }

    /// Entries that are not valid addresses (shown as a warning).
    pub fn invalid(text: &str) -> Vec<String> {
        text.split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .filter(|t| {
                let (a, b) = t.split_once('/').unwrap_or((t, "0"));
                a.parse::<IpAddr>().is_err() || b.parse::<u8>().is_err()
            })
            .map(|t| t.to_string())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.nets.is_empty()
    }

    pub fn allows(&self, ip: IpAddr) -> bool {
        if self.nets.is_empty() {
            return true;
        }
        // IPv4 senders on a dual-stack socket appear as ::ffff:a.b.c.d
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
            v4 => v4,
        };
        self.nets.iter().any(|(net, bits)| match (net, ip) {
            (IpAddr::V4(n), IpAddr::V4(a)) => {
                let m = if *bits == 0 { 0 } else { u32::MAX << (32 - *bits as u32) };
                u32::from(*n) & m == u32::from(a) & m
            }
            (IpAddr::V6(n), IpAddr::V6(a)) => {
                let m = if *bits == 0 { 0 } else { u128::MAX << (128 - *bits as u32) };
                u128::from(*n) & m == u128::from(a) & m
            }
            _ => false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packets(k: usize) -> Vec<Vec<u8>> {
        (0..k).map(|i| (0..(50 + (i * 37) % 900)).map(|j| (i * 7 + j) as u8).collect()).collect()
    }

    fn run(password_tx: &str, password_rx: &str, fec: u8, depth: u8, lose: &dyn Fn(usize, bool) -> bool, k: usize) -> (Vec<Vec<u8>>, ProtStats) {
        let mut p = Protector::new(password_tx, fec, depth);
        let mut u = Unprotector::new(password_rx);
        let mut got = Vec::new();
        let mut wire = Vec::new();
        for pk in packets(k) {
            p.udp(&pk, &mut wire);
        }
        for (i, d) in wire.iter().enumerate() {
            if !lose(i, &d[..4] == FEC) {
                u.udp(d, &mut got);
            }
        }
        (got, u.stats.clone())
    }

    #[test]
    fn encrypted_and_fec_restore() {
        let src = packets(200);
        // no loss
        let (got, st) = run("secret", "secret", 10, 1, &|_, _| false, 200);
        assert_eq!(got, src);
        assert!(st.encrypted && st.restored == 0 && st.lost == 0);
        // one data packet lost in every group of 11 datagrams (10 data + 1 FEC): all restored, in order
        let (got, st) = run("secret", "secret", 10, 1, &|i, fec| !fec && i % 11 == 3, 200);
        assert_eq!(got, src);
        assert_eq!((st.restored, st.lost), (20, 0));
        // without encryption too
        let (got, st) = run("", "", 5, 1, &|i, fec| !fec && i % 6 == 0, 200);
        assert_eq!(got, src);
        assert_eq!(st.restored, 40);
        // two lost in a group: one cannot be restored
        let (got, st) = run("", "", 10, 1, &|i, _| i == 2 || i == 5, 200);
        assert_eq!(got.len(), 198);
        assert_eq!(st.lost, 2);
        // FEC packet lost as well: the gap is given up after the next group
        let (got, st) = run("", "", 10, 1, &|i, fec| i == 4 || (fec && i < 11), 200);
        assert_eq!(got.len(), 199);
        assert_eq!(st.lost, 1);
    }

    #[test]
    fn burst_protection() {
        let src = packets(400);
        // 4 packets lost in a row in every block (10 x 4 data + 4 FEC): all restored with depth 4
        let (got, st) = run("pw", "pw", 10, 4, &|i, fec| !fec && i % 44 >= 20 && i % 44 < 24, 400);
        assert_eq!(got, src);
        assert!(st.restored >= 36 && st.lost == 0, "{st:?}");
        // the same bursts without interleaving: most are lost
        let (got, st) = run("pw", "pw", 10, 1, &|i, fec| !fec && i % 44 >= 20 && i % 44 < 24, 400);
        assert!(got.len() < 390 && st.lost > 0);
    }

    #[test]
    fn wrong_or_missing_password() {
        let (got, st) = run("secret", "Secret", 0, 1, &|_, _| false, 50);
        assert!(got.is_empty());
        assert_eq!(st.rejected, 50);
        let (got, st) = run("secret", "", 10, 1, &|_, _| false, 50);
        assert!(got.is_empty() && st.need_password);
        let (got, st) = run("", "secret", 10, 1, &|_, _| false, 50);
        assert!(got.is_empty() && st.plain_rejected > 0);
        // a changed packet is rejected
        let mut p = Protector::new("k", 0, 1);
        let mut u = Unprotector::new("k");
        let mut wire = Vec::new();
        p.udp(b"hello world", &mut wire);
        wire[0][20] ^= 0x40;
        let mut got = Vec::new();
        u.udp(&wire[0], &mut got);
        assert!(got.is_empty() && u.stats.rejected == 1);
    }

    #[test]
    fn tcp_encrypted() {
        let mut p = Protector::new("pw", 0, 1);
        let mut bytes = p.tcp_start();
        for c in [&b"RMF1"[..], b"abc", &[7u8; 3000]] {
            bytes.extend(p.tcp(c));
        }
        let key = stream_key("pw");
        let mut g = TcpGate::new();
        let mut out = Vec::new();
        for ch in bytes.chunks(7) {
            assert!(matches!(g.feed(Some(&key), ch, &mut out), Gate::Data));
        }
        assert_eq!(out, [&b"RMF1abc"[..], &[7u8; 3000]].concat());
        let mut g = TcpGate::new();
        assert!(matches!(g.feed(Some(&stream_key("other")), &bytes, &mut Vec::new()), Gate::Rejected));
        let mut g = TcpGate::new();
        assert!(matches!(g.feed(None, &bytes, &mut Vec::new()), Gate::NeedPassword));
        let mut g = TcpGate::new();
        assert!(matches!(g.feed(Some(&key), b"RMF1 plain", &mut Vec::new()), Gate::PlainRejected));
        let mut g = TcpGate::new();
        let mut out = Vec::new();
        assert!(matches!(g.feed(None, b"RMF1 plain", &mut out), Gate::Data));
        assert_eq!(out, b"RMF1 plain");
    }

    #[test]
    fn allow_list() {
        let a = AllowList::parse("192.168.1.10, 10.0.0.0/8; 2001:db8::/32");
        assert!(a.allows("192.168.1.10".parse().unwrap()));
        assert!(!a.allows("192.168.1.11".parse().unwrap()));
        assert!(a.allows("10.200.3.4".parse().unwrap()));
        assert!(a.allows("::ffff:10.1.1.1".parse().unwrap()));
        assert!(a.allows("2001:db8:1::5".parse().unwrap()));
        assert!(!a.allows("2001:db9::5".parse().unwrap()));
        assert!(AllowList::parse("").allows("8.8.8.8".parse().unwrap()));
        assert_eq!(AllowList::invalid("1.2.3.4, x.y, 10.0.0.0/8, 10.0.0.0/z"), vec!["x.y".to_string(), "10.0.0.0/z".to_string()]);
    }
}
