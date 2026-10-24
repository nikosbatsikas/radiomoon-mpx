//! Settings of the MPX generator and its outputs.
use serde::{Deserialize, Serialize};

/// FM multiplex (MPX) generator, its outputs (192 kHz sound card, IP) and the receiver for the transmitter site.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MpxCfg {
    pub autostart: bool,
    pub stereo: bool,
    pub audio_level: f32,
    pub pilot_level: f32,
    pub rds_level: f32,
    pub preemph_us: f32,
    pub limiter: bool,
    pub output: f32,
    pub card_on: bool,
    pub card_device: String,
    pub ip_on: bool,
    pub ip_proto: String, // udp | tcp
    /// "l24" = uncompressed 24-bit, "flac" = lossless FLAC (bit-exact, smaller)
    pub ip_codec: String,
    pub ip_host: String,
    pub ip_port: u16,
    pub rds_on: bool,
    pub pi: String,
    pub ps: String,
    pub rt: String,
    pub rt_from_playlist: bool,
    pub pty: u8,
    pub tp: bool,
    pub ta: bool,
    pub music: bool,
}
impl Default for MpxCfg {
    fn default() -> Self {
        MpxCfg {
            autostart: false,
            stereo: true,
            audio_level: 0.85,
            pilot_level: 0.09,
            rds_level: 0.04,
            preemph_us: 50.0,
            limiter: true,
            output: 1.0,
            card_on: false,
            card_device: String::new(),
            ip_on: false,
            ip_proto: "udp".into(),
            ip_codec: "l24".into(),
            ip_host: "127.0.0.1".into(),
            ip_port: 5004,
            rds_on: true,
            pi: "1234".into(),
            ps: "RADIOMON".into(),
            rt: "RadioMoon".into(),
            rt_from_playlist: true,
            pty: 10,
            tp: false,
            ta: false,
            music: true,
        }
    }
}
