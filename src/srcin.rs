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
    let mut handle = move |d: &mut dyn Iterator<Item = f32>| {
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
