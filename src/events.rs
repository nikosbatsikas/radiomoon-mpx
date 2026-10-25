//! The Receiver's event log: stream lost / back, lost and restored packets, rejected or blocked senders, backup on
//! and off, SFN lock, sound card. Shown in the window's Log tab and on the web page; the headless Receiver also
//! prints it (system log).
use crate::mpxrx::{format_name, RxView};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Events kept (the oldest go first).
pub const KEEP: usize = 300;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Event {
    /// Unix time, ms.
    pub t: u64,
    /// 0 = information, 1 = warning, 2 = fault.
    pub level: u8,
    pub text: String,
}

pub fn push(log: &mut VecDeque<Event>, e: Event) {
    log.push_back(e);
    while log.len() > KEEP {
        log.pop_front();
    }
}

/// "25.10.2026 07:53:12" of a Unix time in ms, shifted by `offset_s` (the local time offset).
pub fn time_text(t_ms: u64, offset_s: i64) -> String {
    let s = (t_ms / 1000) as i64 + offset_s;
    let days = s.div_euclid(86_400);
    let sod = s.rem_euclid(86_400);
    // civil date from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{d:02}.{m:02}.{y} {:02}:{:02}:{:02}", sod / 3600, sod % 3600 / 60, sod % 60)
}

/// Watches the live numbers of the receiver (every half second) and writes what changed into its log.
#[derive(Default)]
pub struct Tracker {
    started: bool,
    on: bool,
    quiet_ticks: u32,
    lost_at: u64,
    from: String,
    format: String,
    card: String,
    backup: bool,
    sfn_locked: Option<bool>,
    // packet counters at the start of the current summary window
    win_start: u64,
    base_lost: u64,
    base_restored_fec: u64,
    base_unrec: u64,
    base_concealed: u64,
    base_rejected: u64,
    base_plain: u64,
    base_blocked: u64,
    need_password: bool,
}

/// Losses and refusals are summed up over this time, so the log is not flooded.
const WINDOW_MS: u64 = 10_000;

impl Tracker {
    pub fn new() -> Tracker {
        Tracker::default()
    }

    fn ev(out: &mut Vec<Event>, t: u64, level: u8, text: String) {
        out.push(Event { t, level, text });
    }

    /// The events since the last call. `now` = Unix time in ms.
    pub fn observe(&mut self, v: &RxView, now: u64) -> Vec<Event> {
        let mut out = Vec::new();
        let p = &v.protect;
        let lost = v.stats.lost + v.stats.bad;
        if !self.started {
            self.started = true;
            self.win_start = now;
            self.base_lost = lost;
            self.base_concealed = v.stats.concealed;
            self.base_restored_fec = p.restored;
            self.base_unrec = p.lost;
            self.base_rejected = p.rejected;
            self.base_plain = p.plain_rejected;
            self.base_blocked = p.blocked;
            self.card = v.card.clone();
            Self::ev(&mut out, now, 0, "Receiver started".into());
        }
        // stream on / off (off after 1 s without packets)
        if v.receiving {
            self.quiet_ticks = 0;
            let fmt = format_name(&v.stats);
            if !self.on {
                self.on = true;
                let after = if self.lost_at > 0 { format!(" (after {} without stream)", dur(now.saturating_sub(self.lost_at))) } else { String::new() };
                Self::ev(&mut out, now, 0, format!("Stream from {}: {fmt}{after}", v.from));
            } else if fmt != self.format || v.from != self.from {
                Self::ev(&mut out, now, 0, format!("Stream now from {}: {fmt}", v.from));
            }
            self.format = fmt;
            self.from = v.from.clone();
        } else {
            self.quiet_ticks += 1;
            if self.on && self.quiet_ticks >= 2 {
                self.on = false;
                self.lost_at = now;
                Self::ev(&mut out, now, 2, format!("Stream lost: no packets from {}", if self.from.is_empty() { "-" } else { &self.from }));
            }
        }
        // the backup source
        if v.backup.active != self.backup {
            self.backup = v.backup.active;
            if self.backup {
                Self::ev(&mut out, now, 2, "Backup on air (sound card input)".into());
            } else {
                Self::ev(&mut out, now, 0, "Stream back on air, backup off".into());
            }
        }
        // SFN lock
        if let Some(s) = &v.sfn {
            if self.sfn_locked != Some(s.locked) {
                if s.locked {
                    Self::ev(&mut out, now, 0, format!("SFN locked (error {:+.1} µs)", s.error_us));
                } else if self.sfn_locked == Some(true) {
                    Self::ev(&mut out, now, 1, format!("SFN not locked: {}", s.state));
                }
                self.sfn_locked = Some(s.locked);
            }
        }
        // the sound card (its state text)
        if v.card != self.card {
            self.card = v.card.clone();
            if !v.card.is_empty() {
                let bad = ["error", "not", "fail", "no "].iter().any(|w| v.card.to_lowercase().contains(w));
                Self::ev(&mut out, now, if bad { 1 } else { 0 }, v.card.clone());
            }
        }
        if p.need_password && !self.need_password {
            Self::ev(&mut out, now, 1, "An encrypted stream arrives, but no password is set".into());
        }
        self.need_password = p.need_password;
        // summaries of lost / restored / refused packets
        if now.saturating_sub(self.win_start) >= WINDOW_MS {
            let d_lost = lost.saturating_sub(self.base_lost);
            let d_conc = v.stats.concealed.saturating_sub(self.base_concealed);
            let d_fec = p.restored.saturating_sub(self.base_restored_fec);
            let d_unrec = p.lost.saturating_sub(self.base_unrec);
            if d_lost > 0 || d_fec > 0 || d_unrec > 0 {
                let mut t = if d_lost > 0 {
                    format!("{d_lost} packets lost ({:.0} ms concealed)", d_conc as f64 / 192.0)
                } else {
                    "No packets lost on air".to_string()
                };
                if d_fec > 0 {
                    t += &format!(", {d_fec} restored by FEC");
                }
                t += &format!(" in the last {} s", (now - self.win_start) / 1000);
                Self::ev(&mut out, now, if d_lost > 0 { 1 } else { 0 }, t);
            }
            let d_rej = p.rejected.saturating_sub(self.base_rejected);
            if d_rej > 0 {
                Self::ev(&mut out, now, 2, format!("{d_rej} packets rejected: wrong password, or changed on the way"));
            }
            let d_plain = p.plain_rejected.saturating_sub(self.base_plain);
            if d_plain > 0 {
                Self::ev(&mut out, now, 1, format!("{d_plain} unencrypted packets ignored (a password is set)"));
            }
            let d_blk = p.blocked.saturating_sub(self.base_blocked);
            if d_blk > 0 {
                Self::ev(&mut out, now, 1, format!("{d_blk} packets / connections blocked from {} (not an allowed sender)", p.blocked_from));
            }
            self.win_start = now;
            self.base_lost = lost;
            self.base_concealed = v.stats.concealed;
            self.base_restored_fec = p.restored;
            self.base_unrec = p.lost;
            self.base_rejected = p.rejected;
            self.base_plain = p.plain_rejected;
            self.base_blocked = p.blocked;
        }
        out
    }
}

/// "3 s", "2 min 5 s", "1 h 4 min".
pub fn dur(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min {} s", s / 60, s % 60)
    } else {
        format!("{} h {} min", s / 3600, s % 3600 / 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_lost_and_back() {
        let mut tr = Tracker::new();
        let mut v = RxView { from: "10.0.0.1:5000".into(), ..Default::default() };
        let mut log = Vec::new();
        let mut t = 1_000_000u64;
        let mut step = |tr: &mut Tracker, v: &RxView, log: &mut Vec<Event>, t: &mut u64| {
            log.extend(tr.observe(v, *t));
            *t += 500;
        };
        step(&mut tr, &v, &mut log, &mut t);
        v.receiving = true;
        v.stats.flac = true;
        for _ in 0..30 {
            step(&mut tr, &v, &mut log, &mut t);
        }
        v.stats.lost = 7;
        v.stats.concealed = 1920;
        for _ in 0..21 {
            step(&mut tr, &v, &mut log, &mut t);
        }
        v.receiving = false;
        for _ in 0..10 {
            step(&mut tr, &v, &mut log, &mut t);
        }
        v.receiving = true;
        step(&mut tr, &v, &mut log, &mut t);
        let texts: Vec<&str> = log.iter().map(|e| e.text.as_str()).collect();
        assert_eq!(texts[0], "Receiver started");
        assert!(texts[1].starts_with("Stream from 10.0.0.1:5000: FLAC 24-bit"), "{texts:?}");
        assert!(texts.iter().any(|t| t.starts_with("7 packets lost (10 ms concealed)")), "{texts:?}");
        assert_eq!(texts.iter().filter(|t| t.starts_with("Stream lost")).count(), 1, "{texts:?}");
        assert!(texts.last().unwrap().contains("after 4 s without stream"), "{texts:?}");
        assert_eq!(time_text(1_761_378_792_000, 7200), "25.10.2025 09:53:12");
    }
}
