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
| macOS (Apple Silicon) | `RadioMoon-MPX-<ver>-macos-arm64.dmg` (both apps and the CLAP plug-in) |
| CLAP plug-in | in every package: Windows setup (option "Install the CLAP plug-in"), `CLAP\RadioMoon-MPX-Transmitter.clap` in the zip, `RadioMoon-MPX-Transmitter.clap` in the Linux transmitter package, `RadioMoon MPX Transmitter.clap` in the dmg |

Linux needs a desktop session and `libasound2`. A 192 kHz capable sound card is required for the composite output.

## CLAP plug-in

The Transmitter is also a **CLAP plug-in** ("RadioMoon MPX Transmitter", vendor RadioMoon) for audio programs that load CLAP plug-ins: REAPER, Bitwig Studio, FL Studio, Studio One, MultitrackStudio and others. Put it on the master track (or any stereo track): the audio passes through unchanged, and the plug-in builds the multiplex from it - stereo, pilot, RDS, Compact MPX, SFN time stamps, QoS - and sends it to the IP targets and / or the 192 kHz sound card, exactly as the Transmitter program would.

- **Install:** Windows: the setup's option copies it to `C:\Program Files\Common Files\CLAP` (the x86 setup to the 32-bit Common Files, for 32-bit hosts). Linux: copy `RadioMoon-MPX-Transmitter.clap` to `~/.clap` (or `/usr/lib/clap`). macOS: copy `RadioMoon MPX Transmitter.clap` from the dmg to `~/Library/Audio/Plug-Ins/CLAP`. Then let the audio program rescan its plug-ins.
- **Settings:** the plug-in uses the settings of the Transmitter program (targets, signal, RDS, SFN ...): set them up in the Transmitter window. While the plug-in runs, the window shows **PLUG-IN** and its live status (links, Mbit/s, levels, scope); changes made in the window reach the plug-in within a quarter of a second (outputs and SFN restart its engine, level and RDS are applied at once). The window's own Start is not needed then, and the service should be stopped.
- **Portable:** a `mpx-transmitter.json` next to the plug-in file, or in the folder above it (the portable zip), is used instead of the shared settings.
- The plug-in takes any sample rate of the host (44.1, 48, 96 kHz ...). The audio thread never waits: if the host renders faster than real time (export / freeze), the extra audio is dropped from the MPX, the track itself is not affected.

## AES67 / Dante input

Audio source > **AES67 / Dante** receives an AES67 stream: RTP L24 or L16, 44.1 / 48 / 96 kHz, multicast (or unicast), any number of channels with a free choice of the left and right channel. Announced streams (SAP on 239.255.255.255 and 224.2.127.254, port 9875) appear in the list *Announced streams*; otherwise enter the multicast address, port, format and channels by hand. The Transmitter follows the stream's own rate, so it does not need the PTP clock of the AES67 network.

**Dante:** Dante's own protocol is closed (Audinate). Use one of:
- **AES67 mode** of the Dante device: Dante Controller > Device Config > AES67 Config > enable, set the multicast prefix, then create a multicast transmit flow with the channels; the flow appears in the list.
- **Dante Virtual Soundcard** (or Dante Via) on the Transmitter computer: it is a sound card, so choose *Sound input*.

## MPX input

The Transmitter can also send a ready composite signal: Audio source > **MPX input (192 kHz)**. Choose a sound card input that records at 192 kHz and the channel (left or right) that carries the MPX, e.g. from the MPX output of a broadcast processor. PCM and FLAC outputs send the signal as it is (an exact copy). Compact MPX outputs decode it first - stereo audio, pilot and RDS levels, RDS station name, RadioText, PI, PTY and flags - and the receiver rebuilds the composite from that (other RDS services such as clock time or AF lists are not carried). The Signal and RDS settings are not used with an MPX input.

## SFN (single frequency network)

Several transmitters on the same frequency must put the same signal on air at the same moment.

1. **Transmitter**, SFN tab: switch on *Send SFN time stamps* and choose the reference clock. Every UDP packet then carries the UTC time of its first sample.
2. **Every Receiver**, SFN tab: switch on *Play every sample at its time stamp + delay*, choose the reference clock and set the **same delay** at all sites, longer than the slowest network path (the *Arrival margin* shows how much time is left). Calibrate each site's *Output correction* once (sound card, exciter, cables), e.g. with an oscilloscope against the GPS PPS.

Reference clocks: **System clock** (best: the computer synchronised by PTP - ptp4l + phc2sys on Linux, the PTP client of Windows - or by GPS PPS with gpsd + chrony), the built-in **PTP** client (IEEE 1588v2 default profile, multicast 224.0.1.129, end-to-end delay, ports 319/320; software time stamps, about ±10-100 µs), or a **GPS receiver** on a serial port (NMEA $xxRMC / $xxZDA; with PPS on the DCD line about ±0.1-1 ms, NMEA alone about ±10 ms for tests).

The time stamp is an RTP header extension (profile 0x524D, 8 bytes, ns since 1970 UTC), so it needs UDP; receivers without SFN ignore it. The receiver follows the stamps with a fine resampler, so its sound card clock does not matter. The overall accuracy also depends on how exactly the sound driver reports its output latency; for coverage overlap areas calibrate each site.

## Putting RadioMoon on air

Enable RadioMoon's internet broadcast (Icecast or Shoutcast) and set the MPX Transmitter's source to **Internet stream** with that URL. RadioMoon sends the title of every song to the server, and the transmitter turns it into the RDS RadioText (*RadioText from the song*). A sound input fed from RadioMoon's output works too, with a fixed RadioText.

## Protection of the stream

Every IP output of the Transmitter (each receiver in the IP tab) has its own protection settings; the Receiver sets its own in Input > *Protection*. Needs Transmitter and Receiver 1.10.0 or newer.

- **Password (encryption):** the stream is encrypted with ChaCha20-Poly1305 (the cipher of TLS 1.3 and WireGuard); the key is made from the password with PBKDF2-HMAC-SHA256 (100 000 rounds). Every packet is also authenticated: without the password nobody can listen to the MPX, and nobody can send a false or changed stream to the transmitter site. A Receiver with a password plays only the stream with the same password and ignores unencrypted ones; without a password it plays only unencrypted streams. The header shows **ENCRYPTED**. Works over UDP and TCP, with every format (PCM, FLAC, Compact MPX) and with SFN. Cost: 39 bytes per packet (about 0.25 Mbit/s with L24 / FLAC, 0.02 Mbit/s with Compact Opus). The password is kept in the settings file (readable by the users of the computer).
- **FEC (forward error correction, UDP):** one extra packet for every 20, 10 or 5 packets (+5 / +10 / +20 % bandwidth) lets the Receiver restore a lost packet of the group, so a lost packet is not heard at all. *Burst protection* (2, 4 or 8) interleaves the groups, so that many packets lost in a row are restored too. The Receiver waits for the FEC packet of a group, which adds a delay of n x burst x packet length: shown next to the setting (e.g. FEC 1/10 with FLAC: +20 ms; with Compact Opus: +200 ms, so choose 1/5 there). Tested with 3 % random loss (FEC 1/10 restores about 70 % of the lost packets, 1/5 more) and with bursts of 2 (Opus, FEC 1/5 x2: all restored).
- **Allowed senders (Receiver):** a list of addresses or networks, e.g. `192.168.1.10, 10.0.0.0/8, 2001:db8::/32`. Packets (UDP) and connections (TCP) from other addresses are refused and counted. Empty = everyone. Use it together with a firewall; a password is the stronger protection, since sender addresses can be forged on some networks.

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
