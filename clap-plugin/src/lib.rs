//! RadioMoon MPX Transmitter as a CLAP plug-in: put it on the master (or any stereo) track of an audio program
//! (REAPER, Bitwig, FL Studio, Studio One ...). The audio passes through unchanged; the plug-in builds the FM
//! multiplex from it with the settings of the MPX Transmitter (its settings file) and sends it to the IP
//! receivers / the 192 kHz sound card, with RDS, Compact MPX, SFN and QoS. The Transmitter window shows the
//! plug-in's live status and is where the settings are changed (level and RDS changes reach the plug-in at once,
//! the others restart its engine).
mod gui;

use clap_sys::entry::clap_plugin_entry;
use clap_sys::ext::audio_ports::{clap_audio_port_info, clap_plugin_audio_ports, CLAP_AUDIO_PORT_IS_MAIN, CLAP_EXT_AUDIO_PORTS, CLAP_PORT_STEREO};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::{clap_plugin, clap_plugin_descriptor};
use clap_sys::plugin_features::{CLAP_PLUGIN_FEATURE_AUDIO_EFFECT, CLAP_PLUGIN_FEATURE_STEREO, CLAP_PLUGIN_FEATURE_UTILITY};
use clap_sys::process::{clap_process, clap_process_status, CLAP_PROCESS_CONTINUE};
use clap_sys::version::CLAP_VERSION;
use radiomoon_mpx::model::MpxCfg;
use radiomoon_mpx::mpx::MpxHandle;
use radiomoon_mpx::status::{self, TxStatus};
use std::collections::VecDeque;
use std::ffi::{c_char, c_void, CStr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

pub const PLUGIN_ID: &CStr = c"com.radiomoon.mpx-transmitter";

// ---------------------------------------------------------------- settings

/// Folder of the headless Transmitter (radiomoon-mpx-txd); `RADIOMOON_MPX_DATA` points elsewhere.
const HEADLESS_DIR: &str = "/var/lib/radiomoon-mpx";

/// The Transmitter's settings file, the first that exists of:
/// - next to the plug-in file or in the folder above it (the portable zip keeps the plug-in in its CLAP folder);
/// - the folder in `RADIOMOON_MPX_DATA`;
/// - on Linux the headless Transmitter's folder (/var/lib/radiomoon-mpx): set up on its web page, e.g. when a
///   server program such as radiomoon-server hosts the plug-in on a computer without desktop (used when the
///   headless Transmitter is installed there, i.e. its web login exists);
/// otherwise the shared folder of the installed Transmitter window.
fn settings_path(plugin_dir: Option<&Path>) -> PathBuf {
    if let Some(d) = plugin_dir {
        for dir in [Some(d), d.parent()].into_iter().flatten() {
            let p = dir.join("mpx-transmitter.json");
            if p.exists() {
                return p;
            }
        }
    }
    if let Some(d) = std::env::var_os("RADIOMOON_MPX_DATA") {
        return PathBuf::from(d).join("mpx-transmitter.json");
    }
    if cfg!(target_os = "linux") && Path::new(HEADLESS_DIR).join("web-login.json").exists() {
        // the headless Transmitter is installed here (its file comes with the first "Save and apply")
        return Path::new(HEADLESS_DIR).join("mpx-transmitter.json");
    }
    status::shared_dir().join("mpx-transmitter.json")
}

/// The MPX part and the level of the Transmitter's settings.
pub fn load(path: &Path) -> (MpxCfg, f32) {
    let v: serde_json::Value = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(serde_json::Value::Null);
    let mut mpx: MpxCfg = v.get("mpx").cloned().and_then(|m| serde_json::from_value(m).ok()).unwrap_or_default();
    mpx.migrate();
    let gain_db = v.get("gain_db").and_then(|g| g.as_f64()).unwrap_or(0.0) as f32;
    (mpx, gain_db)
}

fn modified(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Settings that need a new engine when they change (the rest is applied while running).
fn restart_key(c: &MpxCfg) -> String {
    serde_json::to_string(&(c.card_on, &c.card_device, c.ip_on, &c.targets, c.sfn_on, &c.clock)).unwrap_or_default()
}

// ---------------------------------------------------------------- the engine (outside the audio thread)

/// Runs while the plug-in is active: takes the audio from the ring, feeds the transmitter, follows the settings
/// file and writes the live status for the Transmitter window.
struct Engine {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Engine {
    fn start(rate: u32, ring: Arc<Mutex<VecDeque<f32>>>, path: PathBuf) -> Engine {
        let stop = Arc::new(AtomicBool::new(false));
        let st = stop.clone();
        let thread = std::thread::spawn(move || {
            let st_path = status::path_for(&path);
            let (mut cfg, mut gain_db) = load(&path);
            let mut stamp = modified(&path);
            let mut key = restart_key(&cfg);
            let (mut tx, rx) = sync_channel::<Vec<f32>>(256);
            let mut m = MpxHandle::start_rate(&cfg, rx, rate);
            let mut checked = Instant::now();
            let mut peak = 0f32;
            while !st.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(5));
                // the audio of the last few ms
                let blk: Vec<f32> = match ring.lock() {
                    Ok(mut g) => g.drain(..).collect(),
                    Err(_) => Vec::new(),
                };
                if !blk.is_empty() {
                    let g = 10f32.powf(gain_db / 20.0);
                    let blk: Vec<f32> = if (g - 1.0).abs() > 1e-6 { blk.iter().map(|v| v * g).collect() } else { blk };
                    peak = blk.iter().fold(peak * 0.95, |a, b| a.max(b.abs()));
                    let _ = tx.try_send(blk);
                }
                if checked.elapsed() >= Duration::from_millis(250) {
                    checked = Instant::now();
                    // settings changed in the Transmitter window?
                    let now = modified(&path);
                    if now != stamp {
                        stamp = now;
                        let (c, gd) = load(&path);
                        gain_db = gd;
                        if restart_key(&c) != key {
                            key = restart_key(&c);
                            m.stop();
                            let (t2, r2) = sync_channel::<Vec<f32>>(256);
                            tx = t2;
                            m = MpxHandle::start_rate(&c, r2, rate);
                        } else {
                            m.update(&c);
                        }
                        cfg = c;
                    }
                    let snap = TxStatus {
                        time_ms: status::now_ms(),
                        status: m.status(),
                        source: format!("CLAP plug-in: audio from the host, {rate} Hz, level {:.0}%", peak * 100.0),
                        now: String::new(),
                        peak: m.peak(),
                        card: m.card_status(),
                        links: m.links(),
                        scope: m.scope.lock().map(|g| g.clone()).unwrap_or_default(),
                        clock: m.clock_state(),
                        engine: "plugin".into(),
                    };
                    status::write(&st_path, &snap);
                }
            }
            let _ = cfg;
            m.stop();
            status::remove(&st_path);
        });
        Engine { stop, thread: Some(thread) }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

// ---------------------------------------------------------------- the plug-in

struct Plugin {
    raw: clap_plugin,
    /// Interleaved stereo for the engine; the audio thread only uses try_lock and never allocates.
    ring: Arc<Mutex<VecDeque<f32>>>,
    engine: Option<Engine>,
    /// The settings file (chosen when the plug-in is made; the window edits the same file).
    settings: PathBuf,
    /// Its window: the Transmitter program in plug-in mode.
    gui: gui::Gui,
}

static PLUGIN_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

unsafe fn this<'a>(p: *const clap_plugin) -> &'a mut Plugin {
    &mut *((*p).plugin_data as *mut Plugin)
}

unsafe fn gui_of(p: *const clap_plugin) -> Option<&'static mut gui::Gui> {
    if p.is_null() || (*p).plugin_data.is_null() {
        return None;
    }
    Some(&mut this(p).gui)
}

unsafe extern "C" fn p_init(_p: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn p_destroy(p: *const clap_plugin) {
    let me = (*p).plugin_data as *mut Plugin;
    drop(Box::from_raw(me));
}

unsafe extern "C" fn p_activate(p: *const clap_plugin, sample_rate: f64, _min: u32, _max: u32) -> bool {
    let me = this(p);
    let rate = sample_rate.round() as u32;
    // two seconds of stereo: enough for the engine's 5 ms steps, also when the host renders in bursts
    me.ring = Arc::new(Mutex::new(VecDeque::with_capacity(rate as usize * 4)));
    me.engine = Some(Engine::start(rate, me.ring.clone(), me.settings.clone()));
    true
}

unsafe extern "C" fn p_deactivate(p: *const clap_plugin) {
    this(p).engine = None;
}

unsafe extern "C" fn p_start(_p: *const clap_plugin) -> bool {
    true
}

unsafe extern "C" fn p_stop(_p: *const clap_plugin) {}

unsafe extern "C" fn p_reset(_p: *const clap_plugin) {}

unsafe extern "C" fn p_process(p: *const clap_plugin, pr: *const clap_process) -> clap_process_status {
    let me = this(p);
    let pr = &*pr;
    let n = pr.frames_count as usize;
    if pr.audio_inputs_count == 0 || pr.audio_inputs.is_null() || n == 0 {
        return CLAP_PROCESS_CONTINUE;
    }
    let inb = &*pr.audio_inputs;
    if inb.data32.is_null() || inb.channel_count == 0 {
        return CLAP_PROCESS_CONTINUE;
    }
    let l = std::slice::from_raw_parts(*inb.data32, n);
    let r = if inb.channel_count > 1 { std::slice::from_raw_parts(*inb.data32.add(1), n) } else { l };
    // the audio goes on unchanged
    if pr.audio_outputs_count > 0 && !pr.audio_outputs.is_null() {
        let outb = &*pr.audio_outputs;
        if !outb.data32.is_null() {
            for c in 0..outb.channel_count.min(inb.channel_count.max(1)) as usize {
                let src = if c == 0 { l } else { r };
                let dst = *outb.data32.add(c);
                if dst as *const f32 != src.as_ptr() {
                    std::ptr::copy_nonoverlapping(src.as_ptr(), dst, n);
                }
            }
        }
    }
    // a copy for the transmitter (dropped if the engine is busy or behind: never wait in the audio thread)
    if let Ok(mut g) = me.ring.try_lock() {
        if g.len() + n * 2 <= g.capacity() {
            for k in 0..n {
                g.push_back(l[k]);
                g.push_back(r[k]);
            }
        }
    }
    CLAP_PROCESS_CONTINUE
}

unsafe extern "C" fn ports_count(_p: *const clap_plugin, _is_input: bool) -> u32 {
    1
}

unsafe extern "C" fn ports_get(_p: *const clap_plugin, index: u32, is_input: bool, info: *mut clap_audio_port_info) -> bool {
    if index != 0 || info.is_null() {
        return false;
    }
    let i = &mut *info;
    i.id = 0;
    let name: &[u8] = if is_input { b"Audio in\0" } else { b"Audio out\0" };
    i.name = [0; clap_sys::string_sizes::CLAP_NAME_SIZE];
    for (d, s) in i.name.iter_mut().zip(name) {
        *d = *s as c_char;
    }
    i.flags = CLAP_AUDIO_PORT_IS_MAIN;
    i.channel_count = 2;
    i.port_type = CLAP_PORT_STEREO.as_ptr();
    i.in_place_pair = 0;
    true
}

static AUDIO_PORTS: clap_plugin_audio_ports = clap_plugin_audio_ports { count: Some(ports_count), get: Some(ports_get) };

unsafe extern "C" fn p_get_extension(_p: *const clap_plugin, id: *const c_char) -> *const c_void {
    if id.is_null() {
        return std::ptr::null();
    }
    let id = CStr::from_ptr(id);
    if id == CLAP_EXT_AUDIO_PORTS {
        return &AUDIO_PORTS as *const _ as *const c_void;
    }
    if id == clap_sys::ext::gui::CLAP_EXT_GUI {
        return gui::ext_ptr();
    }
    std::ptr::null()
}

unsafe extern "C" fn p_on_main_thread(p: *const clap_plugin) {
    if let Some(g) = gui_of(p) {
        g.main_thread();
    }
}

// ---------------------------------------------------------------- descriptor, factory, entry

struct SyncPtrs([*const c_char; 4]);
unsafe impl Sync for SyncPtrs {}
static FEATURES: SyncPtrs = SyncPtrs([CLAP_PLUGIN_FEATURE_AUDIO_EFFECT.as_ptr(), CLAP_PLUGIN_FEATURE_UTILITY.as_ptr(), CLAP_PLUGIN_FEATURE_STEREO.as_ptr(), std::ptr::null()]);

const VERSION: &CStr = match CStr::from_bytes_with_nul(concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes()) {
    Ok(v) => v,
    Err(_) => c"0.0.0",
};

static DESCRIPTOR: clap_plugin_descriptor = clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: PLUGIN_ID.as_ptr(),
    name: c"RadioMoon MPX Transmitter".as_ptr(),
    vendor: c"RadioMoon".as_ptr(),
    url: c"https://github.com/nikosbatsikas/radiomoon-mpx".as_ptr(),
    manual_url: c"https://github.com/nikosbatsikas/radiomoon-mpx#clap-plug-in".as_ptr(),
    support_url: c"https://github.com/nikosbatsikas/radiomoon-mpx/issues".as_ptr(),
    version: VERSION.as_ptr(),
    description: c"FM multiplex (stereo, RDS) from the track's audio, sent over IP to RadioMoon MPX Receivers".as_ptr(),
    features: &FEATURES.0 as *const *const c_char,
};

unsafe extern "C" fn f_count(_f: *const clap_plugin_factory) -> u32 {
    1
}

unsafe extern "C" fn f_descriptor(_f: *const clap_plugin_factory, index: u32) -> *const clap_plugin_descriptor {
    if index == 0 {
        &DESCRIPTOR
    } else {
        std::ptr::null()
    }
}

unsafe extern "C" fn f_create(_f: *const clap_plugin_factory, host: *const clap_host, id: *const c_char) -> *const clap_plugin {
    if id.is_null() || CStr::from_ptr(id) != PLUGIN_ID {
        return std::ptr::null();
    }
    gui::set_gui_of(gui_of);
    let plugin_dir = PLUGIN_DIR.lock().ok().and_then(|g| g.clone());
    let settings = settings_path(plugin_dir.as_deref());
    let settings_dir = settings.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    let me = Box::new(Plugin {
        raw: clap_plugin {
            desc: &DESCRIPTOR,
            plugin_data: std::ptr::null_mut(),
            init: Some(p_init),
            destroy: Some(p_destroy),
            activate: Some(p_activate),
            deactivate: Some(p_deactivate),
            start_processing: Some(p_start),
            stop_processing: Some(p_stop),
            reset: Some(p_reset),
            process: Some(p_process),
            get_extension: Some(p_get_extension),
            on_main_thread: Some(p_on_main_thread),
        },
        ring: Arc::new(Mutex::new(VecDeque::new())),
        engine: None,
        gui: gui::Gui::new(host, settings_dir, plugin_dir),
        settings,
    });
    let ptr = Box::into_raw(me);
    (*ptr).raw.plugin_data = ptr as *mut c_void;
    &(*ptr).raw
}

static FACTORY: clap_plugin_factory = clap_plugin_factory { get_plugin_count: Some(f_count), get_plugin_descriptor: Some(f_descriptor), create_plugin: Some(f_create) };

unsafe extern "C" fn e_init(path: *const c_char) -> bool {
    if !path.is_null() {
        let p = PathBuf::from(CStr::from_ptr(path).to_string_lossy().to_string());
        // the folder of the plug-in file (or of the bundle on macOS)
        let dir = if p.is_dir() { p.parent().map(|d| d.to_path_buf()) } else { p.parent().map(|d| d.to_path_buf()) };
        if let Ok(mut g) = PLUGIN_DIR.lock() {
            *g = dir;
        }
    }
    true
}

unsafe extern "C" fn e_deinit() {}

unsafe extern "C" fn e_get_factory(id: *const c_char) -> *const c_void {
    if !id.is_null() && CStr::from_ptr(id) == CLAP_PLUGIN_FACTORY_ID {
        return &FACTORY as *const _ as *const c_void;
    }
    std::ptr::null()
}

/// The entry point a CLAP host looks for.
#[no_mangle]
#[allow(non_upper_case_globals)]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry { clap_version: CLAP_VERSION, init: Some(e_init), deinit: Some(e_deinit), get_factory: Some(e_get_factory) };
