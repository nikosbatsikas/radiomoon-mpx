# Changelog

Versions follow MAJOR.MINOR.PATCH. The version lives in `Cargo.toml`. Pushing a new version to `main` creates the tag and the release automatically.

## 1.6.0
- **MPX input (Transmitter):** a new audio source, "MPX input (192 kHz)": a ready FM composite signal (e.g. the MPX output of a processor or stereo encoder) is recorded from a sound card input at 192 kHz (left or right channel, with a level control) and sent as it is to the IP receivers (PCM L16/L20/L24 or FLAC) and the 192 kHz sound card output. The analyzer and deviation meter show it; the Receiver also decodes its RDS. Compact MPX is not possible with an MPX input (it carries audio, not a composite); such an output says so.
- **Fix:** the value tiles (Bandwidth, Format, Buffer ... in the Receiver; Deviation, IP bandwidth ... in the Transmitter) could make the window content wider than the window, which cut off the right side. The tiles now always fit and wrap into more rows on a narrow window.

## 1.5.0
- **RDS fix (Transmitter):** block D of every RDS group was sent with the wrong offset word (C' instead of D). FM radios could read the PI code and programme type, but not the station name (PS) or the RadioText. Both now appear on radios. (The same error was in the MPX generator of RadioMoon up to 0.5.30.)
- **RDS display (Receiver):** a new RDS panel decodes the RDS of the received MPX, whatever the format: station name as on a radio display, RadioText, PI code, programme type (number and name), TP / TA / music, and the share of correct blocks. Works with and without the stereo pilot.
- **Windows fit their content:** both programs make their window as high as its content (e.g. when switching tabs or when the Link and RDS panels appear), within the screen. Can be switched off: Service & tray > Window.

## 1.4.0
- **Compact MPX** (like μMPX, for RadioMoon MPX Transmitter -> Receiver): instead of the composite waveform the transmitter sends the processed stereo audio (band-limited, pre-emphasised and limited) with the levels and the RDS data; the receiver rebuilds the composite with the same stereo matrix, 19 kHz pilot and RDS encoder. Choose it per receiver in IP outputs > Format:
  - **Compact · FLAC**: the audio lossless (16-bit, 48 kHz), about 0.95 Mbit/s.
  - **Compact · Opus**: 320 / 256 / 192 / 128 kbit/s, about 0.35 / 0.28 / 0.21 / 0.15 Mbit/s on the network.
  The rebuilt composite has the same audio, pilot, stereo and RDS levels as the original (checked band by band). Lost packets: Opus fills the gap with its own concealment, FLAC with silence; pilot and RDS continue without a break. Works over UDP (RTP payload type 101) and TCP ("RMC1").
- Receiver: detects Compact MPX by itself and shows "Compact / Opus audio" or "Compact / FLAC audio".
- Not compatible with Telos / Omnia μMPX equipment (a closed, licensed format).

## 1.3.0
- **Run as a service** (Windows and Linux): "Service & tray" > Install as service. On Windows it is a Windows service that starts with the computer, before anyone logs in, and keeps running after logoff (restarted automatically if it stops). On Linux it is a systemd user service. The service uses the settings of the window, and the window shows its live status, bandwidth and analyzer and starts / stops it. Level and RDS changes reach a running transmitter service at once; source and output changes with Restart.
- **System tray icon** (Windows and Linux desktops): coloured while running, grey when stopped, with the state in the tooltip; menu Show / Exit. Closing the window keeps the program running in the tray (can be switched off).
- Transmitter: the settings are in tabs (Audio source, IP outputs, Sound card, Signal, RDS, Service & tray), which makes the window much smaller.
- Transmitter: the "Folder" audio source is removed; the sources are an internet stream (with the song titles as RadioText) and a sound input. A saved folder source changes to internet stream.
- Receiver: Input and Output take one third of the width, the Link panel two thirds.
- Receiver: the settings are kept in ProgramData on Windows (shared with the service); older settings in LocalAppData are taken over.

## 1.2.0
- Smaller, more compact windows: smaller text, buttons, cards, graphs and analyzer, so more fits on the screen.
- Each program has its own icon: a blue transmitter tower for the MPX Transmitter, a green receiving dish for the MPX Receiver (window, taskbar, .exe file, Start menu and macOS apps).
- About window in both programs (version, features, authors, project link).
- Windows: the .exe files carry version information (Properties > Details).

## 1.1.0
- **Several receivers at once:** the Transmitter sends the MPX to any number of IP outputs (Add receiver), each with its own name, address, port, protocol (UDP RTP or TCP), format and QoS. Each output runs on its own, so a slow or unreachable receiver never disturbs the others. A single receiver of older settings is taken over automatically.
- **Bit depth per output:** 24-bit (full quality), 20-bit or 16-bit, as uncompressed PCM (L24 / L20 / L16) or FLAC lossless. The sample rate stays 192 kHz (needed for the 19, 38 and 57 kHz carriers). 16 and 20 bits use TPDF dither. Typical bandwidth: L24 4.9, L20 4.0, L16 3.3, FLAC 24-bit about 3.7, FLAC 20-bit about 3.0, FLAC 16-bit about 2.2 Mbit/s.
- **QoS:** every output marks its packets with a DSCP class (default EF 46; AF41 34 for AES67 networks, CS5, CS6, off or any value 0-63), so routers and switches with QoS give the MPX priority. On Windows the marking needs a QoS policy (see README).
- **Bandwidth readouts:** live Mbit/s per output (including IP/UDP or TCP headers), the share against L24 for FLAC, data sent, and a 2-minute graph; in the Receiver a Link panel with Mbit/s, format and bit depth, buffer, lost / concealed packets, data received and the graph.
- **New look:** both programs use cards in the RadioMoon colours, a header with Start / Stop and an ON AIR / RECEIVING light, the deviation bar and, on wide windows, two columns.
- The Receiver detects L16 / L20 / L24 and FLAC 16 / 20 / 24-bit by itself.

## 1.0.0
- RadioMoon MPX Transmitter and MPX Receiver now have their own repository and releases (they were part of RadioMoon up to 0.5.30). RadioMoon itself no longer contains an MPX generator.
- Transmitter: with an internet stream as the source, the song title sent by the server (Icecast / Shoutcast StreamTitle) becomes the RDS RadioText. This is how RadioMoon's song titles reach RDS: use RadioMoon's internet broadcast as the transmitter's source.
- Everything from RadioMoon 0.5.30: L24 or FLAC lossless over UDP (RTP) or TCP, automatic format detection in the receiver, and phase-aligned concealment of lost packets.
- Windows: own setup (RadioMoon MPX) and portable zip for x64, x86 and arm64. Linux: transmitter and receiver packages for x64 and arm64. macOS: one dmg with both apps.
