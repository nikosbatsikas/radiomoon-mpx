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

| Format | Bitrate | Quality |
|---|---|---|
| L24 uncompressed | 4.6 Mbit/s | 24-bit, 192 kHz |
| FLAC lossless | about 3–4 Mbit/s (depends on the audio) | bit-exact, identical to L24 |

Protocols: UDP (RTP; L24 = payload type 97, FLAC = 98) or TCP. With FLAC every 2 ms packet (384 samples) is one complete, standard FLAC frame (24-bit, mono, 192 kHz); over TCP the stream starts with `RMF1` and every frame follows as a 16-bit length (big endian) and the frame. The receiver detects the format by itself.

A lost UDP packet is concealed by repeating the last 2 ms. That is a whole number of cycles of 19, 38 and 57 kHz, so pilot, stereo and RDS stay in phase (radios do not drop to mono or lose RDS) and the timing stays exact. The receiver status shows the format, the bitrate and the lost / concealed packets.

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
