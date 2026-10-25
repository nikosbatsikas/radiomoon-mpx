# Raspberry Pi

Raspberry Pi 3 / 4 / 5 with a **64-bit** system: Raspberry Pi OS Lite 64-bit (no desktop) or [[Alpine Linux]] for Raspberry Pi.

## 1. Prepare the SD card
1. Install **Raspberry Pi Imager** (<https://www.raspberrypi.com/software/>) on your PC.
2. *Device*: your Pi; *Operating system*: **Raspberry Pi OS (other) → Raspberry Pi OS Lite (64-bit)**; *Storage*: the SD card.
3. *Edit settings*: host name (e.g. `site-lefkada`), user + password, WiFi (if used), time zone; tab *Services*: **Enable SSH**. → *Write*.
4. Put the card in the Pi, connect the network and power.

## 2. Install the Receiver
From your PC: `ssh user@site-lefkada.local`, then
```sh
wget -qO- https://raw.githubusercontent.com/nikosbatsikas/radiomoon-mpx/main/packaging/headless/install.sh | sudo sh -s -- receiver
```
Set the web password when asked. Open `http://site-lefkada.local:8081` ([[Web Interface]]).

## 3. Sound card
The Pi's own audio output cannot do 192 kHz: use a USB or HAT card that plays **192 kHz** (check with `aplay -l`; in the page *Output* shows *192 kHz* when it plays).

## 4. Clock for SFN
A GPS HAT with PPS: `sudo apt install gpsd chrony pps-tools`, add `dtoverlay=pps-gpio,gpiopin=18` to `/boot/firmware/config.txt`, configure chrony with the PPS refclock, then *System clock* in the SFN settings. See [[SFN]].

## Notes
- CPU: a Receiver (FLAC or Compact Opus) uses a few % of a Pi 4; a Pi 3 is fine for one Receiver.
- With a desktop, the normal Linux arm64 packages (window programs) work too: [[Install on Linux Desktop]].
