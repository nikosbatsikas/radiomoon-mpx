# Headless and Web Interface

For Linux servers and Raspberry Pi without a desktop (and [[Alpine Linux]]): two programs without a window, set up and watched in a **browser**.

| Program | Web port | Settings file |
|---|---|---|
| `radiomoon-mpx-txd` (Transmitter) | 8080 | `/var/lib/radiomoon-mpx/mpx-transmitter.json` |
| `radiomoon-mpx-rxd` (Receiver) | 8081 | `/var/lib/radiomoon-mpx/mpx-receiver.json` |

## Install (Debian 10+, Ubuntu 20.04+, Raspberry Pi OS 64-bit, AlmaLinux / Rocky / RHEL 8+, Fedora, Alpine)
```sh
wget -qO- https://raw.githubusercontent.com/nikosbatsikas/radiomoon-mpx/main/packaging/headless/install.sh | sudo sh -s -- receiver
```
(`transmitter`, `both`, or `uninstall`; on Alpine run it as root without `sudo`). Or download `RadioMoon-MPX-Headless-linux-<arch>.tar.gz`, unpack and run `sudo sh install.sh receiver`.

The installer adds the ALSA packages, a service user `radiomoon` (groups audio, dialout), a **systemd** or **OpenRC** service that restarts by itself, and asks for the **web password** (user `admin`). Options: `RM_PASSWORD`, `RM_USER`, `RM_TX_PORT`, `RM_RX_PORT`, `RM_VERSION` (environment).

## The web page
`http://<address>:8081` (Receiver) / `:8080` (Transmitter). Log in; the tabs are the same as in the window:
- **Status:** live link, RDS, protection / backup / SFN, spectrum and levels (every second).
- **Log** (Receiver): the [[Receiver Log]] in local time.
- **Settings tabs:** *Save and apply* — outputs and sources restart, levels and RDS change at once.
- **Login:** change user and password.

Security: login with a password (PBKDF2), session cookie (HttpOnly, SameSite=Strict), every change needs a special request header (no cross-site requests), 10 wrong passwords lock an address for 5 minutes. The page is **plain HTTP**: use it in your own network or through a VPN / SSH tunnel (`ssh -L 8081:localhost:8081 pi@site`), not open to the internet.

## Command line
```
radiomoon-mpx-rxd --data DIR --listen 0.0.0.0:8081     # run
radiomoon-mpx-rxd --data DIR --set-login admin          # set the password (from standard input)
radiomoon-mpx-rxd --no-web                              # without web page
radiomoon-mpx-rxd --version | --help
```
Without a login file the first start sets a random password and writes it to `web-initial-password.txt` (deleted when you change the password).

## Services
- systemd: `systemctl status|restart radiomoon-mpx-rxd`, log `journalctl -u radiomoon-mpx-rxd -f`
- OpenRC: `rc-service radiomoon-mpx-rxd status|restart`, log `/var/log/radiomoon-mpx/rxd.log`

The Transmitter and the Receiver on one computer share the login.
