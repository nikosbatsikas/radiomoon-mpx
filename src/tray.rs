//! System tray icon (Windows and Linux desktops): shows whether the program (or its service) is running, with a
//! menu to show the window and to exit. It runs on its own thread, so it keeps working while the window is hidden.
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// What the tray shows; the window updates it.
#[derive(Clone, Default, PartialEq)]
pub struct TrayState {
    pub running: bool,
    pub tooltip: String,
}

/// Shared between the window and the tray thread.
pub struct Tray {
    pub state: Arc<Mutex<TrayState>>,
    /// Set by the tray menu "Exit"; the window closes the program.
    pub exit: Arc<AtomicBool>,
    /// The tray icon was created (a tray is available on this desktop).
    pub ok: Arc<AtomicBool>,
    /// Native window handle (Windows), used to show a hidden window again.
    pub hwnd: isize,
}

impl Tray {
    pub fn set(&self, running: bool, tooltip: &str) {
        if let Ok(mut s) = self.state.lock() {
            if s.running != running || s.tooltip != tooltip {
                s.running = running;
                s.tooltip = tooltip.to_string();
            }
        }
    }
    pub fn available(&self) -> bool {
        self.ok.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------- showing and hiding the window

#[cfg(windows)]
mod win {
    #[repr(C)]
    pub struct Msg {
        hwnd: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt: [i32; 2],
        private: u32,
    }
    #[link(name = "user32")]
    extern "system" {
        pub fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
        pub fn SetForegroundWindow(hwnd: isize) -> i32;
        pub fn IsIconic(hwnd: isize) -> i32;
        pub fn PeekMessageW(msg: *mut Msg, hwnd: isize, min: u32, max: u32, remove: u32) -> i32;
        pub fn TranslateMessage(msg: *const Msg) -> i32;
        pub fn DispatchMessageW(msg: *const Msg) -> isize;
    }
    pub const SW_HIDE: i32 = 0;
    pub const SW_SHOW: i32 = 5;
    pub const SW_RESTORE: i32 = 9;
    pub const PM_REMOVE: u32 = 1;

    /// Handles the window messages of this thread (the tray icon's hidden window).
    pub fn pump() {
        unsafe {
            let mut m: Msg = std::mem::zeroed();
            while PeekMessageW(&mut m, 0, 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
    }
}

/// Hides the window (it stays in the tray).
pub fn hide_window(ctx: &egui::Context, hwnd: isize) {
    #[cfg(windows)]
    if hwnd != 0 {
        unsafe {
            win::ShowWindow(hwnd, win::SW_HIDE);
        }
        return;
    }
    let _ = hwnd;
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
}

/// Brings the window back to the front.
pub fn show_window(ctx: &egui::Context, hwnd: isize) {
    #[cfg(windows)]
    if hwnd != 0 {
        unsafe {
            if win::IsIconic(hwnd) != 0 {
                win::ShowWindow(hwnd, win::SW_RESTORE);
            } else {
                win::ShowWindow(hwnd, win::SW_SHOW);
            }
            win::SetForegroundWindow(hwnd);
        }
    }
    let _ = hwnd;
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    ctx.request_repaint();
}

// ---------------------------------------------------------------- the tray thread

#[cfg(any(windows, target_os = "linux"))]
fn icons(png: &[u8]) -> Option<(tray_icon::Icon, tray_icon::Icon)> {
    let img = image::load_from_memory(png).ok()?.resize(64, 64, image::imageops::FilterType::Lanczos3).to_rgba8();
    let (w, h) = (img.width(), img.height());
    let on = img.as_raw().clone();
    // stopped: grey and dimmed
    let mut off = on.clone();
    for px in off.chunks_exact_mut(4) {
        let l = (px[0] as u32 * 30 + px[1] as u32 * 59 + px[2] as u32 * 11) / 100;
        let l = (l * 7 / 10) as u8;
        px[0] = l;
        px[1] = l;
        px[2] = l;
    }
    Some((tray_icon::Icon::from_rgba(on, w, h).ok()?, tray_icon::Icon::from_rgba(off, w, h).ok()?))
}

/// Starts the tray icon. `title` is the program name, `png` its icon.
pub fn start(title: &'static str, png: &'static [u8], hwnd: isize, ctx: egui::Context) -> Tray {
    let tray = Tray { state: Arc::new(Mutex::new(TrayState::default())), exit: Arc::new(AtomicBool::new(false)), ok: Arc::new(AtomicBool::new(false)), hwnd };
    #[cfg(any(windows, target_os = "linux"))]
    {
        let (state, exit, ok) = (tray.state.clone(), tray.exit.clone(), tray.ok.clone());
        std::thread::spawn(move || run(title, png, hwnd, ctx, state, exit, ok));
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (title, png, ctx);
    }
    tray
}

#[cfg(any(windows, target_os = "linux"))]
fn run(title: &'static str, png: &'static [u8], hwnd: isize, ctx: egui::Context, state: Arc<Mutex<TrayState>>, exit: Arc<AtomicBool>, ok: Arc<AtomicBool>) {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    let Some((icon_on, icon_off)) = icons(png) else { return };
    let menu = Menu::new();
    let show = MenuItem::new(format!("Show {title}"), true, None);
    let quit = MenuItem::new("Exit", true, None);
    if menu.append(&show).is_err() || menu.append(&PredefinedMenuItem::separator()).is_err() || menu.append(&quit).is_err() {
        return;
    }
    let tray = match TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip(title).with_icon(icon_off.clone()).build() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("No system tray: {e}");
            return;
        }
    };
    ok.store(true, Ordering::Relaxed);
    let mut shown = TrayState::default();
    loop {
        #[cfg(windows)]
        win::pump();
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            if ev.id == *show.id() {
                show_window(&ctx, hwnd);
            } else if ev.id == *quit.id() {
                exit.store(true, Ordering::Relaxed);
                show_window(&ctx, hwnd);
            }
        }
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            match ev {
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => show_window(&ctx, hwnd),
                _ => {}
            }
        }
        if exit.load(Ordering::Relaxed) {
            // the window closes the program; if it cannot (hidden and not repainting), end it here
            std::thread::sleep(std::time::Duration::from_secs(3));
            std::process::exit(0);
        }
        let now = state.lock().map(|g| g.clone()).unwrap_or_default();
        if now != shown {
            if now.running != shown.running {
                let _ = tray.set_icon(Some(if now.running { icon_on.clone() } else { icon_off.clone() }));
            }
            let tip = if now.tooltip.is_empty() { title.to_string() } else { format!("{title}\n{}", now.tooltip) };
            let _ = tray.set_tooltip(Some(tip));
            shown = now;
        }
        std::thread::sleep(std::time::Duration::from_millis(60));
    }
}
