# RadioMoon MPX

Two small stand-alone programs for FM broadcasting, without the RadioMoon database:

- **RadioMoon MPX Transmitter** – takes audio (a sound input, an internet stream or a folder of audio files), builds the FM composite signal (stereo with 19 kHz pilot and 38 kHz subcarrier, RDS at 57 kHz) and sends it to a 192 kHz sound card and/or over IP.
- **RadioMoon MPX Receiver** – for the transmitter site: receives the MPX stream over IP and plays it on a 192 kHz sound card into the MPX input of the exciter.

Both show an analyzer: spectrum 0–96 kHz with markers at 19, 38 and 57 kHz, the composite waveform, the pilot / RDS / L+R / L-R levels and the current and maximum deviation in kHz and % (100% = 75 kHz).

These programs were part of the RadioMoon repository up to version 0.5.30.

## Download

[Releases](../../releases):

| System | Files |
|---|---|
| Windows x64 / x86 / arm64 | `RadioMoon-MPX-<ver>-windows-<arch>-setup.exe`, or the portable `...-portable.zip` |
| Linux x64 / arm64 (Raspberry Pi 3/4/5, 64-bit OS) | `RadioMoon-MPX-Transmitter-<ver>-linux-<arch>.tar.gz`, `RadioMoon-MPX-Receiver-<ver>-linux-<arch>.tar.gz` |
| macOS (Apple Silicon) | `RadioMoon-MPX-<ver>-macos-arm64.dmg` (both apps) |

Linux needs a desktop session and `libasound2`. A 192 kHz capable sound card is required for the composite output.

## Putting RadioMoon on air

Enable RadioMoon's internet broadcast (Icecast or Shoutcast) and set the MPX Transmitter's source to **Internet stream** with that URL. RadioMoon sends the title of every song to the server, and the transmitter turns it into the RDS RadioText (*RadioText from the song*). A sound input fed from RadioMoon's output works too, with a fixed RadioText.

## Transport over IP

The Transmitter sends to any number of receivers (**Add receiver**); each output has its own address, port, protocol, format, bit depth and QoS, and shows its live bandwidth.

| Format | Bandwidth on the network* | Quality |
|---|---|---|
| L24 uncompressed | 4.9 Mbit/s | 24-bit, 192 kHz |
| L20 uncompressed | 4.0 Mbit/s | 20-bit (about 120 dB dynamic range) |
| L16 uncompressed | 3.3 Mbit/s | 16-bit (about 96 dB) |
| FLAC 24-bit | about 3.7 Mbit/s | bit-exact, identical to L24 |
| FLAC 20-bit | about 3.0 Mbit/s | identical to L20 |
| FLAC 16-bit | about 2.2 Mbit/s | identical to L16 |

\* including IP/UDP headers; FLAC depends on the audio. The sample rate is always 192 kHz: the MPX reaches 60 kHz (pilot 19, stereo 38, RDS 57 kHz). FM itself has about 70 dB signal-to-noise, so 16-bit is usually enough; 16 and 20 bits use TPDF dither.

Protocols: UDP (RTP payload types: L24 = 97, FLAC = 98, L16 = 99, L20 = 100) or TCP (FLAC streams start with `RMF1`, 16/20-bit PCM with `RMP1` + one byte with the bits, 24-bit PCM is raw). With FLAC every 2 ms packet (384 samples) is one complete, standard FLAC frame (24-bit, mono, 192 kHz); over TCP the stream starts with `RMF1` and every frame follows as a 16-bit length (big endian) and the frame. The receiver detects the format by itself.

### Compact MPX

Like μMPX, but for RadioMoon MPX Transmitter -> RadioMoon MPX Receiver (not compatible with Telos / Omnia μMPX, which is a closed format). The transmitter sends the processed stereo audio (after the 16 kHz filter, pre-emphasis and limiter) plus the levels and the RDS data; the receiver rebuilds the composite with exactly the same stereo encoder, pilot and RDS generator.

| Format | Bandwidth on the network | Audio |
|---|---|---|
| Compact · FLAC | about 0.95 Mbit/s | lossless (16-bit, 48 kHz) |
| Compact · Opus 320 / 256 / 192 / 128 | about 0.35 / 0.28 / 0.21 / 0.15 Mbit/s | 256 is transparent; 128 for weak links |

Packets: RTP payload type 101 (UDP) or a TCP stream starting with `RMC1`; each packet carries the audio frame, the four levels and (every 0.5 s or when it changes) the RDS data. A lost Opus packet is filled by Opus' own concealment, a lost FLAC packet by silence; the pilot and RDS never stop. Use the full composite formats (PCM / FLAC) when the transmitter's exact composite waveform must arrive, e.g. after an external composite clipper.

**QoS:** every output marks its packets with a DSCP class: EF (46, default) for real-time audio, AF41 (34) on AES67 networks, CS5, CS6, off, or any value 0-63. Linux and macOS set it directly. Windows ignores the marking of programs unless a QoS policy allows it: Group Policy > Computer Configuration > Windows Settings > Policy-based QoS > new policy for `RadioMoon-MPX-Transmitter.exe` with the DSCP value.

A lost UDP packet is concealed by repeating the last 2 ms. That is a whole number of cycles of 19, 38 and 57 kHz, so pilot, stereo and RDS stay in phase (radios do not drop to mono or lose RDS) and the timing stays exact. The receiver status shows the format, the bitrate and the lost / concealed packets.

## Service and system tray

**Service & tray > Install as service** runs the program in the background from the start of the computer:

- **Windows:** a Windows service (asks once for administrator rights). It starts before anyone logs in, keeps running after logoff and is restarted if it stops. Start / Stop in the window control the service (no administrator prompt needed).
- **Linux:** a systemd user service (`~/.config/systemd/user/radiomoon-mpx-*.service`, with `loginctl enable-linger` so it starts at boot).

The service uses the same settings file as the window. While it runs, the window shows its live status, bandwidth and analyzer. Transmitter level and RDS changes reach the service at once; for source and output changes use **Restart (apply settings)**.

The **tray icon** (Windows and Linux desktops with a tray) is coloured while running and grey when stopped; its tooltip shows the state and bandwidth. Closing the window keeps the program in the tray; use **Exit** in the tray menu to end it.

## Settings

`mpx-transmitter.json` and `mpx-receiver.json` are stored in ProgramData\RadioMoon (Windows) or ~/.local/share/RadioMoon (Linux/macOS), or next to the programs when a `portable.flag` file is there. Enable *Start ... when the program starts* for unattended use.

## Building

```
cargo build --release          # both programs in target/release
cargo test --lib               # codec, packet handling and RDS tests
.\build_windows.ps1 -Arch x64  # Windows setup + portable zip (needs Inno Setup 6)
```

Linux needs `libasound2-dev pkg-config libxkbcommon-dev libwayland-dev libgtk-3-dev`.

## Versioning and releases

The version lives in `Cargo.toml`. Pushing a new version to `main` creates the tag `vX.Y.Z` and the release (notes from the matching section of `CHANGELOG.md`) and builds every edition.
