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
    /// Master switch for all IP outputs.
    pub ip_on: bool,
    /// Every receiver the MPX is sent to (each with its own protocol and format).
    pub targets: Vec<IpTarget>,
    /// Settings of versions before 1.1.0 (a single receiver); moved into `targets` by `migrate`.
    #[serde(skip_serializing)]
    pub ip_proto: String,
    #[serde(skip_serializing)]
    pub ip_codec: String,
    #[serde(skip_serializing)]
    pub ip_host: String,
    #[serde(skip_serializing)]
    pub ip_port: u16,
    pub targets_ready: bool,
    /// SFN: stamp every UDP packet with the UTC time of its first sample from the reference clock.
    pub sfn_on: bool,
    pub clock: crate::clock::ClockCfg,
    pub rds_on: bool,
    pub pi: String,
    /// Station name: up to 8 characters, or a longer text shown in pages (see ps_mode).
    pub ps: String,
    /// "static" | "pages" (words, 8 characters a page) | "scroll"
    pub ps_mode: String,
    /// Seconds per page (or per character when scrolling).
    pub ps_interval: f32,
    /// Alternative frequencies in MHz, e.g. "94.3, 101.5".
    pub af: String,
    /// Clock time (CT) once a minute, with the local offset in hours.
    pub ct_on: bool,
    pub ct_offset_h: f32,
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
            targets: Vec::new(),
            ip_proto: "udp".into(),
            ip_codec: "l24".into(),
            ip_host: String::new(),
            ip_port: 5004,
            targets_ready: false,
            sfn_on: false,
            clock: crate::clock::ClockCfg::default(),
            rds_on: true,
            ps_mode: "static".into(),
            ps_interval: 3.0,
            af: String::new(),
            ct_on: false,
            ct_offset_h: 2.0,
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

impl MpxCfg {
    /// Moves the single receiver of older settings into the list (or adds one example the first time).
    pub fn migrate(&mut self) {
        if self.targets_ready {
            return;
        }
        if self.targets.is_empty() {
            let mut t = IpTarget::default();
            if !self.ip_host.trim().is_empty() {
                t.host = self.ip_host.trim().to_string();
                t.port = self.ip_port;
                t.proto = if self.ip_proto == "tcp" { "tcp".into() } else { "udp".into() };
                t.codec = if self.ip_codec == "flac" { "flac".into() } else { "l24".into() };
            }
            t.name = "Transmitter site".into();
            self.targets.push(t);
        }
        self.targets_ready = true;
    }
}

/// One receiver of the MPX stream.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct IpTarget {
    pub enabled: bool,
    /// Free text shown in the list, e.g. the site name.
    pub name: String,
    pub host: String,
    pub port: u16,
    /// "udp" (RTP) | "tcp"
    pub proto: String,
    /// The composite: "l24" = uncompressed PCM, "flac" = lossless FLAC.
    /// Compact MPX (the audio, rebuilt into the composite by the receiver): "cflac" = FLAC, "copus" = Opus.
    pub codec: String,
    /// Bits per sample: 24 (full quality), 20 or 16 (less bandwidth). The sample rate is always 192 kHz.
    pub bits: u32,
    /// QoS: DSCP value written into every IP packet (0 = not marked, 46 = EF, 34 = AF41).
    pub dscp: u8,
    /// Opus bit rate of Compact MPX, kbit/s.
    pub opus_kbps: u32,
}

impl Default for IpTarget {
    fn default() -> Self {
        IpTarget { enabled: true, name: String::new(), host: "127.0.0.1".into(), port: 5004, proto: "udp".into(), codec: "flac".into(), bits: 24, dscp: 46, opus_kbps: 256 }
    }
}

impl IpTarget {
    pub fn address(&self) -> String {
        format!("{}:{}", self.host.trim(), self.port)
    }
    pub fn label(&self) -> String {
        if self.name.trim().is_empty() {
            self.address()
        } else {
            self.name.trim().to_string()
        }
    }
    pub fn bits(&self) -> u32 {
        match self.bits {
            16 => 16,
            20 => 20,
            _ => 24,
        }
    }
    /// Short name of the DSCP class, e.g. "EF".
    pub fn dscp_name(&self) -> String {
        dscp_name(self.dscp)
    }
    /// Compact MPX (audio + levels + RDS) instead of the composite waveform.
    pub fn is_compact(&self) -> bool {
        self.codec == "cflac" || self.codec == "copus"
    }
    pub fn kind(&self) -> String {
        let p = if self.proto == "tcp" { "TCP" } else { "UDP" };
        if self.codec == "copus" {
            format!("{p} · Compact Opus {}", self.opus_kbps)
        } else if self.codec == "cflac" {
            format!("{p} · Compact FLAC")
        } else if self.codec == "flac" {
            format!("{p} · FLAC {}-bit", self.bits())
        } else {
            format!("{p} · L{}", self.bits())
        }
    }
}

/// Usual names of DSCP values.
pub fn dscp_name(v: u8) -> String {
    match v {
        0 => "Off".into(),
        46 => "EF".into(),
        34 => "AF41".into(),
        36 => "AF42".into(),
        26 => "AF31".into(),
        40 => "CS5".into(),
        48 => "CS6".into(),
        32 => "CS4".into(),
        v => format!("DSCP {v}"),
    }
}
