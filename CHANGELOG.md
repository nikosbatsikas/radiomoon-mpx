# Changelog

Versions follow MAJOR.MINOR.PATCH. The version lives in `Cargo.toml`. Pushing a new version to `main` creates the tag and the release automatically.

## 1.12.2
- **SFN: clear state when nothing plays.** When the sound card cannot be opened (e.g. "does not support 192 kHz output"), the SFN state now says *Not playing* with the reason, instead of staying on "Waiting for time stamps" (the playout only runs while a card plays).
- **New output "No sound card (test output)"** in the Receiver (window and web page): takes the samples at the pace of 192 kHz and plays nothing, so the link, the analysis, the log and the SFN lock can be tested on a computer without a 192 kHz card.
- **SFN: clock warning.** When the arrival margin is longer than the delay, the Receiver warns that the Transmitter's clock runs ahead of its own, by how much, and to synchronise the clocks.

## 1.12.1
- **Web interface: About.** An *About* button in the header opens the About box, as in the window programs: program logo, version, what the program does, the authors, links to the project, the wiki and the releases, and - after login - this system: computer name, operating system and processor, how long the program has been running and its data folder. The program logo is also in the header and in the browser tab.

## 1.12.0
- **RDS: Extended Country Code (ECC).** New field *ECC* next to the PI code (Transmitter window and web page): the Transmitter sends group 1A with the ECC about every 3.5 s. With the first digit of the PI it names the country world-wide - e.g. Greece E1 (PI 1xxx), Cyprus E2 (PI 2xxx) - as cars with hybrid radio (RadioDNS) need it to find the station's logo and information. Empty = not sent (as before).
- The Receiver decodes the ECC and shows it next to the PI (window and web page); Compact MPX carries it, and an MPX input passes it on.

## 1.11.1
- **AlmaLinux / Rocky Linux / RHEL / CentOS Stream / Fedora** for the headless editions: the installer uses dnf, opens the web port (and the Receiver's stream port 5004) in firewalld and sets the SELinux labels; then the Transmitter / Receiver runs as a systemd service.
- The headless Linux packages are now built on AlmaLinux 8 (glibc 2.28), so they also run on AlmaLinux / RHEL 8 and 9, Debian 10 / 11 and Ubuntu 20.04 (1.11.0 needed glibc 2.35). Tested on AlmaLinux 9 and 10, Debian 12, Ubuntu 22.04 and 24.04.
- **Alpine fix:** the installer now also installs `libgcc` (the programs did not start on a minimal Alpine: "Error loading shared library libgcc_s.so.1"). The CI test now installs on a clean Alpine.
- The installer stops with a clear message when the program cannot run on the system.
- Wiki: AlmaLinux page, QoS on Windows.

## 1.11.0
- **Headless editions with a web interface** (Linux, Raspberry Pi, **Alpine Linux**): `radiomoon-mpx-txd` (Transmitter, port 8080) and `radiomoon-mpx-rxd` (Receiver, port 8081) run without a desktop as services and are set up in a browser: live status with spectrum, levels and RDS, every setting of the window programs, the Receiver's log, and the login. Login with user and password (PBKDF2), HttpOnly / SameSite cookies, protection against cross-site requests and a lock-out after 10 wrong passwords. Same settings files as the window programs.
- **Installer** `packaging/headless/install.sh` (one command): Alpine (OpenRC, supervise-daemon) and Debian / Ubuntu / Raspberry Pi OS (systemd); installs ALSA, a service user, the service and the web password; `uninstall` removes it. New packages: `RadioMoon-MPX-Headless-alpine-x64/arm64.tar.gz` (musl, built on Alpine) and `RadioMoon-MPX-Headless-linux-x64/arm64.tar.gz`.
- **Receiver log:** stream lost / back (with the time without stream), lost packets every 10 s (concealed ms, restored by FEC), wrong password, unencrypted or blocked senders, backup on / off, SFN lock, sound card, TCP connections. New **Log** tab in the window (with Copy), on the web page, and in the system log of the headless Receiver.
- **Wiki** with pages for installation, both programs, formats, protection, SFN, RDS, backup, log, CLAP, headless / Alpine / Raspberry Pi and troubleshooting (published from the `wiki/` folder).
- FEC: packets before the first one received are no longer counted as "could not restore".
- The window libraries are now an optional feature (`gui`, on by default); the CLAP plug-in is built without them.

## 1.10.0
- **Backup MPX source (Receiver):** Input > Backup source. When the stream stops (after a set time, default 1 s), the MPX of a sound card input (192 kHz, left or right, level in dB) goes on air - an analog STL, an off-air receiver or a local processor - and the stream goes back on air after it has come without a break for a set time (default 5 s). Optionally also when the stream carries silence. The header shows ON BACKUP; scope, RDS and levels follow what is on air.
- **Protection of the MPX stream**, per IP output (Transmitter) and in Input > Protection (Receiver):
  - **Password / encryption:** ChaCha20-Poly1305 with a key from the password (PBKDF2-HMAC-SHA256). Only a Receiver with the same password can play the stream, and every packet is authenticated, so nobody can send a false or changed stream to the transmitter site. Over UDP and TCP, with all formats and SFN. A Receiver with a password ignores unencrypted streams; the header shows ENCRYPTED.
  - **FEC (UDP):** one recovery packet for every 20, 10 or 5 packets restores a lost packet; *burst protection* (interleaving 2 / 4 / 8) also restores several packets lost in a row. The extra delay is shown next to the setting. The Receiver shows the restored packets.
  - **Allowed senders (Receiver):** addresses and networks (IPv4 / IPv6, e.g. 10.0.0.0/8); other senders are refused and counted.
  - The cryptography is built in (no new libraries) and checked against the RFC 8439 test vectors and 3000 random cases of the Python `cryptography` package.
- **CLAP plug-in of the Transmitter:** "RadioMoon MPX Transmitter" runs inside audio programs that load CLAP plug-ins (REAPER, Bitwig Studio, FL Studio, Studio One ...). On the master (or any stereo) track it passes the audio through unchanged and sends the FM multiplex built from it to the IP targets and / or the 192 kHz sound card - with RDS, Compact MPX, SFN and QoS - using the Transmitter's settings. The Transmitter window shows **PLUG-IN** with the plug-in's live status, and its setting changes reach the plug-in at once. Included for Windows x64 / x86 / arm64 (setup option: Common Files\CLAP), Linux x64 / arm64 and macOS. See *CLAP plug-in* in the README.
- **RDS clock time (CT): automatic daylight saving time.** New setting *Daylight saving*: Automatic (Europe), Automatic (USA / Canada) or Off. With automatic, the offset is the standard (winter) time - Greece +2 - and the hour is added in summer by itself (Europe: last Sunday of March to last Sunday of October, at 01:00 UTC). The RDS tab shows the time and offset sent now. The new default is Automatic (Europe). Compact MPX carries the setting.
- **RDS AF fix:** a frequency typed half way (e.g. 95.0 while typing 95.7) could stay in the AF list of radios. The AF list is now sent when it is finished (Enter or leaving the field), and the Receiver shows each complete AF list as received, so a changed list replaces the old one instead of adding to it. Frequencies can also be typed without the dot (957 = 95.7, 1015 = 101.5); the field is tidied to "95.7, 101.5".

## 1.9.0
- **AES67 / Dante source (Transmitter):** a new audio source, "AES67 / Dante": receives an AES67 stream (RTP L24 or L16 at 44.1, 48 or 96 kHz, multicast or unicast, any number of channels; you choose the left and right channel). Streams that announce themselves (SAP / SDP, as Dante, Ravenna and AES67 devices do) appear in a list to choose from. Lost packets are filled with silence so the timing stays exact; the generator takes the stream's sample rate directly. Dante: switch the device to AES67 mode in Dante Controller and create a multicast flow (Dante's own format is closed and licensed by Audinate); Dante Virtual Soundcard also works through "Sound input".
- **RDS (Transmitter), new settings in the RDS tab:**
  - **Long station name (PS):** a longer text is shown 8 characters at a time - *in pages* (whole words, centred) or *scrolling* - with the time per page in seconds, and a preview of the pages. A complete name is always sent before the next page, so radios never show a mix; Compact MPX and SFN sites show the same page.
  - **Alternative frequencies (AF):** a list of up to 25 frequencies (87.6 - 107.9 MHz, method A) in group 0A, so radios can switch to the strongest transmitter of the programme.
  - **Clock time (CT):** group 4A once a minute with the date, time and the local time offset; radios set their clocks.
  - The programme type shows its name next to the number.
- **RDS (Receiver):** the RDS panel also shows the AF list and the clock time; the station name is shown only when it came complete, so pages never appear mixed.
- Compact MPX carries the AF list and the clock time setting too (older receivers ignore them).

## 1.8.0
- **SFN (single frequency network)** with GPS or PTP (IEEE 1588v2) time, in a new SFN tab of both programs:
  - **Transmitter:** "Send SFN time stamps": every UDP packet carries the UTC time of its first sample (RTP header extension; receivers without SFN ignore it). Works with every source, also the MPX input from a sound card (the input card's own clock is followed), and with every format (PCM, FLAC, Compact MPX; the codec delays of Compact MPX are compensated).
  - **Receiver:** "Play every sample at its time stamp + delay": the same delay at all sites puts all transmitters on air in step. A fine resampler (32-tap windowed sinc, flat to 60 kHz) follows the reference clock, so the sound card's own clock does not drift away. Shown live: state, timing error (µs), rate trim (ppm), arrival margin, stamp jitter, restarts; an "SFN LOCKED" light in the header. Per-site output correction (µs) for the delay of sound card, exciter and cables.
  - **Reference clock:** system clock (most precise when the computer is synchronised by PTP, e.g. ptp4l + phc2sys or the Windows PTP client, or by GPS PPS with gpsd + chrony), the built-in PTP client (IEEE 1588v2 default profile, multicast, end-to-end delay; about ±10-100 µs with software time stamps), or a GPS receiver on a serial port (NMEA; with PPS on the DCD line about ±0.1-1 ms).
  - Tested: two simulated sites with sound cards 40 ppm fast and 25 ppm slow and different network delays lock to about 1 µs; live, FLAC, L16 and Compact Opus sites (and an MPX input with a 30 ppm fast card) lock within some tens of µs on a busy test computer.
- **Receiver window:** the Link panel is at the top; below it the tabs RDS, Input, Output, SFN and Service, tray & window.

## 1.7.0
- **MPX input to Compact MPX:** with an MPX input as the source, Compact MPX outputs now work too. The Transmitter decodes the composite (stereo audio with a pilot PLL and the 38 kHz demodulator, the measured pilot and RDS levels, and the RDS station name, RadioText, PI, PTY and flags) and sends that as Compact MPX; the receiver rebuilds the composite with the same levels and RDS. Tested: audio, pilot and stereo levels match the input within 0.1 dB, RDS level measured exactly (2 / 4 / 6 %), more than 30 dB stereo separation. Other RDS services of the input (clock, AF lists ...) are not carried; use PCM or FLAC for an exact copy of the input.

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
