//! Live status that a program running as a service writes to a small JSON file next to its settings, so the
//! window (started by the user) can show what the service is doing: levels, bandwidth, analyzer and state.
use crate::model::IpTarget;
use crate::mpx::LinkStat;
use crate::mpxrx::RxView;
use crate::mpxscope::ScopeData;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A status file older than this is from a service that is no longer running.
pub const FRESH: Duration = Duration::from_secs(4);

/// Status of the MPX Transmitter running as a service.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TxStatus {
    pub time_ms: u64,
    /// "Running", "Waiting for audio..."
    pub status: String,
    /// State of the audio source.
    pub source: String,
    /// Song title (RadioText).
    pub now: String,
    pub peak: f32,
    pub card: String,
    pub links: Vec<(IpTarget, LinkStat)>,
    pub scope: ScopeData,
    /// SFN reference clock (when SFN is on).
    pub clock: Option<crate::clock::ClockState>,
}

/// Status of the MPX Receiver running as a service.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RxStatus {
    pub time_ms: u64,
    pub status: String,
    pub peak: f32,
    pub view: RxView,
    pub scope: ScopeData,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The status file that belongs to a settings file (`mpx-transmitter.json` -> `mpx-transmitter-status.json`).
pub fn path_for(settings: &Path) -> PathBuf {
    let stem = settings.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "mpx".into());
    settings.with_file_name(format!("{stem}-status.json"))
}

/// Writes the status in one piece (temporary file + rename), so a reader never sees half a file.
pub fn write<T: Serialize>(path: &Path, v: &T) {
    if let Ok(js) = serde_json::to_vec(v) {
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, js).is_ok() && std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

/// Reads a status that is not older than `FRESH`.
pub fn read<T: DeserializeOwned>(path: &Path, time_of: impl Fn(&T) -> u64) -> Option<T> {
    let v: T = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let age = now_ms().saturating_sub(time_of(&v));
    (age < FRESH.as_millis() as u64).then_some(v)
}

pub fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}
