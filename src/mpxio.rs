//! Shared pieces of the MPX sender and receiver: sample queue and the 192 kHz sound-card output.
use rodio::cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rodio::cpal::{FromSample, SampleFormat, SampleRate, SizedSample, Stream, StreamConfig};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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
}

impl Queue {
    pub fn new(prefill_ms: u32) -> Arc<Queue> {
        let prefill = (FS as usize * prefill_ms as usize) / 1000;
        Arc::new(Queue { q: Mutex::new(VecDeque::with_capacity(prefill * 4)), prefill, cap: prefill * 3 + FS as usize / 5, under: AtomicU32::new(0), drops: AtomicU32::new(0) })
    }
    pub fn push(&self, v: &[f32]) {
        if let Ok(mut g) = self.q.lock() {
            g.extend(v.iter().copied());
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

pub fn card_thread(name: String, q: Arc<Queue>, stop: Arc<AtomicBool>, status: Text, label: &'static str) {
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
