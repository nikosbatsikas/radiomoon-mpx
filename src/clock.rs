//! Reference clock for SFN (single frequency network): UTC in nanoseconds from one of
//!   - the system clock (best when the computer is synchronised by PTP, e.g. ptp4l + phc2sys / Windows PTP, or by
//!     GPS with PPS, e.g. gpsd + chrony),
//!   - a built-in IEEE 1588v2 (PTP) client: default profile, multicast, end-to-end delay, UDP ports 319 / 320,
//!   - a GPS receiver on a serial port: NMEA time ($xxRMC / $xxZDA), optionally PPS on the DCD line.
//! The clock is the system clock plus an offset that the source keeps up to date. After the first lock the offset
//! changes slowly (slewed), so the time never jumps during playout.
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Settings of the reference clock (saved with the program's settings).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClockCfg {
    /// "system" | "ptp" | "gps"
    pub source: String,
    pub ptp_domain: u8,
    /// Address of the network interface for PTP (empty = the default interface).
    pub ptp_iface: String,
    pub gps_port: String,
    pub gps_baud: u32,
    /// The GPS receiver's PPS output is wired to the DCD line of the serial port.
    pub gps_pps: bool,
    /// Without PPS: how long after the start of the second the receiver sends its NMEA sentences (ms).
    pub gps_delay_ms: f32,
}

impl Default for ClockCfg {
    fn default() -> Self {
        ClockCfg { source: "system".into(), ptp_domain: 0, ptp_iface: String::new(), gps_port: String::new(), gps_baud: 9600, gps_pps: false, gps_delay_ms: 0.0 }
    }
}

/// What the window shows about the clock.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClockState {
    pub synced: bool,
    /// Reference minus system clock, microseconds.
    pub offset_us: f64,
    /// Spread of the recent measurements, microseconds (an estimate of the accuracy).
    pub jitter_us: f64,
    /// Source and details, e.g. "PTP master 001b21fffe123456, path delay 85 µs".
    pub text: String,
}

struct Shared {
    state: Mutex<ClockState>,
    /// Offset in ns used by `now_ns` (slewed towards `target`).
    offset: Mutex<(f64, f64, Instant)>, // (current, target, last update)
    locked_once: AtomicBool,
}

pub struct RefClock {
    sh: Arc<Shared>,
    stop: Arc<AtomicBool>,
    pub system: bool,
}

pub fn system_ns() -> i128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as i128).unwrap_or(0)
}

/// Largest change of the offset per second after the first lock (slewing), ns.
const SLEW_NS_PER_S: f64 = 200_000.0;

impl RefClock {
    pub fn start(cfg: &ClockCfg) -> Arc<RefClock> {
        let sh = Arc::new(Shared { state: Mutex::new(ClockState::default()), offset: Mutex::new((0.0, 0.0, Instant::now())), locked_once: AtomicBool::new(false) });
        let stop = Arc::new(AtomicBool::new(false));
        let system = !matches!(cfg.source.as_str(), "ptp" | "gps");
        if system {
            sh.locked_once.store(true, Ordering::Relaxed);
            if let Ok(mut s) = sh.state.lock() {
                *s = ClockState { synced: true, offset_us: 0.0, jitter_us: 0.0, text: "System clock (keep it synchronised with PTP, GPS or NTP)".into() };
            }
        } else {
            let (c, s2, st2) = (cfg.clone(), sh.clone(), stop.clone());
            if cfg.source == "ptp" {
                std::thread::spawn(move || ptp_thread(c, s2, st2));
            } else {
                std::thread::spawn(move || gps_thread(c, s2, st2));
            }
        }
        Arc::new(RefClock { sh, stop, system })
    }

    /// UTC now in nanoseconds, or None while the source has not locked yet.
    pub fn now_ns(&self) -> Option<i128> {
        if !self.sh.locked_once.load(Ordering::Relaxed) {
            return None;
        }
        let off = {
            let mut g = self.sh.offset.lock().ok()?;
            let dt = g.2.elapsed().as_secs_f64();
            g.2 = Instant::now();
            let step = SLEW_NS_PER_S * dt;
            g.0 += (g.1 - g.0).clamp(-step, step);
            g.0
        };
        Some(system_ns() + off as i128)
    }

    pub fn state(&self) -> ClockState {
        self.sh.state.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for RefClock {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A new offset measurement (reference - system, ns): the first one is taken at once, later ones are slewed to.
fn set_offset(sh: &Shared, target_ns: f64) {
    if let Ok(mut g) = sh.offset.lock() {
        if !sh.locked_once.load(Ordering::Relaxed) {
            g.0 = target_ns;
        }
        g.1 = target_ns;
    }
    sh.locked_once.store(true, Ordering::Relaxed);
}

fn set_state(sh: &Shared, f: impl FnOnce(&mut ClockState)) {
    if let Ok(mut g) = sh.state.lock() {
        f(&mut g);
    }
}

/// Median filter over recent measurements: robust against single late packets / sentences.
struct Filter {
    v: Vec<f64>,
    n: usize,
}

impl Filter {
    fn new(n: usize) -> Self {
        Filter { v: Vec::new(), n }
    }
    fn push(&mut self, x: f64) -> (f64, f64) {
        self.v.push(x);
        if self.v.len() > self.n {
            self.v.remove(0);
        }
        let mut s = self.v.clone();
        s.sort_by(|a, b| a.total_cmp(b));
        let med = s[s.len() / 2];
        // spread: half the distance between the 25 % and 75 % values
        let q = |p: f64| s[((s.len() - 1) as f64 * p).round() as usize];
        (med, (q(0.75) - q(0.25)) / 2.0)
    }
}

// ================================================================ IEEE 1588v2 (PTP)

const PTP_MCAST: Ipv4Addr = Ipv4Addr::new(224, 0, 1, 129);

/// PTP timestamp (48-bit seconds, 32-bit nanoseconds) in ns.
fn ptp_ts(b: &[u8]) -> i128 {
    let s = ((b[0] as u64) << 40) | ((b[1] as u64) << 32) | ((b[2] as u64) << 24) | ((b[3] as u64) << 16) | ((b[4] as u64) << 8) | b[5] as u64;
    let ns = u32::from_be_bytes([b[6], b[7], b[8], b[9]]);
    s as i128 * 1_000_000_000 + ns as i128
}

/// correctionField (ns * 2^16) in ns.
fn ptp_corr(h: &[u8]) -> i128 {
    (i64::from_be_bytes([h[8], h[9], h[10], h[11], h[12], h[13], h[14], h[15]]) >> 16) as i128
}

/// A PTP message: type, domain, sequence, two-step flag, correction, source port identity, body.
pub struct PtpMsg<'a> {
    pub kind: u8,
    pub domain: u8,
    pub seq: u16,
    pub two_step: bool,
    pub utc_valid: bool,
    pub corr: i128,
    pub source: [u8; 10],
    pub body: &'a [u8],
}

pub fn ptp_parse(b: &[u8]) -> Option<PtpMsg<'_>> {
    if b.len() < 44 || b[1] & 0x0F != 2 {
        return None;
    }
    let mut source = [0u8; 10];
    source.copy_from_slice(&b[20..30]);
    Some(PtpMsg {
        kind: b[0] & 0x0F,
        domain: b[4],
        seq: u16::from_be_bytes([b[30], b[31]]),
        two_step: b[6] & 0x02 != 0,
        utc_valid: b[7] & 0x04 != 0,
        corr: ptp_corr(b),
        source,
        body: &b[34..],
    })
}

fn delay_req(domain: u8, me: &[u8; 10], seq: u16) -> Vec<u8> {
    let mut m = vec![0u8; 44];
    m[0] = 0x01; // Delay_Req
    m[1] = 0x02; // PTP version 2
    m[2..4].copy_from_slice(&44u16.to_be_bytes());
    m[4] = domain;
    m[20..30].copy_from_slice(me);
    m[30..32].copy_from_slice(&seq.to_be_bytes());
    m[32] = 0x01; // controlField: Delay_Req
    m[33] = 0x7F;
    m
}

fn mcast_socket(port: u16, iface: Ipv4Addr) -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    s.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port).into())?;
    let u: UdpSocket = s.into();
    u.join_multicast_v4(&PTP_MCAST, &iface)?;
    u.set_read_timeout(Some(Duration::from_millis(50)))?;
    Ok(u)
}

/// Offset from one Sync / Follow_Up and one Delay_Req / Delay_Resp exchange (master - local), and the path delay.
pub fn ptp_offset(t1: i128, t2: i128, t3: i128, t4: i128) -> (i128, i128) {
    let offset = ((t1 - t2) + (t4 - t3)) / 2;
    let delay = ((t2 - t1) + (t4 - t3)) / 2;
    (offset, delay)
}

fn ptp_thread(cfg: ClockCfg, sh: Arc<Shared>, stop: Arc<AtomicBool>) {
    let iface: Ipv4Addr = cfg.ptp_iface.trim().parse().unwrap_or(Ipv4Addr::UNSPECIFIED);
    let (ev, gen) = match (mcast_socket(319, iface), mcast_socket(320, iface)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => {
            set_state(&sh, |s| {
                s.synced = false;
                s.text = format!("PTP: cannot open UDP ports 319 / 320 ({e}). On Linux run as root or give the program the CAP_NET_BIND_SERVICE right; if ptp4l already runs here, use \"System clock\".");
            });
            return;
        }
    };
    let _ = gen.set_read_timeout(Some(Duration::from_millis(5)));
    let _ = ev.set_read_timeout(Some(Duration::from_millis(5)));
    // our port identity: random clock identity, port 1
    let mut me = [0u8; 10];
    let r = (system_ns() as u64) ^ ((std::process::id() as u64) << 32);
    me[..8].copy_from_slice(&r.to_be_bytes());
    me[9] = 1;
    let domain = cfg.ptp_domain;
    let mut utc_offset: i128 = 37_000_000_000; // TAI - UTC, updated from Announce
    let mut master: Option<[u8; 10]> = None;
    // last Sync: (seq, t2 local receive time, t1 if one-step, correction)
    let mut sync: Option<(u16, i128, Option<i128>, i128)> = None;
    let mut t12: Option<(i128, i128)> = None; // (t1, t2) of the last complete Sync
    let mut req: Option<(u16, i128)> = None; // our last Delay_Req: (seq, t3)
    let mut seq: u16 = 0;
    let mut last_req = Instant::now() - Duration::from_secs(10);
    let mut filt = Filter::new(15);
    let mut buf = [0u8; 512];
    let mut last_seen = Instant::now();
    set_state(&sh, |s| s.text = format!("PTP: waiting for a master on domain {domain} ..."));
    while !stop.load(Ordering::Relaxed) {
        for sock in [&ev, &gen] {
            let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
            let t_rx = system_ns();
            let Some(m) = ptp_parse(&buf[..n]) else { continue };
            if m.domain != domain {
                continue;
            }
            if m.source[..8] == me[..8] {
                continue; // our own Delay_Req looping back
            }
            match m.kind {
                0x0B if m.body.len() >= 30 => {
                    // Announce: the first master heard is used; currentUtcOffset for TAI -> UTC
                    if master.is_none() {
                        master = Some(m.source);
                    }
                    if master == Some(m.source) && m.utc_valid {
                        utc_offset = i16::from_be_bytes([m.body[10], m.body[11]]) as i128 * 1_000_000_000;
                    }
                }
                0x00 if m.body.len() >= 10 => {
                    // Sync
                    if master.is_none() {
                        master = Some(m.source);
                    }
                    if master != Some(m.source) {
                        continue;
                    }
                    last_seen = Instant::now();
                    let t1 = (!m.two_step).then(|| ptp_ts(m.body));
                    sync = Some((m.seq, t_rx, t1, m.corr));
                    if let Some(t1) = t1 {
                        t12 = Some((t1 + m.corr, t_rx));
                    }
                }
                0x08 if m.body.len() >= 10 => {
                    // Follow_Up: the precise send time of the Sync
                    if let Some((s, t2, None, c)) = sync {
                        if s == m.seq && master == Some(m.source) {
                            t12 = Some((ptp_ts(m.body) + c + m.corr, t2));
                        }
                    }
                }
                0x09 if m.body.len() >= 20 => {
                    // Delay_Resp for our request
                    if m.body[10..20] != me[..] {
                        continue;
                    }
                    if let (Some((rs, t3)), Some((t1, t2))) = (req, t12) {
                        if rs == m.seq {
                            let t4 = ptp_ts(m.body) - m.corr;
                            let (off, delay) = ptp_offset(t1, t2, t3, t4);
                            // reference = UTC: PTP time is TAI
                            let off_utc = off - utc_offset;
                            let (med, spread) = filt.push(off_utc as f64);
                            set_offset(&sh, med);
                            let mid = master.map(|m| m[..8].iter().map(|b| format!("{b:02x}")).collect::<String>()).unwrap_or_default();
                            set_state(&sh, |s| {
                                s.synced = true;
                                s.offset_us = med / 1000.0;
                                s.jitter_us = spread / 1000.0;
                                s.text = format!("PTP master {mid}, domain {domain}, path delay {:.0} µs", delay as f64 / 1000.0);
                            });
                            req = None;
                        }
                    }
                }
                _ => {}
            }
        }
        // one Delay_Req per second while a master is heard
        if t12.is_some() && last_req.elapsed() >= Duration::from_secs(1) {
            last_req = Instant::now();
            seq = seq.wrapping_add(1);
            let msg = delay_req(domain, &me, seq);
            let t3 = system_ns();
            if ev.send_to(&msg, SocketAddrV4::new(PTP_MCAST, 319)).is_ok() {
                req = Some((seq, t3));
            }
        }
        if last_seen.elapsed() > Duration::from_secs(5) && master.is_some() {
            set_state(&sh, |s| {
                s.synced = false;
                s.text = "PTP: the master is not heard any more (holding the last offset)".into();
            });
            master = None;
            t12 = None;
        }
    }
}

// ================================================================ GPS (NMEA on a serial port)

/// Seconds since 1970 of an NMEA time + date (hhmmss.ss, ddmmyy or day / month / year).
fn utc_secs(hms: &str, day: u32, month: u32, year: i32) -> Option<f64> {
    if hms.len() < 6 || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let h: f64 = hms.get(0..2)?.parse().ok()?;
    let m: f64 = hms.get(2..4)?.parse().ok()?;
    let s: f64 = hms.get(4..)?.parse().ok()?;
    // days since 1970-01-01 (civil calendar)
    let (y, mo) = if month <= 2 { (year - 1, month + 9) } else { (year, month - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * mo as i32 + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era as i64 * 146_097 + doe as i64 - 719_468;
    Some(days as f64 * 86_400.0 + h * 3600.0 + m * 60.0 + s)
}

/// UTC (seconds since 1970) of an NMEA RMC or ZDA sentence with a valid checksum and (RMC) status A.
pub fn nmea_time(line: &str) -> Option<f64> {
    let line = line.trim();
    let body = line.strip_prefix('$')?;
    let (data, sum) = body.split_once('*')?;
    let want = u8::from_str_radix(sum.get(0..2)?, 16).ok()?;
    if data.bytes().fold(0u8, |a, b| a ^ b) != want {
        return None;
    }
    let f: Vec<&str> = data.split(',').collect();
    let kind = f.first()?.get(2..)?;
    match kind {
        "RMC" if f.len() > 9 && f[2] == "A" => {
            let d = f[9];
            let (day, month, yy): (u32, u32, i32) = (d.get(0..2)?.parse().ok()?, d.get(2..4)?.parse().ok()?, d.get(4..6)?.parse().ok()?);
            utc_secs(f[1], day, month, 2000 + yy)
        }
        "ZDA" if f.len() > 4 && !f[2].is_empty() => utc_secs(f[1], f[2].parse().ok()?, f[3].parse().ok()?, f[4].parse().ok()?),
        _ => None,
    }
}

/// Number of satellites in use, from a GGA sentence.
fn nmea_sats(line: &str) -> Option<u32> {
    let f: Vec<&str> = line.trim().trim_start_matches('$').split(',').collect();
    (f.first()?.get(2..)? == "GGA").then(|| f.get(7)?.parse().ok())?
}

/// The serial ports of this computer (for the settings).
pub fn serial_ports() -> Vec<String> {
    serialport::available_ports().map(|v| v.into_iter().map(|p| p.port_name).collect()).unwrap_or_default()
}

fn gps_thread(cfg: ClockCfg, sh: Arc<Shared>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Relaxed) {
        let name = cfg.gps_port.trim().to_string();
        if name.is_empty() {
            set_state(&sh, |s| s.text = "GPS: choose the serial port of the GPS receiver".into());
            return;
        }
        let port = serialport::new(&name, cfg.gps_baud.max(1200)).timeout(Duration::from_millis(1)).open();
        let mut port = match port {
            Ok(p) => p,
            Err(e) => {
                set_state(&sh, |s| {
                    s.synced = false;
                    s.text = format!("GPS: cannot open {name}: {e}");
                });
                for _ in 0..30 {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                continue;
            }
        };
        set_state(&sh, |s| s.text = format!("GPS: reading {name} ..."));
        let mut line = String::new();
        let mut filt = Filter::new(9);
        let (mut burst_start, mut last_byte): (Option<i128>, Instant) = (None, Instant::now());
        let mut pps_edge: Option<i128> = None;
        let mut last_dcd = false;
        let mut sats = 0u32;
        let mut buf = [0u8; 256];
        let mut last_ok = Instant::now();
        loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            if cfg.gps_pps {
                if let Ok(d) = port.read_carrier_detect() {
                    if d && !last_dcd {
                        pps_edge = Some(system_ns()); // rising edge = start of a UTC second
                    }
                    last_dcd = d;
                }
            }
            match port.read(&mut buf) {
                Ok(n) if n > 0 => {
                    let now = system_ns();
                    // the first byte after a pause starts the burst of sentences of this second
                    if last_byte.elapsed() > Duration::from_millis(200) {
                        burst_start = Some(now);
                    }
                    last_byte = Instant::now();
                    for &c in &buf[..n] {
                        if c == b'\n' {
                            if let Some(n) = nmea_sats(&line) {
                                sats = n;
                            }
                            if let Some(utc) = nmea_time(&line) {
                                // the second the sentence names began at the PPS edge, or at the burst start minus the
                                // receiver's output delay
                                let sec = utc.floor();
                                let local = match (cfg.gps_pps, pps_edge) {
                                    (true, Some(e)) if now - e < 1_000_000_000 => Some(e),
                                    (true, _) => None,
                                    (false, _) => burst_start.map(|b| b - (cfg.gps_delay_ms as f64 * 1e6) as i128 - ((utc - sec) * 1e9) as i128),
                                };
                                if let Some(local) = local {
                                    let off = sec as i128 * 1_000_000_000 - local;
                                    let (med, spread) = filt.push(off as f64);
                                    set_offset(&sh, med);
                                    last_ok = Instant::now();
                                    set_state(&sh, |s| {
                                        s.synced = true;
                                        s.offset_us = med / 1000.0;
                                        s.jitter_us = spread / 1000.0;
                                        s.text = format!("GPS {name}: {} · {sats} satellites", if cfg.gps_pps { "NMEA + PPS (DCD)" } else { "NMEA only (about ±10 ms)" });
                                    });
                                }
                                burst_start = None; // one measurement per second
                            }
                            line.clear();
                        } else if c != b'\r' && line.len() < 120 {
                            line.push(c as char);
                        }
                    }
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => {
                    set_state(&sh, |s| {
                        s.synced = false;
                        s.text = format!("GPS: {e}");
                    });
                    break; // reopen
                }
            }
            if last_ok.elapsed() > Duration::from_secs(5) {
                set_state(&sh, |s| {
                    s.synced = false;
                    s.text = format!("GPS {name}: no valid time (no fix yet, wrong baud rate{}?)", if cfg.gps_pps { " or no PPS on DCD" } else { "" });
                });
                last_ok = Instant::now();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nmea_rmc_and_zda() {
        // 2026-10-25 01:50:12 UTC
        let rmc = "$GPRMC,015012.00,A,3758.0000,N,02343.0000,E,0.0,0.0,251026,,,A*";
        let sum = rmc[1..rmc.len() - 1].bytes().fold(0u8, |a, b| a ^ b);
        let t = nmea_time(&format!("{rmc}{sum:02X}")).unwrap();
        assert_eq!(t, 1_792_893_012.0);
        let zda = "$GNZDA,015012.50,25,10,2026,00,00*";
        let sum = zda[1..zda.len() - 1].bytes().fold(0u8, |a, b| a ^ b);
        assert_eq!(nmea_time(&format!("{zda}{sum:02X}")).unwrap(), 1_792_893_012.5);
        assert!(nmea_time("$GPRMC,015012.00,V,,,,,,,251026,,,N*00").is_none());
    }

    #[test]
    fn ptp_offset_math() {
        // master is 1 ms ahead of us, path delay 100 µs
        let (t1, t3) = (10_000_000_000i128, 10_500_000_000i128);
        let t2 = t1 + 100_000 - 1_000_000; // our clock reads 1 ms less
        let t4 = t3 + 1_000_000 + 100_000;
        let (off, delay) = ptp_offset(t1, t2, t3, t4);
        assert_eq!((off, delay), (1_000_000, 100_000));
    }

    #[test]
    fn ptp_message_parse() {
        let m = delay_req(3, &[1, 2, 3, 4, 5, 6, 7, 8, 0, 1], 77);
        let p = ptp_parse(&m).unwrap();
        assert_eq!((p.kind, p.domain, p.seq, p.source[0]), (1, 3, 77, 1));
    }
}
