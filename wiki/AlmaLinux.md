# AlmaLinux / Rocky Linux / RHEL

The headless editions run on **AlmaLinux, Rocky Linux, RHEL, CentOS Stream 8 / 9 / 10 and Fedora** (x86_64 and aarch64) as systemd services, set up in the browser ([[Headless and Web Interface]]).

## Install the Transmitter as a service

As root (or with `sudo`):
```sh
curl -fsSL https://raw.githubusercontent.com/nikosbatsikas/radiomoon-mpx/main/packaging/headless/install.sh | sudo sh -s -- transmitter
```
(`receiver`, `both` or `uninstall` likewise). The installer:
1. `dnf install alsa-lib alsa-utils wget tar`
2. downloads the latest `RadioMoon-MPX-Headless-linux-<arch>.tar.gz` and installs `/usr/local/bin/radiomoon-mpx-txd`
3. adds the system user `radiomoon` (groups audio, dialout) and `/var/lib/radiomoon-mpx`
4. asks for the web password (user `admin`)
5. writes `/etc/systemd/system/radiomoon-mpx-txd.service`, enables and starts it
6. **firewalld:** opens TCP 8080 (Transmitter page), and for the Receiver TCP 8081 and UDP / TCP 5004
7. **SELinux:** sets the usual labels (`restorecon`); the service runs in the normal unconfined domain, nothing to switch off

Then open `http://<address>:8080`, log in and set the source, the IP outputs and RDS. *Save and apply* restarts the transmitter with the new settings.

## Control the service
```sh
systemctl status radiomoon-mpx-txd       # state
systemctl restart radiomoon-mpx-txd      # restart
systemctl stop radiomoon-mpx-txd         # stop
systemctl disable --now radiomoon-mpx-txd   # stop and do not start at boot
journalctl -u radiomoon-mpx-txd -f       # its messages
```
It starts at boot and is restarted by systemd if it stops. Settings: `/var/lib/radiomoon-mpx/mpx-transmitter.json`.

## Without the installer (by hand)
```sh
dnf install -y alsa-lib alsa-utils
tar xzf RadioMoon-MPX-Headless-linux-x64.tar.gz
install -m 755 RadioMoon-MPX-Headless-*/radiomoon-mpx-txd /usr/local/bin/
useradd --system --home-dir /var/lib/radiomoon-mpx --shell /sbin/nologin --gid audio radiomoon
mkdir -p /var/lib/radiomoon-mpx && chown radiomoon:audio /var/lib/radiomoon-mpx
echo 'my-web-password' | sudo -u radiomoon /usr/local/bin/radiomoon-mpx-txd --data /var/lib/radiomoon-mpx --set-login admin
cat >/etc/systemd/system/radiomoon-mpx-txd.service <<'EOF'
[Unit]
Description=RadioMoon MPX Transmitter
After=network-online.target sound.target
Wants=network-online.target
[Service]
User=radiomoon
Group=audio
ExecStart=/usr/local/bin/radiomoon-mpx-txd --data /var/lib/radiomoon-mpx --listen 0.0.0.0:8080
Restart=always
RestartSec=2
AmbientCapabilities=CAP_NET_BIND_SERVICE
[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload && systemctl enable --now radiomoon-mpx-txd
firewall-cmd --permanent --add-port=8080/tcp && firewall-cmd --reload
restorecon -R /usr/local/bin/radiomoon-mpx-txd /var/lib/radiomoon-mpx
```

## With a desktop (GNOME)
The window program (`RadioMoon-MPX-Transmitter-<ver>-linux-x64.tar.gz`) can install itself as a service too: *Service & tray → Install as service* makes a systemd **user** service that starts at boot (with `loginctl enable-linger`). On a server without a desktop use the headless edition above.

## Notes
- Sound cards: `aplay -l`; the user `radiomoon` is in the group `audio`. A 192 kHz card is needed only for the MPX sound card output / MPX input.
- The Transmitter *sends*; firewalld allows outgoing traffic, so nothing more is needed for the IP outputs. At a Receiver, open the stream port if you use another one than 5004: `firewall-cmd --permanent --add-port=6000/udp && firewall-cmd --reload`.
- AlmaLinux 8 / 9 / 10 and Rocky need version **1.11.1** or newer of the packages.
