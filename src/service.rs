//! Running a program as a background service that starts with the computer:
//! Windows: a Windows service (LocalSystem, automatic start, restarted on failure).
//! Linux:   a systemd user service (starts at boot with "linger", restarted on failure).
//! The service runs the same program with `--service`: no window, settings from the settings file, live status
//! written to the status file (see status.rs).
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// Which program a service belongs to.
#[derive(Clone, Copy)]
pub struct Kind {
    /// Windows service name / systemd unit name (without ".service").
    pub id: &'static str,
    pub display: &'static str,
    pub description: &'static str,
}

pub const TRANSMITTER: Kind = Kind {
    id: "radiomoon-mpx-transmitter",
    display: "RadioMoon MPX Transmitter",
    description: "FM multiplex (MPX) generator with RDS, sends the MPX over IP and to a 192 kHz sound card.",
};

pub const RECEIVER: Kind = Kind {
    id: "radiomoon-mpx-receiver",
    display: "RadioMoon MPX Receiver",
    description: "Receives the MPX over IP and plays it on a 192 kHz sound card (exciter MPX input).",
};

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct State {
    /// Services work on this system (Windows, Linux with systemd).
    pub supported: bool,
    pub installed: bool,
    pub running: bool,
}

/// The command-line switch that makes the program run as the service.
pub const SWITCH: &str = "--service";

pub fn requested() -> bool {
    std::env::args().any(|a| a == SWITCH)
}

#[allow(dead_code)]
fn exe() -> String {
    std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()
}

// ================================================================ Windows

#[cfg(windows)]
mod imp {
    use super::{exe, Kind, State, SWITCH};
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const NO_WINDOW: u32 = 0x0800_0000;

    fn sc(args: &[&str]) -> Option<(bool, String)> {
        let out = Command::new("sc.exe").args(args).creation_flags(NO_WINDOW).output().ok()?;
        Some((out.status.success(), String::from_utf8_lossy(&out.stdout).to_string()))
    }

    pub fn query(k: &Kind) -> State {
        match sc(&["query", k.id]) {
            Some((true, out)) => State { supported: true, installed: true, running: out.contains("RUNNING") || out.contains("START_PENDING") },
            _ => State { supported: true, installed: false, running: false },
        }
    }

    /// Runs the commands in a batch file with administrator rights (one UAC prompt) and waits for it.
    fn elevated(lines: &[String]) -> Result<(), String> {
        let dir = std::env::temp_dir();
        let bat = dir.join(format!("radiomoon-mpx-service-{}.cmd", std::process::id()));
        let mut text = String::from("@echo off\r\n");
        for l in lines {
            text.push_str(l);
            text.push_str("\r\n");
        }
        std::fs::write(&bat, text).map_err(|e| e.to_string())?;
        let path = bat.to_string_lossy().replace('\'', "''");
        let ps = format!("Start-Process -FilePath '{path}' -Verb RunAs -Wait -WindowStyle Hidden");
        let res = Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &ps]).creation_flags(NO_WINDOW).status();
        let _ = std::fs::remove_file(&bat);
        match res {
            Ok(s) if s.success() => Ok(()),
            Ok(_) => Err("The administrator permission was not given".into()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn install(k: &Kind) -> Result<(), String> {
        let exe = exe();
        // interactive users (IU) may start and stop the service without a UAC prompt
        let sddl = "D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;CCLCSWRPWPLOCRRC;;;IU)(A;;CCLCSWLOCRRC;;;SU)";
        elevated(&[
            format!("sc.exe create {} binPath= \"\\\"{exe}\\\" {SWITCH}\" start= auto DisplayName= \"{}\"", k.id, k.display),
            format!("sc.exe description {} \"{}\"", k.id, k.description),
            format!("sc.exe failure {} reset= 86400 actions= restart/5000/restart/5000/restart/10000", k.id),
            format!("sc.exe sdset {} {sddl}", k.id),
            format!("sc.exe start {}", k.id),
        ])?;
        if query(k).installed {
            Ok(())
        } else {
            Err("The service was not installed".into())
        }
    }

    pub fn uninstall(k: &Kind) -> Result<(), String> {
        elevated(&[format!("sc.exe stop {}", k.id), "timeout /t 2 /nobreak >nul".into(), format!("sc.exe delete {}", k.id)])?;
        if query(k).installed {
            Err("The service is still installed (it is removed when it has stopped)".into())
        } else {
            Ok(())
        }
    }

    fn control(k: &Kind, verb: &str) -> Result<(), String> {
        match sc(&[verb, k.id]) {
            Some((true, _)) => Ok(()),
            // no right to control it (older installation): ask for administrator rights
            _ => elevated(&[format!("sc.exe {verb} {}", k.id)]),
        }
    }

    pub fn start(k: &Kind) -> Result<(), String> {
        control(k, "start")
    }

    pub fn stop(k: &Kind) -> Result<(), String> {
        control(k, "stop")
    }

    // ---------------------------------------------------------------- the service itself
    use std::ffi::OsString;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;
    use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::{define_windows_service, service_dispatcher};

    static RUN: OnceLock<(&'static str, fn(Arc<AtomicBool>))> = OnceLock::new();

    define_windows_service!(ffi_service_main, service_main);

    fn status(state: ServiceState, accept: ServiceControlAccept) -> ServiceStatus {
        ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::from_secs(5),
            process_id: None,
        }
    }

    fn service_main(_args: Vec<OsString>) {
        let Some((name, run)) = RUN.get().copied() else { return };
        let stop = Arc::new(AtomicBool::new(false));
        let st = stop.clone();
        let handler = move |c| match c {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                st.store(true, Ordering::Relaxed);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let Ok(h) = service_control_handler::register(name, handler) else { return };
        let _ = h.set_service_status(status(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN));
        run(stop);
        let _ = h.set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty()));
    }

    /// Hands the program to the Windows service manager. Returns false when the program was not started by it.
    pub fn dispatch(k: &Kind, run: fn(Arc<AtomicBool>)) -> bool {
        let _ = RUN.set((k.id, run));
        service_dispatcher::start(k.id, ffi_service_main).is_ok()
    }
}

// ================================================================ Linux (systemd user service)

#[cfg(target_os = "linux")]
mod imp {
    use super::{exe, Kind, State, SWITCH};
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn unit_path(k: &Kind) -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("systemd").join("user").join(format!("{}.service", k.id)))
    }

    fn systemctl(args: &[&str]) -> Option<(bool, String)> {
        let out = Command::new("systemctl").arg("--user").args(args).output().ok()?;
        Some((out.status.success(), String::from_utf8_lossy(&out.stdout).trim().to_string()))
    }

    pub fn query(k: &Kind) -> State {
        let supported = Command::new("systemctl").arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
        let installed = unit_path(k).map(|p| p.exists()).unwrap_or(false);
        let running = installed && systemctl(&["is-active", k.id]).map(|(_, s)| s == "active" || s == "activating").unwrap_or(false);
        State { supported, installed, running }
    }

    pub fn install(k: &Kind) -> Result<(), String> {
        let path = unit_path(k).ok_or("No home folder")?;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let unit = format!(
            "[Unit]\nDescription={}\nAfter=network-online.target sound.target\nWants=network-online.target\n\n[Service]\nExecStart=\"{}\" {SWITCH}\nRestart=always\nRestartSec=3\n\n[Install]\nWantedBy=default.target\n",
            k.display,
            exe()
        );
        std::fs::write(&path, unit).map_err(|e| e.to_string())?;
        systemctl(&["daemon-reload"]);
        match systemctl(&["enable", "--now", k.id]) {
            Some((true, _)) => {}
            _ => return Err("systemctl could not enable the service".into()),
        }
        // start at boot without a login (may need the permission of the system; it works without it after a login)
        if let Ok(user) = std::env::var("USER") {
            let _ = Command::new("loginctl").args(["enable-linger", &user]).output();
        }
        Ok(())
    }

    pub fn uninstall(k: &Kind) -> Result<(), String> {
        systemctl(&["disable", "--now", k.id]);
        if let Some(p) = unit_path(k) {
            let _ = std::fs::remove_file(p);
        }
        systemctl(&["daemon-reload"]);
        Ok(())
    }

    pub fn start(k: &Kind) -> Result<(), String> {
        match systemctl(&["start", k.id]) {
            Some((true, _)) => Ok(()),
            _ => Err("systemctl could not start the service".into()),
        }
    }

    pub fn stop(k: &Kind) -> Result<(), String> {
        match systemctl(&["stop", k.id]) {
            Some((true, _)) => Ok(()),
            _ => Err("systemctl could not stop the service".into()),
        }
    }

    /// systemd starts the program directly; nothing to hand over.
    pub fn dispatch(_k: &Kind, _run: fn(Arc<AtomicBool>)) -> bool {
        false
    }
}

// ================================================================ other systems

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    use super::{Kind, State};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    pub fn query(_k: &Kind) -> State {
        State::default()
    }
    pub fn install(_k: &Kind) -> Result<(), String> {
        Err("Services are not supported on this system".into())
    }
    pub fn uninstall(_k: &Kind) -> Result<(), String> {
        Ok(())
    }
    pub fn start(_k: &Kind) -> Result<(), String> {
        Err("Services are not supported on this system".into())
    }
    pub fn stop(_k: &Kind) -> Result<(), String> {
        Ok(())
    }
    pub fn dispatch(_k: &Kind, _run: fn(Arc<AtomicBool>)) -> bool {
        false
    }
}

pub use imp::{install, query, start, stop, uninstall};

/// Entry point for `--service`: on Windows the program is handed to the service manager; when that is not possible
/// (Linux, or started by hand) the service work runs right here until the process ends.
pub fn run(k: &Kind, work: fn(Arc<AtomicBool>)) {
    if imp::dispatch(k, work) {
        return;
    }
    work(Arc::new(AtomicBool::new(false)));
}
