//! Audio sources for the stand-alone MPX transmitter: a sound card input or an internet radio stream.
//! Everything is delivered as 44.1 kHz stereo blocks at real-time pace.
use rodio::cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rodio::cpal::SampleFormat;
use rodio::{Decoder, Source};
use std::collections::VecDeque;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct SrcCfg {
    /// "input" | "stream"
    pub kind: String,
    pub device: String,
    pub url: String,
    pub gain_db: f32,
}

struct Shared {
    q: Mutex<VecDeque<f32>>,
    cap: usize,
    stop: AtomicBool,
    gain: f32,
    peak: AtomicU32,
    status: Mutex<String>,
    now: Mutex<String>,
}

impl Shared {
    fn set_status(&self, s: &str) {
        if let Ok(mut g) = self.status.lock() {
            *g = s.to_string();
        }
    }
    /// Adds stereo samples. Files and streams wait when the queue is full; a sound card drops the oldest.
    fn push(&self, buf: &mut Vec<f32>, block: bool) {
        let mut pk = 0f32;
        for x in buf.iter_mut() {
            *x *= self.gain;
            pk = pk.max(x.abs());
        }
        let old = f32::from_bits(self.peak.load(Ordering::Relaxed));
        self.peak.store(pk.max(old * 0.9).to_bits(), Ordering::Relaxed);
        if block {
            while !self.stop.load(Ordering::Relaxed) && self.q.lock().map(|q| q.len() > self.cap).unwrap_or(false) {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        if let Ok(mut q) = self.q.lock() {
            q.extend(buf.iter().copied());
            while q.len() > self.cap * 2 {
                q.pop_front();
            }
        }
        buf.clear();
    }
}

pub struct SrcHandle {
    sh: Arc<Shared>,
}

impl SrcHandle {
    pub fn status(&self) -> String {
        self.sh.status.lock().map(|g| g.clone()).unwrap_or_default()
    }
    /// The song title sent by the stream server (Icecast / Shoutcast StreamTitle).
    pub fn now(&self) -> String {
        self.sh.now.lock().map(|g| g.clone()).unwrap_or_default()
    }
    pub fn peak(&self) -> f32 {
        f32::from_bits(self.sh.peak.load(Ordering::Relaxed))
    }
    pub fn stop(&self) {
        self.sh.stop.store(true, Ordering::Relaxed);
    }
}

pub fn list_inputs() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(devs) = rodio::cpal::default_host().input_devices() {
        for d in devs {
            if let Ok(n) = d.name() {
                v.push(n);
            }
        }
    }
    v
}

/// Linear resampler to 44.1 kHz.
struct Rs {
    step: f64,
    pos: f64,
    prev: (f32, f32),
}

impl Rs {
    fn new(rate: u32) -> Self {
        Rs { step: rate.max(1) as f64 / 44100.0, pos: 0.0, prev: (0.0, 0.0) }
    }
    fn frame(&mut self, l: f32, r: f32, out: &mut Vec<f32>) {
        while self.pos < 1.0 {
            let a = self.pos as f32;
            out.push(self.prev.0 + (l - self.prev.0) * a);
            out.push(self.prev.1 + (r - self.prev.1) * a);
            self.pos += self.step;
        }
        self.pos -= 1.0;
        self.prev = (l, r);
    }
}

/// Starts the source. The receiver delivers interleaved stereo 44.1 kHz blocks, one block every 10 ms or so.
pub fn start(cfg: &SrcCfg) -> (SrcHandle, Receiver<Vec<f32>>) {
    let sh = Arc::new(Shared {
        q: Mutex::new(VecDeque::new()),
        cap: if cfg.kind == "input" { 44100 / 2 } else { 44100 * 4 },
        stop: AtomicBool::new(false),
        gain: 10f32.powf(cfg.gain_db / 20.0),
        peak: AtomicU32::new(0),
        status: Mutex::new("Starting ...".into()),
        now: Mutex::new(String::new()),
    });
    let c = cfg.clone();
    let s2 = sh.clone();
    match cfg.kind.as_str() {
        "stream" => {
            std::thread::spawn(move || stream_thread(c.url, s2));
        }
        _ => {
            std::thread::spawn(move || capture_thread(c.device, s2));
        }
    }
    // pacer: hands the samples on at exactly 44.1 kHz, silence when the source has nothing
    let (tx, rx) = sync_channel::<Vec<f32>>(64);
    let s3 = sh.clone();
    std::thread::spawn(move || {
        let t0 = Instant::now();
        let mut sent: u64 = 0;
        while !s3.stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
            let due = (t0.elapsed().as_secs_f64() * 88_200.0) as u64;
            let mut need = (due.saturating_sub(sent) & !1) as usize;
            if need > 88_200 {
                need = 8820; // the program was stopped for a while: do not send a burst
                sent = due - 8820;
            }
            if need == 0 {
                continue;
            }
            let mut blk: Vec<f32> = Vec::with_capacity(need);
            if let Ok(mut q) = s3.q.lock() {
                let n = need.min(q.len());
                blk.extend(q.drain(..n));
            }
            blk.resize(need, 0.0);
            sent += need as u64;
            if tx.send(blk).is_err() {
                break;
            }
        }
    });
    (SrcHandle { sh }, rx)
}

// ---------------------------------------------------------------- sound card input
fn capture_thread(device: String, sh: Arc<Shared>) {
    let host = rodio::cpal::default_host();
    let dev = host.input_devices().ok().and_then(|mut it| it.find(|d| d.name().map(|n| n == device).unwrap_or(false))).or_else(|| host.default_input_device());
    let Some(dev) = dev else {
        sh.set_status("No sound input found");
        return;
    };
    let cfg = match dev.default_input_config() {
        Ok(c) => c,
        Err(e) => {
            sh.set_status(&format!("Sound input: {e}"));
            return;
        }
    };
    let ch = cfg.channels().max(1) as usize;
    let mut rs = Rs::new(cfg.sample_rate().0);
    let sh2 = sh.clone();
    let mut out: Vec<f32> = Vec::new();
    let mut handle = move |d: &[f32]| {
        for f in d.chunks_exact(ch) {
            let (l, r) = if ch == 1 { (f[0], f[0]) } else { (f[0], f[1]) };
            rs.frame(l, r, &mut out);
        }
        sh2.push(&mut out, false);
    };
    let err = |e| eprintln!("sound input: {e}");
    let stream = match cfg.sample_format() {
        SampleFormat::F32 => dev.build_input_stream(&cfg.clone().into(), move |d: &[f32], _| handle(d), err, None),
        SampleFormat::I16 => {
            let mut tmp: Vec<f32> = Vec::new();
            dev.build_input_stream(
                &cfg.clone().into(),
                move |d: &[i16], _| {
                    tmp.clear();
                    tmp.extend(d.iter().map(|x| *x as f32 / 32768.0));
                    handle(&tmp)
                },
                err,
                None,
            )
        }
        SampleFormat::U16 => {
            let mut tmp: Vec<f32> = Vec::new();
            dev.build_input_stream(
                &cfg.clone().into(),
                move |d: &[u16], _| {
                    tmp.clear();
                    tmp.extend(d.iter().map(|x| (*x as f32 - 32768.0) / 32768.0));
                    handle(&tmp)
                },
                err,
                None,
            )
        }
        f => {
            sh.set_status(&format!("Unsupported input format {f:?}"));
            return;
        }
    };
    let stream = match stream {
        Ok(s) => s,
        Err(e) => {
            sh.set_status(&format!("Sound input: {e}"));
            return;
        }
    };
    if let Err(e) = stream.play() {
        sh.set_status(&format!("Sound input: {e}"));
        return;
    }
    sh.set_status("Sound input on air");
    while !sh.stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(stream);
}

// ---------------------------------------------------------------- internet stream
struct NoSeek<R>(R);

impl<R: Read> Read for NoSeek<R> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(b)
    }
}

impl<R> Seek for NoSeek<R> {
    fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
        match p {
            SeekFrom::Current(0) => Ok(0),
            _ => Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "live stream")),
        }
    }
}

/// Removes the Shoutcast/Icecast metadata blocks from the audio data and keeps the song title (StreamTitle)
/// for the RDS RadioText. RadioMoon sends its playing song this way when it broadcasts to the server.
struct Icy<R> {
    inner: R,
    metaint: Option<usize>,
    left: usize,
    sh: Arc<Shared>,
}

impl<R: Read> Read for Icy<R> {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let Some(metaint) = self.metaint else { return self.inner.read(b) };
        if self.left == 0 {
            let mut len = [0u8; 1];
            self.inner.read_exact(&mut len)?;
            let mut meta = vec![0u8; len[0] as usize * 16];
            self.inner.read_exact(&mut meta)?;
            if let Some(t) = stream_title(&meta) {
                if let Ok(mut n) = self.sh.now.lock() {
                    *n = t;
                }
            }
            self.left = metaint;
        }
        let n = b.len().min(self.left);
        let got = self.inner.read(&mut b[..n])?;
        self.left -= got;
        Ok(got)
    }
}

/// `StreamTitle='Artist - Title';StreamUrl='';` -> "Artist - Title"
fn stream_title(meta: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(meta);
    let text = text.trim_end_matches('\0');
    let start = text.find("StreamTitle='")? + "StreamTitle='".len();
    let rest = &text[start..];
    let end = rest.find("';").or_else(|| rest.rfind('\'')).unwrap_or(rest.len());
    let t = rest[..end].trim().to_string();
    (!t.is_empty()).then_some(t)
}

/// Decodes every sample of `dec` into the queue. Returns false when stopped.
fn feed<S: Source<Item = f32>>(dec: S, sh: &Shared) -> bool {
    let ch = dec.channels().max(1) as usize;
    let mut rs = Rs::new(dec.sample_rate());
    let mut it = dec;
    let mut out: Vec<f32> = Vec::with_capacity(8192);
    loop {
        let mut f = [0f32; 8];
        for k in 0..ch {
            match it.next() {
                Some(x) => {
                    if k < 8 {
                        f[k] = x;
                    }
                }
                None => {
                    if !out.is_empty() {
                        sh.push(&mut out, true);
                    }
                    return !sh.stop.load(Ordering::Relaxed);
                }
            }
        }
        let (l, r) = if ch == 1 { (f[0], f[0]) } else { (f[0], f[1]) };
        rs.frame(l, r, &mut out);
        if out.len() >= 4096 {
            sh.push(&mut out, true);
            if sh.stop.load(Ordering::Relaxed) {
                return false;
            }
        }
    }
}

fn stream_thread(url: String, sh: Arc<Shared>) {
    while !sh.stop.load(Ordering::Relaxed) {
        sh.set_status("Connecting to the stream ...");
        let res = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(8))
            .timeout_read(Duration::from_secs(15))
            .build()
            .get(url.trim())
            .set("User-Agent", "RadioMoon")
            .set("Icy-MetaData", "1")
            .call();
        match res {
            Ok(resp) => {
                let metaint = resp.header("icy-metaint").and_then(|v| v.trim().parse::<usize>().ok()).filter(|&m| m > 0);
                let icy = Icy { inner: resp.into_reader(), metaint, left: metaint.unwrap_or(0), sh: sh.clone() };
                match Decoder::new(NoSeek(BufReader::with_capacity(64 * 1024, icy))) {
                    Ok(dec) => {
                        sh.set_status("Stream on air");
                        if !feed(dec.convert_samples::<f32>(), &sh) {
                            return;
                        }
                        sh.set_status("Stream ended, reconnecting ...");
                    }
                    Err(e) => sh.set_status(&format!("Stream format not supported: {e}")),
                }
            }
            Err(e) => sh.set_status(&format!("Stream connection failed: {e}")),
        }
        for _ in 0..30 {
            if sh.stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

// ---------------------------------------------------------------- MPX input (a ready composite signal)

/// Captures a ready FM composite signal (MPX) from a sound card input at exactly 192 kHz, one channel
/// (0 = left, 1 = right), with the level in dB. The blocks come at the sound card's pace.
pub fn start_mpx(device: &str, channel: usize, gain_db: f32) -> (SrcHandle, Receiver<Vec<f32>>) {
    let sh = Arc::new(Shared {
        q: Mutex::new(VecDeque::new()),
        cap: 0,
        stop: AtomicBool::new(false),
        gain: 10f32.powf(gain_db / 20.0),
        peak: AtomicU32::new(0),
        status: Mutex::new("Starting ...".into()),
        now: Mutex::new(String::new()),
    });
    let (tx, rx) = sync_channel::<Vec<f32>>(256);
    let (s2, device) = (sh.clone(), device.to_string());
    std::thread::spawn(move || mpx_capture_thread(device, channel, s2, tx));
    (SrcHandle { sh }, rx)
}

fn mpx_capture_thread(device: String, channel: usize, sh: Arc<Shared>, tx: std::sync::mpsc::SyncSender<Vec<f32>>) {
    use rodio::cpal::{SampleRate, StreamConfig};
    const FS: u32 = 192_000;
    let host = rodio::cpal::default_host();
    let dev = host.input_devices().ok().and_then(|mut it| it.find(|d| d.name().map(|n| n == device).unwrap_or(false))).or_else(|| host.default_input_device());
    let Some(dev) = dev else {
        sh.set_status("MPX input: no sound input found");
        return;
    };
    let name = dev.name().unwrap_or_default();
    // a configuration that runs at exactly 192 kHz (no resampling of the composite)
    let mut best: Option<(u32, rodio::cpal::SupportedStreamConfig)> = None;
    if let Ok(ranges) = dev.supported_input_configs() {
        for r in ranges {
            if r.min_sample_rate().0 <= FS && FS <= r.max_sample_rate().0 {
                let score = match r.sample_format() {
                    SampleFormat::F32 => 4,
                    SampleFormat::I32 => 3,
                    SampleFormat::I16 => 2,
                    _ => 0,
                };
                if score > 0 && best.as_ref().map(|b| score > b.0).unwrap_or(true) {
                    best = Some((score, r.with_sample_rate(SampleRate(FS))));
                }
            }
        }
    }
    let Some((_, sc)) = best else {
        sh.set_status(&format!("MPX input: \"{name}\" does not support 192 kHz recording"));
        return;
    };
    let fmt = sc.sample_format();
    let cfg: StreamConfig = sc.into();
    let ch = cfg.channels.max(1) as usize;
    let pick = channel.min(ch - 1);
    let (gain, sh2) = (sh.gain, sh.clone());
    let handle = move |d: &mut dyn Iterator<Item = f32>| {
        let mut blk: Vec<f32> = Vec::with_capacity(4096);
        let mut pk = 0f32;
        for (i, v) in d.enumerate() {
            if i % ch == pick {
                let x = v * gain;
                pk = pk.max(x.abs());
                blk.push(x);
            }
        }
        let old = f32::from_bits(sh2.peak.load(Ordering::Relaxed));
        sh2.peak.store(pk.max(old * 0.9).to_bits(), Ordering::Relaxed);
        let _ = tx.try_send(blk); // the program is behind: drop rather than block the sound card
    };
    let err = |e| eprintln!("MPX input: {e}");
    let stream = match fmt {
        SampleFormat::F32 => dev.build_input_stream(&cfg, move |d: &[f32], _| handle(&mut d.iter().copied()), err, None),
        SampleFormat::I32 => dev.build_input_stream(&cfg, move |d: &[i32], _| handle(&mut d.iter().map(|x| *x as f32 / 2_147_483_648.0)), err, None),
        _ => dev.build_input_stream(&cfg, move |d: &[i16], _| handle(&mut d.iter().map(|x| *x as f32 / 32_768.0)), err, None),
    };
    let stream = match stream {
        Ok(s) => s,
        Err(e) => {
            sh.set_status(&format!("MPX input: {e}"));
            return;
        }
    };
    if let Err(e) = stream.play() {
        sh.set_status(&format!("MPX input: {e}"));
        return;
    }
    sh.set_status(&format!("MPX input: {name}, 192 kHz, {} channel", if pick == 0 { "left" } else { "right" }));
    while !sh.stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(stream);
}

// ---------------------------------------------------------------- AES67 (and Dante in AES67 mode)

/// An AES67 stream to receive: RTP L24 / L16 at 44.1, 48 or 96 kHz, usually multicast.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Aes67Cfg {
    /// Multicast group (e.g. 239.69.1.10) or empty for unicast to this computer.
    pub group: String,
    pub port: u16,
    /// Address of the network interface to receive on (empty = the default interface).
    pub iface: String,
    /// "L24" | "L16"
    pub encoding: String,
    pub rate: u32,
    pub channels: u16,
    /// Channels used as left and right (0 = the first).
    pub ch_l: u16,
    pub ch_r: u16,
    /// Name of the stream (from its announcement), for display.
    pub name: String,
}

impl Default for Aes67Cfg {
    fn default() -> Self {
        Aes67Cfg { group: "239.69.1.10".into(), port: 5004, iface: String::new(), encoding: "L24".into(), rate: 48_000, channels: 2, ch_l: 0, ch_r: 1, name: String::new() }
    }
}

/// Reads an SDP description (as AES67 / Dante / Ravenna devices announce them with SAP).
pub fn parse_sdp(sdp: &str) -> Option<Aes67Cfg> {
    let mut c = Aes67Cfg { group: String::new(), ..Aes67Cfg::default() };
    let mut pt: Option<String> = None;
    let mut have_media = false;
    for line in sdp.lines().map(|l| l.trim()) {
        if let Some(v) = line.strip_prefix("s=") {
            c.name = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("c=IN IP4 ") {
            c.group = v.split('/').next().unwrap_or("").trim().to_string();
        } else if let Some(v) = line.strip_prefix("m=audio ") {
            let f: Vec<&str> = v.split_whitespace().collect();
            c.port = f.first()?.parse().ok()?;
            pt = f.get(2).map(|s| s.to_string());
            have_media = true;
        } else if let Some(v) = line.strip_prefix("a=rtpmap:") {
            // a=rtpmap:98 L24/48000/2
            let (p, enc) = v.split_once(' ')?;
            if pt.as_deref().map(|x| x == p).unwrap_or(true) {
                let parts: Vec<&str> = enc.split('/').collect();
                c.encoding = parts.first()?.to_ascii_uppercase();
                c.rate = parts.get(1).and_then(|r| r.parse().ok()).unwrap_or(48_000);
                c.channels = parts.get(2).and_then(|r| r.parse().ok()).unwrap_or(1);
            }
        }
    }
    if !have_media || !(c.encoding == "L24" || c.encoding == "L16") {
        return None;
    }
    c.ch_l = 0;
    c.ch_r = if c.channels > 1 { 1 } else { 0 };
    Some(c)
}

/// The SDP text inside a SAP announcement (RFC 2974); None for deletions and other payloads.
pub fn sap_sdp(p: &[u8]) -> Option<String> {
    if p.len() < 8 || p[0] >> 5 != 1 || p[0] & 0x04 != 0 {
        return None; // not SAP version 1, or a deletion
    }
    let ipv6 = p[0] & 0x10 != 0;
    let mut at = 4 + if ipv6 { 16 } else { 4 } + p[1] as usize * 4;
    let rest = p.get(at..)?;
    if rest.starts_with(b"v=0") {
        return Some(String::from_utf8_lossy(rest).to_string());
    }
    // a payload type such as "application/sdp\0" comes first
    let nul = rest.iter().position(|&b| b == 0)?;
    at += nul + 1;
    Some(String::from_utf8_lossy(p.get(at..)?).to_string())
}

/// Listens for stream announcements (SAP, 239.255.255.255 and 224.2.127.254, port 9875) and keeps the list.
pub struct Aes67Finder {
    pub list: Arc<Mutex<Vec<(Aes67Cfg, Instant)>>>,
    stop: Arc<AtomicBool>,
}

impl Aes67Finder {
    pub fn start(iface: &str) -> Aes67Finder {
        let list: Arc<Mutex<Vec<(Aes67Cfg, Instant)>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (l2, s2) = (list.clone(), stop.clone());
        let ifa: std::net::Ipv4Addr = iface.trim().parse().unwrap_or(std::net::Ipv4Addr::UNSPECIFIED);
        std::thread::spawn(move || {
            let Ok(sock) = reuse_socket(9875) else { return };
            for g in ["239.255.255.255", "224.2.127.254"] {
                if let Ok(a) = g.parse::<std::net::Ipv4Addr>() {
                    let _ = sock.join_multicast_v4(&a, &ifa);
                }
            }
            let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
            let mut buf = vec![0u8; 4096];
            while !s2.load(Ordering::Relaxed) {
                if let Ok((n, _)) = sock.recv_from(&mut buf) {
                    if let Some(c) = sap_sdp(&buf[..n]).and_then(|t| parse_sdp(&t)) {
                        if let Ok(mut g) = l2.lock() {
                            g.retain(|(x, _)| !(x.group == c.group && x.port == c.port));
                            g.push((c, Instant::now()));
                            g.sort_by(|a, b| a.0.name.cmp(&b.0.name));
                        }
                    }
                }
                // announcements come every 30 s or so: forget streams not heard for 5 minutes
                if let Ok(mut g) = l2.lock() {
                    g.retain(|(_, t)| t.elapsed() < Duration::from_secs(300));
                }
            }
        });
        Aes67Finder { list, stop }
    }

    pub fn streams(&self) -> Vec<Aes67Cfg> {
        self.list.lock().map(|g| g.iter().map(|(c, _)| c.clone()).collect()).unwrap_or_default()
    }
}

impl Drop for Aes67Finder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn reuse_socket(port: u16) -> std::io::Result<std::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    s.bind(&std::net::SocketAddrV4::new(std::net::Ipv4Addr::UNSPECIFIED, port).into())?;
    Ok(s.into())
}

/// Takes the samples of one RTP packet: big-endian L24 / L16, `ch` channels interleaved; appends L R as f32.
pub fn aes67_samples(payload: &[u8], bytes: usize, ch: usize, l: usize, r: usize, gain: f32, out: &mut Vec<f32>) -> usize {
    let frame = bytes * ch;
    let mut n = 0;
    for f in payload.chunks_exact(frame) {
        let at = |c: usize| -> f32 {
            let b = &f[c.min(ch - 1) * bytes..];
            if bytes == 3 {
                ((((b[0] as i32) << 24) | ((b[1] as i32) << 16) | ((b[2] as i32) << 8)) >> 8) as f32 / 8_388_608.0
            } else {
                i16::from_be_bytes([b[0], b[1]]) as f32 / 32_768.0
            }
        };
        out.push(at(l) * gain);
        out.push(at(r) * gain);
        n += 1;
    }
    n
}

/// Receives an AES67 stream. The blocks (interleaved stereo at the stream's rate) come at the sender's pace;
/// the sample rate of the blocks is returned too.
pub fn start_aes67(c: &Aes67Cfg, gain_db: f32) -> (SrcHandle, Receiver<Vec<f32>>, u32) {
    let sh = Arc::new(Shared {
        q: Mutex::new(VecDeque::new()),
        cap: 0,
        stop: AtomicBool::new(false),
        gain: 10f32.powf(gain_db / 20.0),
        peak: AtomicU32::new(0),
        status: Mutex::new("Starting ...".into()),
        now: Mutex::new(String::new()),
    });
    let rate = if [44_100, 48_000, 96_000].contains(&c.rate) { c.rate } else { 48_000 };
    let (tx, rx) = sync_channel::<Vec<f32>>(512);
    let (s2, cfg) = (sh.clone(), c.clone());
    std::thread::spawn(move || aes67_thread(cfg, rate, s2, tx));
    (SrcHandle { sh }, rx, rate)
}

fn aes67_thread(c: Aes67Cfg, rate: u32, sh: Arc<Shared>, tx: std::sync::mpsc::SyncSender<Vec<f32>>) {
    let sock = match reuse_socket(c.port) {
        Ok(s) => s,
        Err(e) => {
            sh.set_status(&format!("AES67: cannot listen on port {}: {e}", c.port));
            return;
        }
    };
    let ifa: std::net::Ipv4Addr = c.iface.trim().parse().unwrap_or(std::net::Ipv4Addr::UNSPECIFIED);
    let group = c.group.trim().parse::<std::net::Ipv4Addr>().ok().filter(|g| g.is_multicast());
    if let Some(g) = group {
        if let Err(e) = sock.join_multicast_v4(&g, &ifa) {
            sh.set_status(&format!("AES67: cannot join {g}: {e}"));
            return;
        }
    }
    let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
    let bytes = if c.encoding == "L16" { 2 } else { 3 };
    let ch = c.channels.max(1) as usize;
    let (l, r) = (c.ch_l as usize, c.ch_r as usize);
    let mut buf = vec![0u8; 9000];
    let mut blk: Vec<f32> = Vec::with_capacity(2048);
    let mut expected: Option<u32> = None;
    let (mut lost, mut packets, mut from) = (0u64, 0u64, String::from("-"));
    let mut shown = Instant::now() - Duration::from_secs(5);
    let block = (rate / 200) as usize; // hand on about every 5 ms
    let where_ = match group {
        Some(g) => format!("{g}:{}", c.port),
        None => format!("port {}", c.port),
    };
    sh.set_status(&format!("AES67: waiting for the stream on {where_} ..."));
    while !sh.stop.load(Ordering::Relaxed) {
        let got = sock.recv_from(&mut buf);
        if let Ok((n, a)) = got {
            let p = &buf[..n];
            if p.len() < 12 || p[0] >> 6 != 2 {
                continue;
            }
            let mut off = 12 + 4 * (p[0] & 0x0F) as usize;
            if p[0] & 0x10 != 0 && p.len() >= off + 4 {
                off += 4 + 4 * u16::from_be_bytes([p[off + 2], p[off + 3]]) as usize;
            }
            let mut end = p.len();
            if p[0] & 0x20 != 0 {
                end = end.saturating_sub(p[end - 1] as usize);
            }
            if off >= end {
                continue;
            }
            let ts = u32::from_be_bytes([p[4], p[5], p[6], p[7]]);
            let frames = (end - off) / (bytes * ch);
            if let Some(e) = expected {
                let d = ts.wrapping_sub(e) as i32;
                if d < 0 && d > -(rate as i32) {
                    continue; // late or doubled
                }
                if d > 0 && d < rate as i32 / 10 {
                    // lost packets: silence in their place, so the timing stays right
                    lost += 1;
                    blk.resize(blk.len() + d as usize * 2, 0.0);
                }
            }
            expected = Some(ts.wrapping_add(frames as u32));
            aes67_samples(&p[off..end], bytes, ch, l, r, sh.gain, &mut blk);
            packets += 1;
            from = a.ip().to_string();
        }
        if blk.len() >= block * 2 || (got.is_err() && !blk.is_empty()) {
            let pk = blk.iter().fold(0f32, |m, v| m.max(v.abs()));
            let old = f32::from_bits(sh.peak.load(Ordering::Relaxed));
            sh.peak.store(pk.max(old * 0.9).to_bits(), Ordering::Relaxed);
            let _ = tx.try_send(std::mem::take(&mut blk));
        }
        if shown.elapsed() > Duration::from_millis(700) {
            shown = Instant::now();
            let name = if c.name.is_empty() { String::new() } else { format!("\"{}\" ", c.name) };
            if packets == 0 {
                sh.set_status(&format!("AES67: waiting for the stream {name}on {where_} ..."));
            } else {
                sh.set_status(&format!("AES67 {name}from {from}: {} {} Hz, {ch} ch, lost packets {lost}", c.encoding, rate));
            }
        }
    }
}

#[cfg(test)]
mod aes67_tests {
    use super::*;

    #[test]
    fn sdp_and_sap() {
        let sdp = "v=0\r\no=- 1 1 IN IP4 192.168.1.20\r\ns=Studio A Main\r\nc=IN IP4 239.69.1.10/32\r\nt=0 0\r\nm=audio 5004 RTP/AVP 98\r\na=rtpmap:98 L24/48000/8\r\na=ptime:1\r\n";
        let c = parse_sdp(sdp).unwrap();
        assert_eq!((c.name.as_str(), c.group.as_str(), c.port, c.encoding.as_str(), c.rate, c.channels), ("Studio A Main", "239.69.1.10", 5004, "L24", 48_000, 8));
        let mut sap = vec![0x20, 0, 0x12, 0x34, 192, 168, 1, 20];
        sap.extend_from_slice(b"application/sdp\0");
        sap.extend_from_slice(sdp.as_bytes());
        assert_eq!(sap_sdp(&sap).and_then(|t| parse_sdp(&t)), Some(c));
    }

    #[test]
    fn l24_channel_pick() {
        // 2 frames of 4 channels, L24: pick channels 2 and 3
        let mut p = Vec::new();
        for f in 0..2i32 {
            for ch in 0..4i32 {
                let v = (f * 4 + ch) * 100_000;
                p.extend_from_slice(&[(v >> 16) as u8, (v >> 8) as u8, v as u8]);
            }
        }
        let mut out = Vec::new();
        assert_eq!(aes67_samples(&p, 3, 4, 2, 3, 1.0, &mut out), 2);
        let s = 8_388_608.0;
        assert_eq!(out, vec![200_000.0 / s, 300_000.0 / s, 600_000.0 / s, 700_000.0 / s]);
    }
}
