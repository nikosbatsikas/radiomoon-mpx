//! Audio sources for the stand-alone MPX transmitter: a sound card input, an internet radio stream or a folder of
//! audio files played in random order. Everything is delivered as 44.1 kHz stereo blocks at real-time pace.
use rodio::cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rodio::cpal::SampleFormat;
use rodio::{Decoder, Source};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{sync_channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const EXTS: [&str; 9] = ["mp3", "wav", "flac", "ogg", "m4a", "aac", "mp4", "opus", "wma"];

#[derive(Clone, Debug)]
pub struct SrcCfg {
    /// "input" | "stream" | "folder"
    pub kind: String,
    pub device: String,
    pub url: String,
    pub folder: String,
    pub shuffle: bool,
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
    /// Title of the file being played (folder source) or the song title sent by the stream server.
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
        "folder" => {
            std::thread::spawn(move || folder_thread(c.folder, c.shuffle, s2));
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

// ---------------------------------------------------------------- folder of audio files
fn scan(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth < 8 {
                scan(&p, out, depth + 1);
            }
        } else if p.extension().and_then(|x| x.to_str()).map(|x| EXTS.contains(&x.to_lowercase().as_str())).unwrap_or(false) {
            out.push(p);
        }
    }
}

fn folder_thread(folder: String, shuffle: bool, sh: Arc<Shared>) {
    use rand::seq::SliceRandom;
    while !sh.stop.load(Ordering::Relaxed) {
        let mut files = Vec::new();
        scan(std::path::Path::new(&folder), &mut files, 0);
        if files.is_empty() {
            sh.set_status("No audio files in the folder");
            for _ in 0..30 {
                if sh.stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            continue;
        }
        files.sort();
        if shuffle {
            files.shuffle(&mut rand::thread_rng()); // a different order every round and every start
        }
        for p in files {
            if sh.stop.load(Ordering::Relaxed) {
                return;
            }
            let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let dec = match File::open(&p).ok().and_then(|f| Decoder::new(BufReader::new(f)).ok()) {
                Some(d) => d,
                None => continue,
            };
            if let Ok(mut n) = sh.now.lock() {
                *n = name.clone();
            }
            sh.set_status(&format!("Playing: {name}"));
            if !feed(dec.convert_samples::<f32>(), &sh) {
                return;
            }
        }
    }
}
