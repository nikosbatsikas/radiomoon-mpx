# Alpine Linux

The headless editions are built for Alpine (musl) on x86_64 and aarch64 (Raspberry Pi 3/4/5 with 64-bit Alpine): `RadioMoon-MPX-Headless-alpine-<arch>.tar.gz`. They need only `alsa-lib` from Alpine.

## Install
On a fresh Alpine (after `setup-alpine`; only the main repository is needed), as root:
```sh
apk add wget
wget -qO- https://raw.githubusercontent.com/nikosbatsikas/radiomoon-mpx/main/packaging/headless/install.sh | sh -s -- receiver
```
The script:
1. `apk add alsa-lib alsa-utils`
2. downloads the latest Alpine package for the processor, installs `radiomoon-mpx-rxd` / `-txd` in `/usr/local/bin`
3. adds the system user `radiomoon` (groups audio, dialout) and `/var/lib/radiomoon-mpx`
4. asks for the web password
5. writes the OpenRC service (`supervise-daemon`, restarts by itself, `capabilities` for the PTP ports), `rc-update add ... default`, starts it

Then open `http://<address>:8081`. Port: `/etc/conf.d/radiomoon-mpx-rxd` (`RM_LISTEN`). Log: `/var/log/radiomoon-mpx/rxd.log`.

## Sound card
`aplay -l` lists the cards. For the MPX the card must play (and for a backup source record) at **192 kHz**; USB cards need the `snd-usb-audio` module (in Alpine's `linux-lts` / `linux-rpi`). Run `rc-update add alsasound boot` to keep the mixer levels (`alsactl store`).

## Diskless / data mode (Raspberry Pi)
On a diskless Alpine (`lbu`), keep the program and its data: `lbu include /usr/local/bin/radiomoon-mpx-rxd /var/lib/radiomoon-mpx` and `lbu commit`.

## Firewall (awall / iptables)
Open the stream port (e.g. UDP 5004) and the web port (8081) for your network.
