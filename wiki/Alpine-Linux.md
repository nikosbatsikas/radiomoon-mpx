# Alpine Linux

The headless editions are built for Alpine (musl) on **x86_64** and **aarch64** (Raspberry Pi 3 / 4 / 5 with 64-bit Alpine): `RadioMoon-MPX-Headless-alpine-<arch>.tar.gz`. They need only `libgcc` and `alsa-lib` from Alpine; the installer adds them.

## 1. Install Alpine
1. Download the **Standard** (PC) or **Raspberry Pi** image from <https://alpinelinux.org/downloads/> and write it to a USB stick / SD card (e.g. with Raspberry Pi Imager, balenaEtcher or `dd`).
2. Boot, log in as `root` (no password) and run `setup-alpine`: keyboard, host name, network (DHCP or a fixed address – a fixed one is better for a site), root password, time zone, **NTP: chrony**, mirror, **SSH server: openssh**, disk: `sda` → `sys` (installs to the disk).
3. `reboot` and remove the stick.

## 2. Install RadioMoon MPX
As root (directly or over `ssh root@site`):
```sh
apk add wget
wget -qO- https://raw.githubusercontent.com/nikosbatsikas/radiomoon-mpx/main/packaging/headless/install.sh | sh -s -- receiver
```
(`transmitter`, `both` or `uninstall`). What you see:
```
==> System: alpine (apk, openrc), x86_64
==> Downloading https://github.com/nikosbatsikas/radiomoon-mpx/releases/latest/download/RadioMoon-MPX-Headless-alpine-x64.tar.gz
==> Installed: radiomoon-mpx-rxd 1.12.4
Password for the web interface (user "admin", at least 8 characters):
Again:
==> Web login set (user "admin"). ...
==> Done.
  Receiver:    http://192.168.1.20:8081
```
The script installs `libgcc alsa-lib alsa-utils`, the program in `/usr/local/bin`, the user `radiomoon` (groups audio, dialout), `/var/lib/radiomoon-mpx`, the OpenRC service (supervise-daemon: restarted if it stops; `capabilities` for the PTP ports) in the *default* runlevel, and starts it.

## 3. Open the page
`http://<address>:8081` → log in → *Output*: the 192 kHz card → **Save and apply** ([[Web Interface]]).

## Service
```sh
rc-service radiomoon-mpx-rxd status        # state
rc-service radiomoon-mpx-rxd restart
tail -f /var/log/radiomoon-mpx/rxd.log      # the log (stream lost / back, lost packets …)
vi /etc/conf.d/radiomoon-mpx-rxd            # RM_LISTEN="0.0.0.0:8081" (port of the page)
```

## Sound card
`aplay -l` lists the cards. The card must play (and for a backup source record) at **192 kHz**; USB cards need `snd-usb-audio` (in `linux-lts` / `linux-rpi`). Keep the mixer levels: `rc-update add alsasound boot`, set them with `alsamixer`, then `alsactl store`.

## Clock (SFN)
`setup-alpine` with *chrony* keeps the clock in time (NTP, a few ms). For real SFN use a GPS with PPS (`apk add gpsd chrony`, PPS on the serial DCD line) or PTP (`apk add linuxptp`), and *System clock* in the SFN settings.

## Diskless (Raspberry Pi, `lbu`)
`lbu include /usr/local/bin/radiomoon-mpx-rxd /var/lib/radiomoon-mpx` and `lbu commit`.

## CLAP plug-in (radiomoon-server)
With `transmitter` or `both` the installer also puts the musl plug-in into `/usr/lib/clap`, so radiomoon-server on the same computer can load it; set it up on the Transmitter's web page. See [[CLAP Plug-in]].

## Firewall (awall / iptables)
Open the stream port (e.g. UDP 5004) and the web port (8081) for your network.
