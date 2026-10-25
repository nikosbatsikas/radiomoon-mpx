//! Shared pieces of the MPX sender and receiver: sample queue and the 192 kHz sound-card output.
use rodio::cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rodio::cpal::{FromSample, SampleFormat, SampleRate, SizedSample, Stream, StreamConfig};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const FS: u32 = 192_000;

pub type Text = Arc<Mutex<String>>;

pub fn set(t: &Text, s: &str) {
    if let Ok(mut g) = t.lock() {
        *g = s.to_string();
    }
}

pub fn get(t: &Text) -> String {
    t.lock().map(|g| g.clone()).unwrap_or_default()
}

// ---------------------------------------------------------------- sample queue between threads

pub struct Queue {
    pub q: Mutex<VecDeque<f32>>,
    pub prefill: usize,
    cap: usize,
    pub under: AtomicU32,
    pub drops: AtomicU32,
    /// Number of values ever pushed (gives every value a fixed number: total - len is the next one popped).
    total: AtomicU64,
}

impl Queue {
    pub fn new(prefill_ms: u32) -> Arc<Queue> {
        let prefill = (FS as usize * prefill_ms as usize) / 1000;
        Arc::new(Queue { q: Mutex::new(VecDeque::with_capacity(prefill * 4)), prefill, cap: prefill * 3 + FS as usize / 5, under: AtomicU32::new(0), drops: AtomicU32::new(0), total: AtomicU64::new(0) })
    }
    pub fn push(&self, v: &[f32]) {
        if let Ok(mut g) = self.q.lock() {
            g.extend(v.iter().copied());
            self.total.fetch_add(v.len() as u64, Ordering::Relaxed);
            if g.len() > self.cap {
                let cut = g.len() - self.prefill;
                g.drain(..cut);
                self.drops.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    pub fn len(&self) -> usize {
        self.q.lock().map(|g| g.len()).unwrap_or(0)
    }
    pub fn ms(&self) -> u32 {
        (self.len() as u64 * 1000 / FS as u64) as u32
    }
    /// Like `pop_n`, and returns the number of the first value popped (counted from the start of the stream).
    pub fn pop_n_at(&self, n: usize, out: &mut Vec<f32>) -> Option<u64> {
        let mut g = self.q.lock().ok()?;
        if g.len() < n {
            return None;
        }
        let first = self.total.load(Ordering::Relaxed) - g.len() as u64;
        out.extend(g.drain(..n));
        Some(first)
    }

    #[allow(dead_code)]
    pub fn pop_n(&self, n: usize, out: &mut Vec<f32>) -> bool {
        if let Ok(mut g) = self.q.lock() {
            if g.len() >= n {
                out.extend(g.drain(..n));
                return true;
            }
        }
        false
    }
}

// ---------------------------------------------------------------- sound card output (192 kHz)

fn find_device(name: &str) -> Option<rodio::cpal::Device> {
    let host = rodio::cpal::default_host();
    if !name.is_empty() {
        if let Ok(devs) = host.output_devices() {
            for d in devs {
                if d.name().map(|n| n == name).unwrap_or(false) {
                    return Some(d);
                }
            }
        }
    }
    host.default_output_device()
}

fn build<T: SizedSample + FromSample<f32>>(dev: &rodio::cpal::Device, cfg: &StreamConfig, q: Arc<Queue>) -> Result<Stream, String> {
    let ch = cfg.channels as usize;
    let mut running = false;
    dev.build_output_stream(
        cfg,
        move |data: &mut [T], _| {
            let Ok(mut g) = q.q.lock() else { return };
            if !running && g.len() >= q.prefill {
                running = true;
            }
            for frame in data.chunks_mut(ch) {
                let v = if running {
                    match g.pop_front() {
                        Some(v) => v,
                        None => {
                            running = false; // buffer ran dry: wait until it has refilled
                            q.under.fetch_add(1, Ordering::Relaxed);
                            0.0
                        }
                    }
                } else {
                    0.0
                };
                for s in frame.iter_mut() {
                    *s = T::from_sample(v);
                }
            }
        },
        |e| eprintln!("MPX output error: {e}"),
        None,
    )
    .map_err(|e| e.to_string())
}

/// Opens the named output at exactly 192 kHz (no resampling). The stream must stay on the thread that made it.
fn open_card(name: &str, q: Arc<Queue>) -> Result<Stream, String> {
    let dev = find_device(name).ok_or("No sound output found")?;
    let ranges = dev.supported_output_configs().map_err(|e| e.to_string())?;
    let mut best: Option<(u32, rodio::cpal::SupportedStreamConfig)> = None;
    for r in ranges {
        if r.min_sample_rate().0 <= FS && FS <= r.max_sample_rate().0 {
            let fmt_score = match r.sample_format() {
                SampleFormat::F32 => 3,
                SampleFormat::I32 => 2,
                SampleFormat::I16 => 1,
                _ => 0,
            };
            if fmt_score == 0 {
                continue;
            }
            let score = fmt_score + if r.channels() == 2 { 10 } else { 0 };
            if best.as_ref().map(|b| score > b.0).unwrap_or(true) {
                best = Some((score, r.with_sample_rate(SampleRate(FS))));
            }
        }
    }
    let Some((_, sc)) = best else {
        return Err("This sound device does not support 192 kHz output".into());
    };
    let fmt = sc.sample_format();
    let cfg: StreamConfig = sc.into();
    let stream = match fmt {
        SampleFormat::F32 => build::<f32>(&dev, &cfg, q)?,
        SampleFormat::I32 => build::<i32>(&dev, &cfg, q)?,
        _ => build::<i16>(&dev, &cfg, q)?,
    };
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

/// The output "no sound card": takes the samples at the pace of 192 kHz by the computer's clock and plays nothing.
/// For tests (the receiving, the analysis, the log and SFN work as with a card) and for sites without a card yet.
pub const NO_CARD: &str = "No sound card (test output)";

/// Calls `f` with blocks of 10 ms (1920 samples) at real-time pace until `stop`, like a sound card with an exact
/// clock: the second argument is the time (ns, from now) at which the block would be heard - negative when the
/// computer was busy and the block comes late (it is still taken, as a card's buffer would be).
fn paced(stop: &AtomicBool, mut f: impl FnMut(&mut Vec<f32>, i128)) {
    let t0 = std::time::Instant::now();
    let mut n: u64 = 0;
    let mut buf = vec![0f32; 1920];
    while !stop.load(Ordering::Relaxed) {
        let due = Duration::from_micros(n * 10_000);
        n += 1;
        if let Some(w) = due.checked_sub(t0.elapsed()) {
            std::thread::sleep(w);
        }
        let heard_in = due.as_nanos() as i128 - t0.elapsed().as_nanos() as i128;
        f(&mut buf, heard_in);
    }
}

pub fn card_thread(name: String, q: Arc<Queue>, stop: Arc<AtomicBool>, status: Text, label: &'static str) {
    if name == NO_CARD {
        std::thread::spawn(move || {
            let mut last = std::time::Instant::now();
            let mut out = Vec::with_capacity(1920);
            paced(&stop, |_, _| {
                out.clear();
                if !q.pop_n(1920, &mut out) {
                    q.under.fetch_add(1, Ordering::Relaxed);
                }
                if last.elapsed() > Duration::from_millis(400) {
                    last = std::time::Instant::now();
                    set(&status, &format!("{label}: none (test output, nothing is played), buffer {} ms", q.ms()));
                }
            });
        });
        return;
    }
    std::thread::spawn(move || {
        let stream = match open_card(&name, q.clone()) {
            Ok(s) => s,
            Err(e) => {
                set(&status, &format!("{label}: {e}"));
                return;
            }
        };
        while !stop.load(Ordering::Relaxed) {
            set(&status, &format!("{label}: 192 kHz, buffer {} ms, underruns {}, dropped {}", q.ms(), q.under.load(Ordering::Relaxed), q.drops.load(Ordering::Relaxed)));
            std::thread::sleep(Duration::from_millis(400));
        }
        drop(stream);
    });
}


#[allow(dead_code)]
pub fn track_peak(s: &[f32], pk: &AtomicU32) {
    let m = s.iter().fold(0.0f32, |a, b| a.max(b.abs()));
    let old = f32::from_bits(pk.load(Ordering::Relaxed));
    pk.store(if m > old { m } else { old * 0.95 }.to_bits(), Ordering::Relaxed);
}


// ---------------------------------------------------------------- SFN: sound card output at the time stamps

fn build_sfn<T: SizedSample + FromSample<f32>>(dev: &rodio::cpal::Device, cfg: &StreamConfig, pl: Arc<crate::sfn::SfnPlayout>) -> Result<Stream, String> {
    let ch = cfg.channels as usize;
    let mut mono: Vec<f32> = Vec::new();
    dev.build_output_stream(
        cfg,
        move |data: &mut [T], info: &rodio::cpal::OutputCallbackInfo| {
            let frames = data.len() / ch;
            mono.resize(frames, 0.0);
            // how long until the first sample of this block is heard, as the driver reports it
            let ts = info.timestamp();
            let lat = ts.playback.duration_since(&ts.callback).map(|d| d.as_nanos() as i128).unwrap_or(0);
            pl.render(&mut mono, lat);
            for (frame, v) in data.chunks_mut(ch).zip(&mono) {
                for s in frame.iter_mut() {
                    *s = T::from_sample(*v);
                }
            }
        },
        |e| eprintln!("MPX output error: {e}"),
        None,
    )
    .map_err(|e| e.to_string())
}

/// Like `card_thread`, but every sample is played at its SFN time (see sfn.rs).
pub fn sfn_card_thread(name: String, pl: Arc<crate::sfn::SfnPlayout>, stop: Arc<AtomicBool>, status: Text, label: &'static str) {
    if name == NO_CARD {
        std::thread::spawn(move || {
            let mut last = std::time::Instant::now();
            paced(&stop, |buf, heard_in| {
                pl.render(buf, heard_in);
                if last.elapsed() > Duration::from_millis(400) {
                    last = std::time::Instant::now();
                    set(&status, &format!("{label}: none (test output, nothing is played), SFN playout, buffer {} ms", pl.ms()));
                }
            });
        });
        return;
    }
    std::thread::spawn(move || {
        let open = || -> Result<Stream, String> {
            let dev = find_device(&name).ok_or("No sound output found")?;
            let ranges = dev.supported_output_configs().map_err(|e| e.to_string())?;
            let mut best: Option<(u32, rodio::cpal::SupportedStreamConfig)> = None;
            for r in ranges {
                if r.min_sample_rate().0 <= FS && FS <= r.max_sample_rate().0 {
                    let fmt_score = match r.sample_format() {
                        SampleFormat::F32 => 3,
                        SampleFormat::I32 => 2,
                        SampleFormat::I16 => 1,
                        _ => 0,
                    };
                    if fmt_score == 0 {
                        continue;
                    }
                    let score = fmt_score + if r.channels() == 2 { 10 } else { 0 };
                    if best.as_ref().map(|b| score > b.0).unwrap_or(true) {
                        best = Some((score, r.with_sample_rate(SampleRate(FS))));
                    }
                }
            }
            let Some((_, sc)) = best else {
                return Err("This sound device does not support 192 kHz output".into());
            };
            let fmt = sc.sample_format();
            let cfg: StreamConfig = sc.into();
            let stream = match fmt {
                SampleFormat::F32 => build_sfn::<f32>(&dev, &cfg, pl.clone())?,
                SampleFormat::I32 => build_sfn::<i32>(&dev, &cfg, pl.clone())?,
                _ => build_sfn::<i16>(&dev, &cfg, pl.clone())?,
            };
            stream.play().map_err(|e| e.to_string())?;
            Ok(stream)
        };
        let stream = match open() {
            Ok(s) => s,
            Err(e) => {
                set(&status, &format!("{label}: {e}"));
                // without a playing card the SFN playout cannot run: say so in the SFN state
                if let Ok(mut s) = pl.status.lock() {
                    s.state = format!("Not playing: {e}. Choose a sound card that plays 192 kHz (Output tab), or \"{NO_CARD}\" for a test.");
                }
                return;
            }
        };
        while !stop.load(Ordering::Relaxed) {
            set(&status, &format!("{label}: 192 kHz, SFN playout, buffer {} ms", pl.ms()));
            std::thread::sleep(Duration::from_millis(400));
        }
        drop(stream);
    });
}
