//! Analysis of the MPX composite signal (192 kHz, 1.0 = 75 kHz deviation): spectrum, waveform,
//! pilot / RDS / L+R / L-R levels and the peak deviation. Used by the MPX generator and by the receiver.
use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const FS: f32 = 192_000.0;
const N: usize = 8192;
const COLS: usize = 512; // spectrum columns, 0 .. 96 kHz
const WAVE: usize = 1152; // samples shown in the oscilloscope (6 ms)
const MAXF: f32 = 96_000.0; // Nyquist at 192 kHz

#[derive(Clone, Default)]
pub struct ScopeData {
    pub active: bool,
    /// spectrum in dB relative to 75 kHz deviation, COLS values for 0 .. 96 kHz
    pub spec: Vec<f32>,
    pub wave: Vec<f32>,
    /// levels as a fraction of 75 kHz deviation
    pub pilot: f32,
    pub rds: f32,
    pub sum: f32,
    pub diff: f32,
    pub dev_peak: f32,
    pub dev_max: f32,
}

pub type Scope = Arc<Mutex<ScopeData>>;

pub fn new_scope() -> Scope {
    Arc::new(Mutex::new(ScopeData::default()))
}

pub struct Analyzer {
    buf: Vec<f32>,
    win: Vec<f32>,
    cos: Vec<f32>,
    sin: Vec<f32>,
    last: Instant,
    peak: f32,
}

impl Analyzer {
    pub fn new() -> Self {
        let win: Vec<f32> = (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos()).collect();
        let cos: Vec<f32> = (0..N / 2).map(|i| (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos()).collect();
        let sin: Vec<f32> = (0..N / 2).map(|i| -(2.0 * std::f32::consts::PI * i as f32 / N as f32).sin()).collect();
        Analyzer { buf: Vec::with_capacity(N * 2), win, cos, sin, last: Instant::now() - Duration::from_secs(1), peak: 0.0 }
    }

    fn fft(&self, re: &mut [f32], im: &mut [f32]) {
        let n = re.len();
        let mut j = 0usize;
        for i in 1..n {
            let mut bit = n >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j ^= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let step = n / len;
            for s in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let (wr, wi) = (self.cos[k * step], self.sin[k * step]);
                    let (a, b) = (s + k, s + k + len / 2);
                    let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len <<= 1;
        }
    }

    /// Feeds composite samples (mono, 192 kHz). Cheap: one FFT at most every 40 ms.
    pub fn push(&mut self, s: &[f32], out: &Scope) {
        if s.is_empty() {
            return;
        }
        let blk = s.iter().fold(0.0f32, |a, b| a.max(b.abs()));
        self.peak = if blk > self.peak { blk } else { self.peak * 0.9 };
        self.buf.extend_from_slice(s);
        if self.buf.len() > N {
            let cut = self.buf.len() - N;
            self.buf.drain(..cut);
        }
        if self.buf.len() < N || self.last.elapsed() < Duration::from_millis(40) {
            return;
        }
        self.last = Instant::now();
        let mut re: Vec<f32> = self.buf.iter().zip(&self.win).map(|(x, w)| x * w).collect();
        let mut im = vec![0.0f32; N];
        self.fft(&mut re, &mut im);
        let g: f32 = self.win.iter().sum();
        let w2: f32 = self.win.iter().map(|w| w * w).sum();
        let binhz = FS / N as f32;
        let power: Vec<f32> = (0..N / 2).map(|k| re[k] * re[k] + im[k] * im[k]).collect();
        // equivalent sine amplitude of the energy between f1 and f2
        let band = |f1: f32, f2: f32| -> f32 {
            let (a, b) = ((f1 / binhz) as usize, ((f2 / binhz) as usize).min(N / 2 - 1));
            let e: f32 = power[a..=b].iter().sum();
            (4.0 * e / (N as f32 * w2)).sqrt()
        };
        let mut spec = Vec::with_capacity(COLS);
        let nb = ((MAXF / binhz) as usize).min(N / 2);
        for c in 0..COLS {
            let a = (c * nb / COLS).min(N / 2 - 1);
            let b = ((c + 1) * nb / COLS).max(a + 1).min(N / 2);
            let m = power[a..b].iter().fold(0.0f32, |x, y| x.max(*y)).sqrt() * 2.0 / g;
            spec.push(20.0 * m.max(1e-6).log10());
        }
        let wave: Vec<f32> = self.buf[N - WAVE..].to_vec();
        if let Ok(mut d) = out.lock() {
            d.active = true;
            d.spec = spec;
            d.wave = wave;
            d.sum = band(30.0, 15_000.0);
            d.pilot = band(18_500.0, 19_500.0);
            d.diff = band(23_000.0, 53_000.0);
            d.rds = band(54_600.0, 59_400.0);
            d.dev_peak = self.peak;
            d.dev_max = d.dev_max.max(self.peak);
        }
    }
}

/// Draws the readouts, the spectrum (0-96 kHz) and the oscilloscope.
pub fn draw(ui: &mut egui::Ui, scope: &Scope) {
    let d = scope.lock().map(|g| g.clone()).unwrap_or_default();
    let kz = |v: f32| format!("{:.1} kHz ({:.1}%)", v * 75.0, v * 100.0);
    egui::Grid::new("mpx_levels").num_columns(4).spacing([18.0, 4.0]).show(ui, |ui| {
        ui.label("Pilot 19 kHz");
        ui.label(egui::RichText::new(kz(d.pilot)).color(Color32::from_rgb(120, 220, 140)));
        ui.label("RDS 57 kHz");
        ui.label(egui::RichText::new(kz(d.rds)).color(Color32::from_rgb(255, 200, 90)));
        ui.end_row();
        ui.label("L+R (audio)");
        ui.label(kz(d.sum));
        ui.label("L-R (38 kHz)");
        ui.label(kz(d.diff));
        ui.end_row();
        let hot = |v: f32| if v > 1.0 { Color32::from_rgb(255, 90, 90) } else { Color32::WHITE };
        ui.label("Deviation peak");
        ui.label(egui::RichText::new(kz(d.dev_peak)).color(hot(d.dev_peak)));
        ui.label("Maximum");
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(kz(d.dev_max)).color(hot(d.dev_max)));
            if ui.small_button("Reset").clicked() {
                if let Ok(mut g) = scope.lock() {
                    g.dev_max = 0.0;
                }
            }
        });
        ui.end_row();
    });
    ui.add_space(4.0);
    let w = ui.available_width().max(200.0);
    // ---- spectrum
    let (resp, p) = ui.allocate_painter(Vec2::new(w, 150.0), Sense::hover());
    let r = resp.rect;
    p.rect_filled(r, 2.0, Color32::from_rgb(8, 10, 14));
    let (lo, hi) = (-80.0f32, 0.0f32);
    let y_of = |db: f32| r.bottom() - (db.clamp(lo, hi) - lo) / (hi - lo) * r.height();
    for db in [-60.0, -40.0, -20.0] {
        p.line_segment([Pos2::new(r.left(), y_of(db)), Pos2::new(r.right(), y_of(db))], Stroke::new(1.0_f32, Color32::from_rgb(35, 40, 48)));
        p.text(Pos2::new(r.left() + 3.0, y_of(db) - 1.0), egui::Align2::LEFT_BOTTOM, format!("{db:.0} dB"), egui::FontId::proportional(10.0), Color32::from_rgb(110, 118, 130));
    }
    for (f, name) in [(15_000.0, "15"), (19_000.0, "19"), (38_000.0, "38"), (57_000.0, "57"), (76_000.0, "76"), (90_000.0, "90")] {
        let x = r.left() + f / MAXF * r.width();
        p.line_segment([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())], Stroke::new(1.0_f32, Color32::from_rgb(45, 52, 64)));
        p.text(Pos2::new(x + 2.0, r.bottom() - 1.0), egui::Align2::LEFT_BOTTOM, format!("{name} kHz"), egui::FontId::proportional(10.0), Color32::from_rgb(130, 140, 155));
    }
    if d.spec.len() == COLS {
        let pts: Vec<Pos2> = d.spec.iter().enumerate().map(|(i, db)| Pos2::new(r.left() + (i as f32 + 0.5) / COLS as f32 * r.width(), y_of(*db))).collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.3_f32, Color32::from_rgb(90, 190, 255))));
    } else {
        p.text(r.center(), egui::Align2::CENTER_CENTER, "No signal", egui::FontId::proportional(13.0), Color32::GRAY);
    }
    ui.add_space(4.0);
    // ---- oscilloscope (waveform of the composite, +-125% = +-93.75 kHz)
    let (resp, p) = ui.allocate_painter(Vec2::new(w, 110.0), Sense::hover());
    let r: Rect = resp.rect;
    p.rect_filled(r, 2.0, Color32::from_rgb(8, 10, 14));
    let y_of = |v: f32| r.center().y - v.clamp(-1.25, 1.25) / 1.25 * (r.height() / 2.0);
    for v in [1.0f32, -1.0] {
        p.line_segment([Pos2::new(r.left(), y_of(v)), Pos2::new(r.right(), y_of(v))], Stroke::new(1.0_f32, Color32::from_rgb(110, 40, 40)));
    }
    p.line_segment([Pos2::new(r.left(), r.center().y), Pos2::new(r.right(), r.center().y)], Stroke::new(1.0_f32, Color32::from_rgb(35, 40, 48)));
    p.text(Pos2::new(r.left() + 3.0, y_of(1.0) + 1.0), egui::Align2::LEFT_TOP, "+75 kHz", egui::FontId::proportional(10.0), Color32::from_rgb(170, 90, 90));
    if !d.wave.is_empty() {
        let n = d.wave.len();
        let pts: Vec<Pos2> = d.wave.iter().enumerate().map(|(i, v)| Pos2::new(r.left() + i as f32 / (n - 1) as f32 * r.width(), y_of(*v))).collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.0_f32, Color32::from_rgb(120, 230, 150))));
    }
    ui.ctx().request_repaint_after(Duration::from_millis(60));
}
