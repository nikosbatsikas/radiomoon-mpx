//! Time over the stream link (SFN): every Receiver can take the Transmitter's clock as its reference, without NTP,
//! PTP or GPS. The Transmitter sends a small time packet ("RMT1") to each UDP receiver once a second, on the same
//! socket and path as the stream; the Receiver answers at once. From the four time stamps (sent / received / answered
//! / answer received) the Receiver gets its offset to the Transmitter's clock and the round trip, as NTP does, but
//! straight to the Transmitter and over the very link of the stream. Only the measurements with the shortest round
//! trip are used (they were not held up in a queue), and a straight line through them follows the drift of the two
//! clocks.
//!
//! The Transmitter starts every exchange, so it works through NAT and needs no extra port: the Receiver answers to
//! the address the probe came from. With a stream password both directions carry an HMAC of the packet, so nobody
//! can shift a site's time.
//!
//! Accuracy: the path is assumed to take as long one way as the other. On a LAN that gives some tens of µs; over
//! WiFi or the internet the difference between the two directions (often 0.1 - 2 ms) is the error. All sites then
//! follow the same clock — the Transmitter's — so their difference to each other is what counts, not the true UTC.
//! For SFN with an overlap of a few µs use GPS (PPS) at every site.
use crate::clock::{system_ns, RefClock};
use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const MAGIC: &[u8; 4] = b"RMT1";
const PROBE: u8 = 1;
const REPLY: u8 = 2;
const PROBE_LEN: usize = 30;
const REPLY_LEN: usize = 34;
const MAC_LEN: usize = 16;
/// Reply flag: the Receiver plays by this Transmitter's clock (and has locked to it).
pub const FLAG_FOLLOWS: u8 = 1;
/// Probe flag: the previous exchange's arrival time is included.
const FLAG_PREV: u8 = 1;

pub fn is_time_packet(d: &[u8]) -> bool {
    d.len() >= PROBE_LEN && &d[..4] == MAGIC
}

fn mac(key: &[u8; 32], data: &[u8]) -> [u8; MAC_LEN] {
    let h = crate::crypt::hmac_sha256(key, &[b"RadioMoon time".as_slice(), data].concat());
    let mut m = [0u8; MAC_LEN];
    m.copy_from_slice(&h[..MAC_LEN]);
    m
}

fn seal(mut v: Vec<u8>, key: Option<&[u8; 32]>) -> Vec<u8> {
    if let Some(k) = key {
        let m = mac(k, &v);
        v.extend_from_slice(&m);
    }
    v
}

/// The packet without its HMAC, if it is authentic for `key` (no key: only packets without HMAC).
fn open<'a>(d: &'a [u8], base_len: usize, key: Option<&[u8; 32]>) -> Option<&'a [u8]> {
    match key {
        None => (d.len() == base_len).then_some(d),
        Some(k) => {
            if d.len() != base_len + MAC_LEN {
                return None;
            }
            let (body, m) = d.split_at(base_len);
            // constant-time compare
            let want = mac(k, body);
            let diff = want.iter().zip(m).fold(0u8, |a, (x, y)| a | (x ^ y));
            (diff == 0).then_some(body)
        }
    }
}

fn be64(b: &[u8]) -> i64 {
    i64::from_be_bytes(b[..8].try_into().unwrap_or([0; 8]))
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes(b[..4].try_into().unwrap_or([0; 4]))
}

pub fn probe(seq: u32, t1: i64, prev: Option<(u32, i64)>, key: Option<&[u8; 32]>) -> Vec<u8> {
    let mut v = Vec::with_capacity(PROBE_LEN + MAC_LEN);
    v.extend_from_slice(MAGIC);
    v.push(PROBE);
    v.push(if prev.is_some() { FLAG_PREV } else { 0 });
    v.extend_from_slice(&seq.to_be_bytes());
    v.extend_from_slice(&t1.to_be_bytes());
    let (ps, pt4) = prev.unwrap_or((0, 0));
    v.extend_from_slice(&ps.to_be_bytes());
    v.extend_from_slice(&pt4.to_be_bytes());
    seal(v, key)
}

pub fn reply(seq: u32, t1: i64, t2: i64, t3: i64, flags: u8, key: Option<&[u8; 32]>) -> Vec<u8> {
    let mut v = Vec::with_capacity(REPLY_LEN + MAC_LEN);
    v.extend_from_slice(MAGIC);
    v.push(REPLY);
    v.push(flags);
    v.extend_from_slice(&seq.to_be_bytes());
    v.extend_from_slice(&t1.to_be_bytes());
    v.extend_from_slice(&t2.to_be_bytes());
    v.extend_from_slice(&t3.to_be_bytes());
    seal(v, key)
}

/// A probe: (seq, t1, previous (seq, t4)).
pub fn parse_probe(d: &[u8], key: Option<&[u8; 32]>) -> Option<(u32, i64, Option<(u32, i64)>)> {
    if d.len() < PROBE_LEN || &d[..4] != MAGIC || d[4] != PROBE {
        return None;
    }
    let b = open(d, PROBE_LEN, key)?;
    let prev = (b[5] & FLAG_PREV != 0).then(|| (be32(&b[18..]), be64(&b[22..])));
    Some((be32(&b[6..]), be64(&b[10..]), prev))
}

/// A reply: (seq, t1, t2, t3, flags).
pub fn parse_reply(d: &[u8], key: Option<&[u8; 32]>) -> Option<(u32, i64, i64, i64, u8)> {
    if d.len() < REPLY_LEN || &d[..4] != MAGIC || d[4] != REPLY {
        return None;
    }
    let b = open(d, REPLY_LEN, key)?;
    Some((be32(&b[6..]), be64(&b[10..]), be64(&b[18..]), be64(&b[26..]), b[5]))
}

/// Offset (clock of the answering side minus the asking side) and round trip, ns, from the four time stamps:
/// t1 sent and t4 answer received by the asking side, t2 received and t3 answered by the other side.
pub fn offset_delay(t1: i64, t2: i64, t3: i64, t4: i64) -> (i64, i64) {
    let off = ((t2 as i128 - t1 as i128) + (t3 as i128 - t4 as i128)) / 2;
    let delay = (t4 as i128 - t1 as i128) - (t3 as i128 - t2 as i128);
    (off as i64, delay.max(0) as i64)
}

// ================================================================ Transmitter side

/// What the Transmitter knows about one Receiver's clock (shown with its IP output).
#[derive(Clone, Debug, Default)]
pub struct PeerTime {
    pub answered: bool,
    /// The Receiver's clock minus the Transmitter's, ms (its own clock; 0 when it follows this one).
    pub offset_ms: f64,
    pub rtt_ms: f64,
    pub follows: bool,
}

impl PeerTime {
    pub fn text(&self) -> String {
        if !self.answered {
            return String::new();
        }
        if self.follows {
            format!("Time: the Receiver follows this clock (round trip {:.1} ms)", self.rtt_ms)
        } else {
            format!("Time: the Receiver's clock {:+.1} ms (round trip {:.1} ms)", self.offset_ms, self.rtt_ms)
        }
    }
}

/// Probes one Receiver once a second (4 times a second at the start) on a copy of the stream's socket and reads
/// the answers. Stops with `stop`. Receivers older than 1.13 do not answer: after 10 unanswered probes it asks only
/// every 30 s (they count the probe as a bad packet).
pub fn prober(sock: &UdpSocket, clock: Arc<RefClock>, password: &str, stop: Arc<AtomicBool>, out: Arc<Mutex<PeerTime>>) {
    let Ok(rd) = sock.try_clone() else { return };
    let key = (!password.is_empty()).then(|| crate::crypt::stream_key(password));
    let _ = rd.set_read_timeout(Some(Duration::from_millis(100)));
    std::thread::spawn(move || {
        let mut seq: u32 = 1;
        let mut sent: VecDeque<(u32, i64)> = VecDeque::new();
        let mut last: Option<(u32, i64)> = None; // the last answer: (seq, t4)
        let mut unanswered = 0u32;
        let mut next = Instant::now();
        let mut buf = [0u8; 128];
        let mut filter = Best::new();
        while !stop.load(Ordering::Relaxed) {
            if Instant::now() >= next {
                if let Some(t1) = clock.now_ns() {
                    let t1 = t1 as i64;
                    let _ = rd.send(&probe(seq, t1, last, key.as_ref()));
                    sent.push_back((seq, t1));
                    while sent.len() > 64 {
                        sent.pop_front();
                    }
                    seq = seq.wrapping_add(1);
                    unanswered += 1;
                }
                let gap = if unanswered > 10 {
                    30_000
                } else if seq < 16 {
                    250
                } else {
                    1000
                };
                next = Instant::now() + Duration::from_millis(gap);
            }
            if let Ok(n) = rd.recv(&mut buf) {
                let t4 = clock.now_ns().unwrap_or(0) as i64;
                if let Some((s, t1, t2, t3, flags)) = parse_reply(&buf[..n], key.as_ref()) {
                    if sent.iter().any(|&(q, t)| q == s && t == t1) {
                        unanswered = 0;
                        last = Some((s, t4));
                        let (off, d) = offset_delay(t1, t2, t3, t4);
                        let off = filter.push(system_ns(), off as f64, d as f64).map(|(o, _)| o).unwrap_or(off as f64);
                        if let Ok(mut g) = out.lock() {
                            *g = PeerTime { answered: true, offset_ms: off / 1e6, rtt_ms: d as f64 / 1e6, follows: flags & FLAG_FOLLOWS != 0 };
                        }
                    }
                }
            }
        }
    });
}

// ================================================================ Receiver side

/// Picks the trustworthy measurements: of the last 64, those with the shortest round trip (at most a quarter, at least 4), and fits a straight
/// line through their offsets (the two clocks drift by some ppm). Gives the offset now and the spread.
pub struct Best {
    v: VecDeque<(i128, f64, f64)>, // (local system time ns, offset ns, round trip ns)
}

impl Best {
    pub fn new() -> Best {
        Best { v: VecDeque::new() }
    }

    /// A new measurement; returns (offset now, spread) once there are enough.
    pub fn push(&mut self, now: i128, off: f64, delay: f64) -> Option<(f64, f64)> {
        self.v.push_back((now, off, delay));
        while self.v.len() > 64 {
            self.v.pop_front();
        }
        if self.v.len() < 4 {
            return None;
        }
        let mut s: Vec<(i128, f64, f64)> = self.v.iter().copied().collect();
        s.sort_by(|a, b| a.2.total_cmp(&b.2));
        // at most a quarter, and only those close to the shortest round trip (the others waited in a queue)
        let limit = s[0].2 + (s[0].2 * 0.02).max(50_000.0);
        let close = s.iter().take_while(|p| p.2 <= limit).count();
        let keep = close.min(s.len() / 4).max(4).min(s.len());
        let best = &s[..keep];
        // least squares offset = a + b * (t - now), t in seconds
        let n = best.len() as f64;
        let xs: Vec<f64> = best.iter().map(|p| (p.0 - now) as f64 / 1e9).collect();
        let ys: Vec<f64> = best.iter().map(|p| p.1).collect();
        let mx = xs.iter().sum::<f64>() / n;
        let my = ys.iter().sum::<f64>() / n;
        let sxx: f64 = xs.iter().map(|x| (x - mx) * (x - mx)).sum();
        let sxy: f64 = xs.iter().zip(&ys).map(|(x, y)| (x - mx) * (y - my)).sum();
        // fit the drift only over enough time; it cannot be more than 500 ppm
        let slope = if sxx > 25.0 { (sxy / sxx).clamp(-500_000.0, 500_000.0) } else { 0.0 };
        let a = my - slope * mx;
        let spread = (ys.iter().zip(&xs).map(|(y, x)| (y - (a + slope * x)).powi(2)).sum::<f64>() / n).sqrt();
        Some((a, spread))
    }

    pub fn min_delay(&self) -> f64 {
        self.v.iter().map(|p| p.2).fold(f64::INFINITY, f64::min)
    }
}

impl Default for Best {
    fn default() -> Self {
        Best::new()
    }
}

/// The Receiver's part: answers the probes on the stream socket and, when the reference clock is "transmitter",
/// feeds it the Transmitter's time.
pub struct Responder {
    key: Option<[u8; 32]>,
    /// (seq, t1, t2, t3) of the recent probes, waiting for their t4.
    pending: VecDeque<(u32, i64, i64, i64)>,
    clock: Option<Arc<RefClock>>,
    best: Best,
}

impl Responder {
    /// `clock`: the SFN reference clock when it is "transmitter" (it is fed), else None (only answered).
    pub fn new(password: &str, clock: Option<Arc<RefClock>>) -> Responder {
        Responder { key: (!password.is_empty()).then(|| crate::crypt::stream_key(password)), pending: VecDeque::new(), clock, best: Best::new() }
    }

    /// A time packet from `from` (already checked: allowed sender). Answers it on `sock`.
    pub fn packet(&mut self, d: &[u8], from: SocketAddr, sock: &UdpSocket) {
        let t2 = system_ns() as i64;
        let Some((seq, t1, prev)) = parse_probe(d, self.key.as_ref()) else { return };
        // the previous exchange is complete now: t4 came with this probe
        if let Some((ps, t4)) = prev {
            if let Some(&(_, p1, p2, p3)) = self.pending.iter().find(|p| p.0 == ps) {
                // offset of the Transmitter's clock against this computer's system clock
                let (rx_minus_tx, delay) = offset_delay(p1, p2, p3, t4);
                if let Some(c) = &self.clock {
                    if let Some((off, spread)) = self.best.push(system_ns(), -(rx_minus_tx as f64), delay as f64) {
                        c.feed_transmitter(off, spread, self.best.min_delay(), &from.ip().to_string());
                    }
                }
            }
        }
        let follows = self.clock.as_ref().map(|c| c.now_ns().is_some()).unwrap_or(false);
        let t3 = system_ns() as i64;
        let r = reply(seq, t1, t2, t3, if follows { FLAG_FOLLOWS } else { 0 }, self.key.as_ref());
        let _ = sock.send_to(&r, from);
        self.pending.push_back((seq, t1, t2, t3));
        while self.pending.len() > 16 {
            self.pending.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets_and_hmac() {
        let k = crate::crypt::stream_key("secret");
        let p = probe(7, 123, Some((6, 456)), Some(&k));
        assert!(is_time_packet(&p));
        assert_eq!(parse_probe(&p, Some(&k)), Some((7, 123, Some((6, 456)))));
        // a wrong key, no key, or a changed byte: refused
        assert_eq!(parse_probe(&p, Some(&crate::crypt::stream_key("other"))), None);
        assert_eq!(parse_probe(&p, None), None);
        let mut bad = p.clone();
        bad[12] ^= 1;
        assert_eq!(parse_probe(&bad, Some(&k)), None);
        let r = reply(7, 123, 200, 210, FLAG_FOLLOWS, None);
        assert_eq!(parse_reply(&r, None), Some((7, 123, 200, 210, FLAG_FOLLOWS)));
        assert_eq!(parse_probe(&probe(1, 2, None, None), None), Some((1, 2, None)));
    }

    #[test]
    fn receiver_locks_to_the_transmitter_over_udp() {
        use crate::clock::ClockCfg;
        let rx_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let rx_addr = rx_sock.local_addr().unwrap();
        let tx_sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        tx_sock.connect(rx_addr).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let tx_clock = RefClock::start(&ClockCfg::default());
        let peer = Arc::new(Mutex::new(PeerTime::default()));
        prober(&tx_sock, tx_clock, "pw-123", stop.clone(), peer.clone());
        let rx_clock = RefClock::start(&ClockCfg { source: "transmitter".into(), ..Default::default() });
        assert!(rx_clock.now_ns().is_none(), "not locked before the first time packets");
        let mut resp = Responder::new("pw-123", Some(rx_clock.clone()));
        rx_sock.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let mut buf = [0u8; 128];
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(4) && rx_clock.now_ns().is_none() {
            if let Ok((n, a)) = rx_sock.recv_from(&mut buf) {
                assert!(is_time_packet(&buf[..n]));
                resp.packet(&buf[..n], a, &rx_sock);
            }
        }
        // keep answering a little, so the Transmitter sees that it is followed
        let t1 = Instant::now();
        while t1.elapsed() < Duration::from_millis(1500) {
            if let Ok((n, a)) = rx_sock.recv_from(&mut buf) {
                resp.packet(&buf[..n], a, &rx_sock);
            }
        }
        stop.store(true, Ordering::Relaxed);
        let st = rx_clock.state();
        assert!(st.synced, "{st:?}");
        // the same computer: the Transmitter's clock is this clock
        assert!(st.offset_us.abs() < 1000.0, "{st:?}");
        let p = peer.lock().unwrap().clone();
        assert!(p.answered && p.follows, "{p:?}");
        // a Receiver with another password ignores the probes
        let mut wrong = Responder::new("other", None);
        let probe = probe(1, 5, None, Some(&crate::crypt::stream_key("pw-123")));
        let before = wrong.pending.len();
        wrong.packet(&probe, rx_addr, &rx_sock);
        assert_eq!(wrong.pending.len(), before);
    }

    #[test]
    fn offset_from_four_stamps() {
        // the other clock 159 ms ahead, 3 ms each way, 0.1 ms to answer
        let (t1, t2, t3, t4) = (0i64, 159_000_000 + 3_000_000, 159_000_000 + 3_100_000, 6_100_000);
        assert_eq!(offset_delay(t1, t2, t3, t4), (159_000_000, 6_000_000));
    }

    #[test]
    fn best_ignores_queued_packets_and_follows_drift() {
        // true offset 159 ms with a drift of 20 ppm; most answers are held up by 0 - 8 ms on one way
        let mut b = Best::new();
        let mut r = 12345u64;
        let mut last = None;
        for i in 0..64i64 {
            r = r.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let queue = ((r >> 33) % 8_000_000) as f64 * if i % 5 == 0 { 0.0 } else { 1.0 };
            let t = i as i128 * 1_000_000_000;
            let truth = 159e6 + 20.0 * 1e3 * i as f64; // 20 µs per second
            // a delay on one way shifts the offset by half of it
            last = b.push(t, truth + queue / 2.0, 6e6 + queue);
        }
        let (off, _) = last.unwrap();
        let truth_now = 159e6 + 20.0 * 1e3 * 63.0;
        assert!((off - truth_now).abs() < 50_000.0, "off {off} truth {truth_now}");
    }
}
