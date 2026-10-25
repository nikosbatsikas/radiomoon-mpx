//! A small web server for the headless editions (Linux, Raspberry Pi, Alpine): the settings and the live status of
//! the Transmitter or the Receiver in a browser, with a login. No external crates: HTTP/1.1 on std's TcpListener.
//!
//! - `GET /` the page; `GET /api/status`, `/api/settings`, `/api/devices`, `/api/me` (JSON)
//! - `POST /api/login` {user, password}, `POST /api/logout`, `POST /api/settings` (the whole settings object),
//!   `POST /api/account` {old, user, password}
//!
//! The login: one user, the password kept as PBKDF2-HMAC-SHA256 (100 000 rounds, random salt) in `web-login.json`.
//! Sessions are random 128-bit tokens in an HttpOnly, SameSite=Strict cookie; every POST must also carry the header
//! `X-RadioMoon: 1` (a page of another site cannot send it), against cross-site requests. Wrong passwords are slowed
//! down and an address is locked out for 5 minutes after 10 failures.
use crate::crypt::{pbkdf2_sha256, random_bytes};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What the server serves: implemented by the headless Transmitter and Receiver.
pub trait App: Send + Sync + 'static {
    /// The page (HTML with its script).
    fn page(&self) -> &'static str;
    fn status_json(&self) -> String;
    fn settings_json(&self) -> String;
    /// Saves new settings (the running program applies them); returns an error text for the page.
    fn save_settings(&self, json: &str) -> Result<(), String>;
    /// Sound devices, serial ports ... for the choice lists of the page.
    fn devices_json(&self) -> String;
    /// The program icon (PNG), for the page header, the About box and the browser tab.
    fn icon(&self) -> &'static [u8];
}

/// When the program started (for the About box).
fn started() -> &'static (Instant, u64) {
    static S: std::sync::OnceLock<(Instant, u64)> = std::sync::OnceLock::new();
    S.get_or_init(|| (Instant::now(), crate::status::now_ms()))
}

fn host_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_default()
}

/// The operating system, e.g. "Alpine Linux v3.20" (from /etc/os-release), else the platform name.
fn os_name() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())))
        .unwrap_or_else(|| std::env::consts::OS.to_string())
}

// ---------------------------------------------------------------- login

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Login {
    pub user: String,
    pub salt: String,
    pub hash: String,
    pub rounds: u32,
}

const ROUNDS: u32 = 100_000;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl Login {
    pub fn new(user: &str, password: &str) -> Login {
        let salt = hex(&random_bytes()[..16]);
        let hash = hex(&pbkdf2_sha256(password.as_bytes(), salt.as_bytes(), ROUNDS));
        Login { user: user.to_string(), salt, hash, rounds: ROUNDS }
    }
    pub fn check(&self, user: &str, password: &str) -> bool {
        let h = hex(&pbkdf2_sha256(password.as_bytes(), self.salt.as_bytes(), self.rounds.max(1)));
        // constant time comparison
        let same = h.len() == self.hash.len() && h.bytes().zip(self.hash.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0;
        same && user == self.user
    }
}

pub fn login_path() -> PathBuf {
    crate::status::shared_dir().join("web-login.json")
}

pub fn load_login() -> Option<Login> {
    std::fs::read_to_string(login_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).filter(|l: &Login| !l.hash.is_empty())
}

pub fn save_login(l: &Login) -> std::io::Result<()> {
    let p = login_path();
    std::fs::write(&p, serde_json::to_string_pretty(l).map_err(std::io::Error::other)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// A readable random password (for the first start without a login).
pub fn random_password() -> String {
    const A: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    random_bytes().iter().take(12).map(|b| A[*b as usize % A.len()] as char).collect()
}

// ---------------------------------------------------------------- server

struct State {
    login: Mutex<Option<Login>>,
    sessions: Mutex<HashMap<String, Instant>>,
    fails: Mutex<HashMap<IpAddr, (u32, Instant)>>,
}

const SESSION: Duration = Duration::from_secs(12 * 3600);

/// Starts the server on `listen` (e.g. "0.0.0.0:8080"). Returns an error text when the port cannot be opened.
pub fn start(listen: &str, app: Arc<dyn App>, stop: Arc<AtomicBool>) -> Result<(), String> {
    let l = TcpListener::bind(listen).map_err(|e| format!("web interface: cannot listen on {listen}: {e}"))?;
    let _ = l.set_nonblocking(true);
    let _ = started();
    let st = Arc::new(State { login: Mutex::new(load_login()), sessions: Mutex::new(HashMap::new()), fails: Mutex::new(HashMap::new()) });
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match l.accept() {
                Ok((s, a)) => {
                    let (app, st) = (app.clone(), st.clone());
                    std::thread::spawn(move || {
                        let _ = s.set_nonblocking(false);
                        let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
                        let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
                        let _ = handle(s, a.ip(), &*app, &st);
                    });
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    });
    Ok(())
}

struct Req {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn read_req(s: &mut TcpStream) -> Option<Req> {
    let mut r = BufReader::new(s);
    let mut line = String::new();
    r.read_line(&mut line).ok()?;
    let mut it = line.split_whitespace();
    let method = it.next()?.to_string();
    let path = it.next()?.to_string();
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).ok()? == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
        if headers.len() > 64 {
            return None;
        }
    }
    let n: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    if n > 1 << 20 {
        return None;
    }
    let mut body = vec![0u8; n];
    r.read_exact(&mut body).ok()?;
    Some(Req { method, path, headers, body })
}

fn respond(s: &mut TcpStream, code: u16, ctype: &str, body: &[u8], extra: &str) -> std::io::Result<()> {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n{}X-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'self' data:\r\nConnection: close\r\n{extra}\r\n",
        body.len(),
        if extra.contains("Cache-Control") { "" } else { "Cache-Control: no-store\r\n" }
    );
    s.write_all(head.as_bytes())?;
    s.write_all(body)?;
    s.flush()
}

fn json(s: &mut TcpStream, code: u16, body: &str) -> std::io::Result<()> {
    respond(s, code, "application/json; charset=utf-8", body.as_bytes(), "")
}

fn cookie(req: &Req, name: &str) -> Option<String> {
    req.headers.get("cookie")?.split(';').find_map(|c| {
        let (k, v) = c.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

fn logged_in(req: &Req, st: &State) -> bool {
    let Some(tok) = cookie(req, "rmsession") else { return false };
    let mut s = match st.sessions.lock() {
        Ok(s) => s,
        Err(_) => return false,
    };
    s.retain(|_, t| t.elapsed() < SESSION);
    s.contains_key(&tok)
}

fn handle(mut s: TcpStream, ip: IpAddr, app: &dyn App, st: &State) -> std::io::Result<()> {
    let Some(req) = read_req(&mut s) else { return respond(&mut s, 400, "text/plain", b"bad request", "") };
    let path = req.path.split('?').next().unwrap_or("/").to_string();
    // against requests sent by other sites: a custom header that a cross-site form cannot set
    if req.method == "POST" && req.headers.get("x-radiomoon").map(|v| v.as_str()) != Some("1") {
        return json(&mut s, 403, r#"{"error":"missing request header"}"#);
    }
    match (req.method.as_str(), path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => respond(&mut s, 200, "text/html; charset=utf-8", app.page().as_bytes(), ""),
        ("GET", "/favicon.ico") | ("GET", "/logo.png") => respond(&mut s, 200, "image/png", app.icon(), "Cache-Control: max-age=86400\r\n"),
        ("POST", "/api/login") => {
            // lock-out after 10 failures from one address
            if let Ok(f) = st.fails.lock() {
                if let Some((n, t)) = f.get(&ip) {
                    if *n >= 10 && t.elapsed() < Duration::from_secs(300) {
                        return json(&mut s, 429, r#"{"error":"Too many wrong passwords: wait 5 minutes"}"#);
                    }
                }
            }
            #[derive(Deserialize)]
            struct L {
                user: String,
                password: String,
            }
            let Ok(l) = serde_json::from_slice::<L>(&req.body) else { return json(&mut s, 400, r#"{"error":"bad request"}"#) };
            let ok = st.login.lock().ok().and_then(|g| g.clone()).map(|g| g.check(&l.user, &l.password)).unwrap_or(false);
            if !ok {
                if let Ok(mut f) = st.fails.lock() {
                    let e = f.entry(ip).or_insert((0, Instant::now()));
                    if e.1.elapsed() > Duration::from_secs(300) {
                        *e = (0, Instant::now());
                    }
                    e.0 += 1;
                    e.1 = Instant::now();
                }
                std::thread::sleep(Duration::from_millis(1000));
                return json(&mut s, 401, r#"{"error":"Wrong user or password"}"#);
            }
            if let Ok(mut f) = st.fails.lock() {
                f.remove(&ip);
            }
            let tok = hex(&random_bytes()[..16]);
            if let Ok(mut ss) = st.sessions.lock() {
                ss.insert(tok.clone(), Instant::now());
            }
            let c = format!("Set-Cookie: rmsession={tok}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}\r\n", SESSION.as_secs());
            respond(&mut s, 200, "application/json", br#"{"ok":true}"#, &c)
        }
        ("POST", "/api/logout") => {
            if let (Some(tok), Ok(mut ss)) = (cookie(&req, "rmsession"), st.sessions.lock()) {
                ss.remove(&tok);
            }
            respond(&mut s, 200, "application/json", br#"{"ok":true}"#, "Set-Cookie: rmsession=; Path=/; Max-Age=0\r\n")
        }
        _ if path.starts_with("/api/") && !logged_in(&req, st) => json(&mut s, 401, r#"{"error":"login"}"#),
        ("GET", "/api/me") => {
            let user = st.login.lock().ok().and_then(|g| g.clone()).map(|l| l.user).unwrap_or_default();
            let (t0, since) = started();
            json(
                &mut s,
                200,
                &serde_json::json!({
                    "user": user,
                    "version": env!("CARGO_PKG_VERSION"),
                    "os": os_name(),
                    "arch": std::env::consts::ARCH,
                    "host": host_name(),
                    "data": crate::status::shared_dir().display().to_string(),
                    "uptime_s": t0.elapsed().as_secs(),
                    "started_ms": since,
                })
                .to_string(),
            )
        }
        ("GET", "/api/status") => json(&mut s, 200, &app.status_json()),
        ("GET", "/api/settings") => json(&mut s, 200, &app.settings_json()),
        ("GET", "/api/devices") => json(&mut s, 200, &app.devices_json()),
        ("POST", "/api/settings") => match std::str::from_utf8(&req.body).map_err(|e| e.to_string()).and_then(|b| app.save_settings(b)) {
            Ok(()) => json(&mut s, 200, r#"{"ok":true}"#),
            Err(e) => json(&mut s, 400, &serde_json::json!({ "error": e }).to_string()),
        },
        ("POST", "/api/account") => {
            #[derive(Deserialize)]
            struct A {
                old: String,
                user: String,
                password: String,
            }
            let Ok(a) = serde_json::from_slice::<A>(&req.body) else { return json(&mut s, 400, r#"{"error":"bad request"}"#) };
            let cur = st.login.lock().ok().and_then(|g| g.clone());
            let Some(cur) = cur else { return json(&mut s, 400, r#"{"error":"no login"}"#) };
            if !cur.check(&cur.user, &a.old) {
                std::thread::sleep(Duration::from_millis(1000));
                return json(&mut s, 401, r#"{"error":"The current password is wrong"}"#);
            }
            if a.user.trim().is_empty() || a.password.chars().count() < 8 {
                return json(&mut s, 400, r#"{"error":"A user name and a password of at least 8 characters are needed"}"#);
            }
            let n = Login::new(a.user.trim(), &a.password);
            if let Err(e) = save_login(&n) {
                return json(&mut s, 400, &serde_json::json!({ "error": format!("cannot save: {e}") }).to_string());
            }
            if let Ok(mut g) = st.login.lock() {
                *g = Some(n);
            }
            let _ = std::fs::remove_file(crate::status::shared_dir().join("web-initial-password.txt"));
            // other sessions end; this one stays
            if let (Some(tok), Ok(mut ss)) = (cookie(&req, "rmsession"), st.sessions.lock()) {
                ss.retain(|k, _| *k == tok);
            }
            json(&mut s, 200, r#"{"ok":true}"#)
        }
        _ => respond(&mut s, 404, "text/plain", b"not found", ""),
    }
}

// ---------------------------------------------------------------- command line of the headless editions

/// Options of `radiomoon-mpx-txd` / `radiomoon-mpx-rxd`.
pub struct Cli {
    pub listen: String,
    pub web: bool,
}

/// Reads the command line. Handles `--data DIR`, `--set-login USER` (password from standard input), `--help`,
/// `--version` (those end the program).
pub fn cli(name: &str, default_port: u16) -> Cli {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut c = Cli { listen: format!("0.0.0.0:{default_port}"), web: true };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let val = args.get(i + 1).cloned();
        match a {
            "--data" | "-d" => {
                if let Some(v) = val {
                    std::env::set_var("RADIOMOON_DATA", v);
                    i += 1;
                }
            }
            "--listen" | "-l" => {
                if let Some(v) = val {
                    c.listen = if v.contains(':') { v } else { format!("0.0.0.0:{v}") };
                    i += 1;
                }
            }
            "--no-web" => c.web = false,
            "--set-login" => {
                let user = val.unwrap_or_else(|| "admin".into());
                eprint!("New password for \"{user}\" (at least 8 characters): ");
                let mut pw = String::new();
                let _ = std::io::stdin().read_line(&mut pw);
                let pw = pw.trim_end_matches(['\r', '\n']).to_string();
                if pw.chars().count() < 8 {
                    eprintln!("\nThe password needs at least 8 characters.");
                    std::process::exit(2);
                }
                match save_login(&Login::new(&user, &pw)) {
                    Ok(()) => {
                        eprintln!("\nSaved: {}", login_path().display());
                        std::process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("\nCannot save {}: {e}", login_path().display());
                        std::process::exit(1);
                    }
                }
            }
            "--version" | "-V" => {
                println!("{name} {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!(
                    "{name} {} - RadioMoon MPX, headless edition with a web interface\n\n\
                     Options:\n  --data DIR        folder of the settings, status and login (default: RADIOMOON_DATA or ~/.local/share/RadioMoon)\n  \
                     --listen ADDR     address:port of the web interface (default 0.0.0.0:{default_port})\n  \
                     --no-web          no web interface (settings only from the file)\n  \
                     --set-login USER  set the user and the password of the web interface (password from standard input) and exit\n  \
                     --version         show the version",
                    env!("CARGO_PKG_VERSION")
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown option {other} (see --help)");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    c
}

/// Makes sure there is a login: on the first start a random password is set and printed (and kept in
/// `web-initial-password.txt`, readable by the program's user only, until the password is changed on the page).
pub fn ensure_login() {
    if load_login().is_some() {
        return;
    }
    let pw = random_password();
    if save_login(&Login::new("admin", &pw)).is_ok() {
        let p = crate::status::shared_dir().join("web-initial-password.txt");
        let _ = std::fs::write(&p, format!("user: admin\npassword: {pw}\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
        }
        eprintln!("Web interface login: user \"admin\", password \"{pw}\" (also in {}). Change it on the page.", p.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_check() {
        let l = Login::new("admin", "secret pass");
        assert!(l.check("admin", "secret pass"));
        assert!(!l.check("admin", "secret pasS"));
        assert!(!l.check("root", "secret pass"));
        assert_eq!(random_password().len(), 12);
    }
}
