# Raspberry Pi

Raspberry Pi 3 / 4 / 5 with a **64-bit** system: Raspberry Pi OS Lite (bookworm or newer) or Alpine for Raspberry Pi. Use the headless edition ([[Headless and Web Interface]], [[Alpine Linux]]):
```sh
wget -qO- https://raw.githubusercontent.com/nikosbatsikas/radiomoon-mpx/main/packaging/headless/install.sh | sudo sh -s -- receiver
```
- **Sound card:** the Pi's own audio does not do 192 kHz; use a USB or HAT card that plays 192 kHz (check with `aplay -l` and the Receiver's status).
- **CPU:** the Receiver (FLAC or Compact) uses a few % of a Pi 4; the Transmitter with several outputs and Opus more. A Pi 3 is fine for one Receiver.
- **Clock for SFN:** a GPS HAT with PPS + gpsd + chrony, then *System clock* in the SFN tab.
- With a desktop, the normal Linux arm64 packages (window programs) work too.
