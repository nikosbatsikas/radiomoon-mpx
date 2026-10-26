//! The plug-in's window (CLAP "gui" extension, floating): the RadioMoon MPX Transmitter program opened in its plug-in
//! mode (`--plugin`), on the settings file this plug-in uses. It has the same tabs as the program (IP outputs, sound
//! card, signal, RDS, SFN, the level of the track's audio) and the live status of the plug-in. Running the window as
//! its own program keeps the audio program safe (no second GUI toolkit inside the host) and works the same on
//! Windows, macOS and Linux.
//!
//! On a computer with only the headless Transmitter (no window program) the plug-in window is its web page, opened in
//! the browser.
use clap_sys::ext::gui::{clap_gui_resize_hints, clap_host_gui, clap_plugin_gui, clap_window, CLAP_EXT_GUI};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use std::ffi::{c_char, c_void, CStr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The platform window API the host is told about (the window is floating, so it is only a name for the host).
#[cfg(windows)]
const API: &CStr = c"win32";
#[cfg(target_os = "macos")]
const API: &CStr = c"cocoa";
#[cfg(all(unix, not(target_os = "macos")))]
const API: &CStr = c"x11";

/// What opens the window.
#[derive(Clone, Debug, PartialEq)]
pub enum Opener {
    /// The Transmitter window program.
    Program(PathBuf),
    /// The headless Transmitter's web page (browser).
    Web(String),
}

/// The window of one plug-in instance.
pub struct Gui {
    host: *const clap_host,
    settings_dir: PathBuf,
    plugin_dir: Option<PathBuf>,
    title: String,
    child: Arc<Mutex<Option<Child>>>,
    /// Set by the watcher when the user closed the window; the host is told on its main thread.
    closed: Arc<AtomicBool>,
    created: bool,
}

unsafe impl Send for Gui {}

impl Gui {
    pub fn new(host: *const clap_host, settings_dir: PathBuf, plugin_dir: Option<PathBuf>) -> Gui {
        Gui { host, settings_dir, plugin_dir, title: String::new(), child: Arc::new(Mutex::new(None)), closed: Arc::new(AtomicBool::new(false)), created: false }
    }

    fn host_name(&self) -> String {
        unsafe {
            if self.host.is_null() || (*self.host).name.is_null() {
                return String::new();
            }
            CStr::from_ptr((*self.host).name).to_string_lossy().to_string()
        }
    }

    fn running(&self) -> bool {
        match self.child.lock() {
            Ok(mut g) => match g.as_mut() {
                Some(c) => matches!(c.try_wait(), Ok(None)),
                None => false,
            },
            Err(_) => false,
        }
    }

    pub fn show(&mut self) -> bool {
        if self.running() {
            return true;
        }
        let Some(op) = opener(self.plugin_dir.as_deref()) else {
            return false;
        };
        match op {
            Opener::Program(exe) => {
                let mut cmd = Command::new(&exe);
                cmd.arg("--plugin").arg(if self.title.is_empty() { self.host_name() } else { self.title.clone() });
                // the window edits exactly the settings file of this plug-in
                cmd.env("RADIOMOON_DATA", &self.settings_dir);
                cmd.env_remove("RM_TAB");
                let Ok(child) = cmd.spawn() else {
                    return false;
                };
                if let Ok(mut g) = self.child.lock() {
                    *g = Some(child);
                }
                self.watch();
                true
            }
            Opener::Web(url) => open_url(&url),
        }
    }

    /// Tells the host (on its main thread) when the user closes the window.
    fn watch(&self) {
        let (child, closed, host) = (self.child.clone(), self.closed.clone(), self.host as usize);
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(300));
            let done = match child.lock() {
                Ok(mut g) => match g.as_mut() {
                    Some(c) => !matches!(c.try_wait(), Ok(None)),
                    None => return, // hidden by the host
                },
                Err(_) => return,
            };
            if done {
                if let Ok(mut g) = child.lock() {
                    *g = None;
                }
                closed.store(true, Ordering::Relaxed);
                let h = host as *const clap_host;
                unsafe {
                    if !h.is_null() {
                        if let Some(rc) = (*h).request_callback {
                            rc(h);
                        }
                    }
                }
                return;
            }
        });
    }

    pub fn hide(&mut self) -> bool {
        if let Ok(mut g) = self.child.lock() {
            if let Some(mut c) = g.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        true
    }

    /// On the host's main thread (`on_main_thread`): report a window the user closed.
    pub fn main_thread(&self) {
        if self.closed.swap(false, Ordering::Relaxed) && !self.host.is_null() {
            unsafe {
                if let Some(ge) = (*self.host).get_extension {
                    let g = ge(self.host, CLAP_EXT_GUI.as_ptr()) as *const clap_host_gui;
                    if !g.is_null() {
                        if let Some(closed) = (*g).closed {
                            closed(self.host, false);
                        }
                    }
                }
            }
        }
    }
}

impl Drop for Gui {
    fn drop(&mut self) {
        self.hide();
    }
}

/// The Transmitter window program next to the plug-in or where the packages install it, else the headless
/// Transmitter's web page. `RADIOMOON_MPX_TRANSMITTER` names the program file.
pub fn opener(plugin_dir: Option<&Path>) -> Option<Opener> {
    if let Some(p) = std::env::var_os("RADIOMOON_MPX_TRANSMITTER").map(PathBuf::from) {
        if p.is_file() {
            return Some(Opener::Program(p));
        }
    }
    for p in program_candidates(plugin_dir) {
        if p.is_file() {
            return Some(Opener::Program(p));
        }
    }
    web_page().map(Opener::Web)
}

fn program_candidates(plugin_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = plugin_dir {
        dirs.push(d.to_path_buf());
        if let Some(p) = d.parent() {
            dirs.push(p.to_path_buf());
        }
    }
    let mut v: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(pf) = std::env::var_os(var) {
                dirs.push(PathBuf::from(pf).join("RadioMoon MPX"));
            }
        }
        v.extend(dirs.iter().map(|d| d.join("RadioMoon-MPX-Transmitter.exe")));
    } else if cfg!(target_os = "macos") {
        let app = "RadioMoon MPX Transmitter.app/Contents/MacOS/RadioMoon-MPX-Transmitter";
        v.extend(dirs.iter().map(|d| d.join(app)));
        v.push(PathBuf::from("/Applications").join(app));
        if let Some(h) = std::env::var_os("HOME") {
            v.push(PathBuf::from(h).join("Applications").join(app));
        }
    } else {
        if let Some(h) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(h).join(".local/bin"));
        }
        for d in ["/usr/local/bin", "/usr/bin", "/opt/radiomoon-mpx"] {
            dirs.push(PathBuf::from(d));
        }
        if let Some(path) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&path));
        }
        for d in &dirs {
            v.push(d.join("radiomoon-mpx-transmitter"));
            v.push(d.join("RadioMoon-MPX-Transmitter").join("radiomoon-mpx-transmitter"));
        }
    }
    v
}

/// The web page of the headless Transmitter on this computer (Linux), when a browser can be opened here.
fn web_page() -> Option<String> {
    if !cfg!(all(unix, not(target_os = "macos"))) {
        return None;
    }
    if !Path::new("/var/lib/radiomoon-mpx/web-login.json").exists() {
        return None;
    }
    let desktop = std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();
    if !desktop || !in_path("xdg-open") {
        return None;
    }
    // the port: RM_LISTEN of the OpenRC service, --listen of the systemd unit, else the default
    let mut port = "8080".to_string();
    for f in ["/etc/conf.d/radiomoon-mpx-txd", "/etc/systemd/system/radiomoon-mpx-txd.service"] {
        if let Ok(t) = std::fs::read_to_string(f) {
            if let Some(p) = listen_port(&t) {
                port = p;
            }
        }
    }
    Some(format!("http://127.0.0.1:{port}/"))
}

/// The port of `RM_LISTEN="0.0.0.0:8080"` or `--listen 0.0.0.0:8080`.
pub fn listen_port(text: &str) -> Option<String> {
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('#') {
            continue;
        }
        let rest = if let Some(i) = l.find("RM_LISTEN=") {
            &l[i + 10..]
        } else if let Some(i) = l.find("--listen ") {
            &l[i + 9..]
        } else {
            continue;
        };
        let addr: String = rest.trim_matches('"').split_whitespace().next().unwrap_or("").trim_matches('"').to_string();
        if let Some(p) = addr.rsplit(':').next() {
            if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) {
                return Some(p.to_string());
            }
        }
    }
    None
}

fn in_path(name: &str) -> bool {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d.join(name).is_file())).unwrap_or(false)
}

fn open_url(url: &str) -> bool {
    Command::new("xdg-open").arg(url).spawn().is_ok()
}

// ---------------------------------------------------------------- the CLAP functions

/// The window of the plug-in behind `p` (set up by lib.rs).
pub type GuiOf = unsafe fn(*const clap_plugin) -> Option<&'static mut Gui>;

static GUI_OF: Mutex<Option<GuiOf>> = Mutex::new(None);

pub fn set_gui_of(f: GuiOf) {
    if let Ok(mut g) = GUI_OF.lock() {
        *g = Some(f);
    }
}

unsafe fn gui<'a>(p: *const clap_plugin) -> Option<&'a mut Gui> {
    let f = GUI_OF.lock().ok().and_then(|g| *g)?;
    f(p)
}

unsafe extern "C" fn is_api_supported(p: *const clap_plugin, _api: *const c_char, is_floating: bool) -> bool {
    // floating only (its own window); any API name, the host only needs a floating window
    is_floating && gui(p).map(|g| opener(g.plugin_dir.as_deref()).is_some()).unwrap_or(false)
}

unsafe extern "C" fn get_preferred_api(_p: *const clap_plugin, api: *mut *const c_char, is_floating: *mut bool) -> bool {
    if !api.is_null() {
        *api = API.as_ptr();
    }
    if !is_floating.is_null() {
        *is_floating = true;
    }
    true
}

unsafe extern "C" fn create(p: *const clap_plugin, _api: *const c_char, is_floating: bool) -> bool {
    if !is_floating {
        return false;
    }
    match gui(p) {
        Some(g) => {
            g.created = true;
            true
        }
        None => false,
    }
}

unsafe extern "C" fn destroy(p: *const clap_plugin) {
    if let Some(g) = gui(p) {
        g.hide();
        g.created = false;
    }
}

unsafe extern "C" fn set_scale(_p: *const clap_plugin, _scale: f64) -> bool {
    false
}

unsafe extern "C" fn get_size(_p: *const clap_plugin, _w: *mut u32, _h: *mut u32) -> bool {
    false
}

unsafe extern "C" fn can_resize(_p: *const clap_plugin) -> bool {
    false
}

unsafe extern "C" fn get_resize_hints(_p: *const clap_plugin, _h: *mut clap_gui_resize_hints) -> bool {
    false
}

unsafe extern "C" fn adjust_size(_p: *const clap_plugin, _w: *mut u32, _h: *mut u32) -> bool {
    false
}

unsafe extern "C" fn set_size(_p: *const clap_plugin, _w: u32, _h: u32) -> bool {
    false
}

unsafe extern "C" fn set_parent(_p: *const clap_plugin, _w: *const clap_window) -> bool {
    false
}

unsafe extern "C" fn set_transient(_p: *const clap_plugin, _w: *const clap_window) -> bool {
    true
}

unsafe extern "C" fn suggest_title(p: *const clap_plugin, title: *const c_char) {
    if let (Some(g), false) = (gui(p), title.is_null()) {
        g.title = CStr::from_ptr(title).to_string_lossy().to_string();
    }
}

unsafe extern "C" fn show(p: *const clap_plugin) -> bool {
    gui(p).map(|g| g.show()).unwrap_or(false)
}

unsafe extern "C" fn hide(p: *const clap_plugin) -> bool {
    gui(p).map(|g| g.hide()).unwrap_or(false)
}

pub static GUI: clap_plugin_gui = clap_plugin_gui {
    is_api_supported: Some(is_api_supported),
    get_preferred_api: Some(get_preferred_api),
    create: Some(create),
    destroy: Some(destroy),
    set_scale: Some(set_scale),
    get_size: Some(get_size),
    can_resize: Some(can_resize),
    get_resize_hints: Some(get_resize_hints),
    adjust_size: Some(adjust_size),
    set_size: Some(set_size),
    set_parent: Some(set_parent),
    set_transient: Some(set_transient),
    suggest_title: Some(suggest_title),
    show: Some(show),
    hide: Some(hide),
};

pub fn ext_ptr() -> *const c_void {
    &GUI as *const _ as *const c_void
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports_of_the_service_files() {
        assert_eq!(listen_port("# x\nRM_LISTEN=\"0.0.0.0:8090\"\n").as_deref(), Some("8090"));
        assert_eq!(listen_port("ExecStart=/usr/local/bin/radiomoon-mpx-txd --data /var/lib/radiomoon-mpx --listen 0.0.0.0:8080\n").as_deref(), Some("8080"));
        assert_eq!(listen_port("nothing").as_deref(), None);
    }

    #[test]
    fn finds_the_program_next_to_the_plugin() {
        let d = std::env::temp_dir().join(format!("rm-gui-{}", std::process::id()));
        let _ = std::fs::create_dir_all(d.join("CLAP"));
        let exe = if cfg!(windows) { "RadioMoon-MPX-Transmitter.exe" } else if cfg!(target_os = "macos") { "x" } else { "radiomoon-mpx-transmitter" };
        if !cfg!(target_os = "macos") {
            std::fs::write(d.join(exe), b"").unwrap();
            // the portable zip: the plug-in in CLAP/, the program one folder up
            assert_eq!(opener(Some(&d.join("CLAP"))), Some(Opener::Program(d.join(exe))));
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
