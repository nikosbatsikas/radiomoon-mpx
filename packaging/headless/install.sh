#!/bin/sh
# RadioMoon MPX - installer of the headless editions (no desktop; web interface) for
#   Alpine Linux (OpenRC)            x86_64, aarch64 (Raspberry Pi 3/4/5)
#   Debian / Ubuntu / Raspberry Pi OS (systemd), 64-bit: x86_64, aarch64 (Debian 10 or newer, Ubuntu 20.04 or newer)
#   AlmaLinux / Rocky Linux / RHEL / CentOS Stream / Fedora (systemd, dnf), version 8 or newer; firewalld and SELinux are handled
#
# One command (downloads the latest release):
#   wget -qO- https://raw.githubusercontent.com/nikosbatsikas/radiomoon-mpx/main/packaging/headless/install.sh | sh -s -- receiver
# or from an unpacked package:   sh install.sh receiver | transmitter | both | uninstall
#
# Options (environment): RM_PASSWORD=...    web login password (else asked; at least 8 characters)
#                        RM_USER=admin      web login user
#                        RM_TX_PORT=8080    RM_RX_PORT=8081   ports of the web interfaces
#                        RM_VERSION=1.11.0  a release instead of the latest
set -eu

REPO="nikosbatsikas/radiomoon-mpx"
PREFIX=/usr/local/bin
DATA=/var/lib/radiomoon-mpx
LOGS=/var/log/radiomoon-mpx
SVCUSER=radiomoon
TX_PORT="${RM_TX_PORT:-8080}"
RX_PORT="${RM_RX_PORT:-8081}"
WEBUSER="${RM_USER:-admin}"

say() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31mError:\033[0m %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || die "run as root (Alpine: doas / su; Debian: sudo)"

# ---------------------------------------------------------------- the system
if [ -x /sbin/openrc-run ] && command -v rc-update >/dev/null 2>&1; then INIT=openrc
elif command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then INIT=systemd
else die "neither OpenRC nor systemd found"; fi
if command -v apk >/dev/null 2>&1; then PKG=apk; LIBC=alpine
elif command -v apt-get >/dev/null 2>&1; then PKG=apt; LIBC=linux
elif command -v dnf >/dev/null 2>&1; then PKG=dnf; LIBC=linux
elif command -v yum >/dev/null 2>&1; then PKG=yum; LIBC=linux
else die "this installer knows Alpine (apk), Debian / Ubuntu / Raspberry Pi OS (apt) and AlmaLinux / Rocky / RHEL / Fedora (dnf)"; fi
case "$(uname -m)" in
  x86_64|amd64) ARCH=x86_64; GARCH=x64 ;;
  aarch64|arm64) ARCH=aarch64; GARCH=arm64 ;;
  *) die "$(uname -m) is not supported (64-bit x86_64 or aarch64 needed; on a Raspberry Pi use a 64-bit system)" ;;
esac

WHAT="${1:-}"
if [ -z "$WHAT" ]; then
  if [ -t 0 ] || [ -r /dev/tty ]; then
    printf 'Install which program? [r]eceiver, [t]ransmitter, [b]oth: ' >&2
    read -r a </dev/tty || a=r
    case "$a" in t*|T*) WHAT=transmitter ;; b*|B*) WHAT=both ;; *) WHAT=receiver ;; esac
  else WHAT=receiver; fi
fi

svc_names() {
  case "$WHAT" in
    receiver|rx) echo "rxd" ;;
    transmitter|tx) echo "txd" ;;
    both) echo "txd rxd" ;;
    uninstall) echo "txd rxd" ;;
    *) die "unknown choice \"$WHAT\" (receiver, transmitter, both, uninstall)" ;;
  esac
}

stop_service() { # $1 = txd | rxd
  n="radiomoon-mpx-$1"
  if [ "$INIT" = openrc ]; then
    [ -f "/etc/init.d/$n" ] && { rc-service "$n" stop >/dev/null 2>&1 || true; }
  else
    systemctl stop "$n" >/dev/null 2>&1 || true
  fi
  return 0
}

# ---------------------------------------------------------------- uninstall
if [ "$WHAT" = uninstall ]; then
  for s in txd rxd; do
    n="radiomoon-mpx-$s"
    stop_service "$s"
    if [ "$INIT" = openrc ]; then rc-update del "$n" default >/dev/null 2>&1 || true; rm -f "/etc/init.d/$n" "/etc/conf.d/$n"
    else systemctl disable "$n" >/dev/null 2>&1 || true; rm -f "/etc/systemd/system/$n.service"; systemctl daemon-reload; fi
    rm -f "$PREFIX/$n"
  done
  say "Removed the programs and services. Settings and logs are kept in $DATA and $LOGS (delete them by hand if not needed)."
  exit 0
fi

# ---------------------------------------------------------------- packages
say "System: $LIBC ($PKG, $INIT), $ARCH"
if [ "$PKG" = apk ]; then
  apk add --no-cache alsa-lib alsa-utils ca-certificates >/dev/null
elif [ "$PKG" = dnf ] || [ "$PKG" = yum ]; then
  "$PKG" install -y -q alsa-lib alsa-utils ca-certificates wget tar gzip >/dev/null
else
  DEBIAN_FRONTEND=noninteractive apt-get update -qq >/dev/null
  DEBIAN_FRONTEND=noninteractive apt-get install -y -qq libasound2 alsa-utils ca-certificates wget >/dev/null || \
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq libasound2t64 alsa-utils ca-certificates wget >/dev/null
fi

# ---------------------------------------------------------------- the programs
HERE="$(cd "$(dirname "$0")" 2>/dev/null && pwd || echo .)"
SRC=""
if [ -x "$HERE/radiomoon-mpx-rxd" ] && [ -x "$HERE/radiomoon-mpx-txd" ]; then
  SRC="$HERE"
else
  ASSET="RadioMoon-MPX-Headless-$LIBC-$GARCH.tar.gz"
  if [ -n "${RM_VERSION:-}" ]; then URL="https://github.com/$REPO/releases/download/v$RM_VERSION/$ASSET"
  else URL="https://github.com/$REPO/releases/latest/download/$ASSET"; fi
  TMP="$(mktemp -d)"
  trap 'rm -rf "$TMP"' EXIT
  say "Downloading $URL"
  if command -v wget >/dev/null 2>&1; then wget -qO "$TMP/p.tar.gz" "$URL" || die "download failed"
  else curl -fsSL -o "$TMP/p.tar.gz" "$URL" || die "download failed"; fi
  tar -xzf "$TMP/p.tar.gz" -C "$TMP"
  SRC="$(dirname "$(find "$TMP" -name radiomoon-mpx-rxd -type f | head -n1)")"
  [ -x "$SRC/radiomoon-mpx-rxd" ] || die "the package has no programs"
fi

# the service user (in the audio group, for the sound cards; dialout for a GPS receiver)
if ! id "$SVCUSER" >/dev/null 2>&1; then
  if [ "$PKG" = apk ]; then adduser -S -D -H -h "$DATA" -s /sbin/nologin -G audio "$SVCUSER"
  else
    NOLOGIN=/usr/sbin/nologin; [ -x "$NOLOGIN" ] || NOLOGIN=/sbin/nologin
    useradd --system --home-dir "$DATA" --shell "$NOLOGIN" --gid audio "$SVCUSER"
  fi
fi
for g in audio dialout; do
  if grep -q "^$g:" /etc/group; then
    if [ "$PKG" = apk ]; then addgroup "$SVCUSER" "$g" >/dev/null 2>&1 || true; else usermod -aG "$g" "$SVCUSER" || true; fi
  fi
done
mkdir -p "$DATA" "$LOGS"
chown "$SVCUSER":audio "$DATA" "$LOGS"
chmod 750 "$DATA"

for s in $(svc_names); do
  stop_service "$s"
  install -m 755 "$SRC/radiomoon-mpx-$s" "$PREFIX/radiomoon-mpx-$s"
done
# SELinux (AlmaLinux / RHEL / Fedora): the usual labels for programs and data
if command -v restorecon >/dev/null 2>&1; then
  restorecon -R "$PREFIX"/radiomoon-mpx-* "$DATA" "$LOGS" >/dev/null 2>&1 || true
fi
B1="$PREFIX/radiomoon-mpx-$(svc_names | cut -d' ' -f1)"
if ! "$B1" --version >/dev/null 2>&1; then
  "$B1" --version || true
  die "the program does not run on this system (see the message above; a too old system library?)"
fi
VER="$("$B1" --version 2>/dev/null || echo "RadioMoon MPX")"
say "Installed: $VER"

# ---------------------------------------------------------------- web login
if [ ! -s "$DATA/web-login.json" ] || [ -n "${RM_PASSWORD:-}" ]; then
  PW="${RM_PASSWORD:-}"
  while [ -z "$PW" ]; do
    [ -r /dev/tty ] || die "set RM_PASSWORD (no terminal to ask)"
    printf 'Password for the web interface (user "%s", at least 8 characters): ' "$WEBUSER" >/dev/tty
    stty -echo </dev/tty 2>/dev/null || true; read -r p1 </dev/tty; stty echo </dev/tty 2>/dev/null || true; printf '\n' >/dev/tty
    printf 'Again: ' >/dev/tty
    stty -echo </dev/tty 2>/dev/null || true; read -r p2 </dev/tty; stty echo </dev/tty 2>/dev/null || true; printf '\n' >/dev/tty
    if [ "$p1" != "$p2" ]; then echo "Not the same, again." >&2; continue; fi
    if [ "${#p1}" -lt 8 ]; then echo "At least 8 characters, again." >&2; continue; fi
    PW="$p1"
  done
  B="$PREFIX/radiomoon-mpx-$(svc_names | cut -d' ' -f1)"
  printf '%s\n' "$PW" | su -s /bin/sh "$SVCUSER" -c "$B --data $DATA --set-login '$WEBUSER'" >/dev/null 2>&1 || \
    { printf '%s\n' "$PW" | "$B" --data "$DATA" --set-login "$WEBUSER" >/dev/null 2>&1; chown "$SVCUSER":audio "$DATA/web-login.json"; }
  rm -f "$DATA/web-initial-password.txt"
  say "Web login set (user \"$WEBUSER\"). The Transmitter and the Receiver on this computer share it."
fi

# ---------------------------------------------------------------- services
for s in $(svc_names); do
  n="radiomoon-mpx-$s"
  if [ "$s" = txd ]; then title="RadioMoon MPX Transmitter"; port="$TX_PORT"; else title="RadioMoon MPX Receiver"; port="$RX_PORT"; fi
  if [ "$INIT" = openrc ]; then
    cat >"/etc/conf.d/$n" <<EOF
# $title: address and port of the web interface
RM_LISTEN="0.0.0.0:$port"
EOF
    cat >"/etc/init.d/$n" <<EOF
#!/sbin/openrc-run
name="$title"
description="$title (headless, web interface on port \${RM_LISTEN##*:})"
supervisor=supervise-daemon
command="$PREFIX/$n"
command_args="--data $DATA --listen \${RM_LISTEN:-0.0.0.0:$port}"
command_user="$SVCUSER:audio"
output_log="$LOGS/$s.log"
error_log="$LOGS/$s.log"
respawn_delay=2
respawn_max=0
# the built-in PTP client listens on the ports 319 / 320
capabilities="^cap_net_bind_service"

depend() {
	use net
	after net alsasound firewall
}

start_pre() {
	checkpath -d -o $SVCUSER:audio -m 0750 $DATA
	checkpath -d -o $SVCUSER:audio $LOGS
}
EOF
    chmod 755 "/etc/init.d/$n"
    rc-update add "$n" default >/dev/null
    [ -e /etc/init.d/alsasound ] && rc-update add alsasound boot >/dev/null 2>&1 || true
    if rc-service "$n" status >/dev/null 2>&1; then act=restart; else act=start; fi
    if ! rc-service "$n" "$act"; then
      echo "Could not start $n. Its log ($LOGS/$s.log):" >&2
      tail -n 20 "$LOGS/$s.log" >&2 2>/dev/null || true
      exit 1
    fi
  else
    cat >"/etc/systemd/system/$n.service" <<EOF
[Unit]
Description=$title (headless, web interface on port $port)
After=network-online.target sound.target
Wants=network-online.target

[Service]
User=$SVCUSER
Group=audio
SupplementaryGroups=dialout
ExecStart=$PREFIX/$n --data $DATA --listen 0.0.0.0:$port
Restart=always
RestartSec=2
# the built-in PTP client listens on the ports 319 / 320
AmbientCapabilities=CAP_NET_BIND_SERVICE
LimitRTPRIO=95
Nice=-10

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
    systemctl enable "$n" >/dev/null 2>&1
    if ! systemctl restart "$n"; then
      journalctl -u "$n" -n 20 --no-pager >&2 || true
      exit 1
    fi
  fi
done

# ---------------------------------------------------------------- firewall (firewalld: AlmaLinux / RHEL / Fedora)
if command -v firewall-cmd >/dev/null 2>&1 && firewall-cmd --state >/dev/null 2>&1; then
  for s in $(svc_names); do
    if [ "$s" = txd ]; then firewall-cmd -q --permanent --add-port="$TX_PORT/tcp" || true
    else
      firewall-cmd -q --permanent --add-port="$RX_PORT/tcp" || true
      # the default stream port of the Receiver (change it here if you use another one)
      firewall-cmd -q --permanent --add-port=5004/udp --add-port=5004/tcp || true
    fi
  done
  firewall-cmd -q --reload || true
  say "firewalld: opened the web port(s)$(echo " $(svc_names)" | grep -q rxd && echo " and the stream port 5004 (UDP and TCP)")"
fi

# ---------------------------------------------------------------- done
IP="$(ip -4 -o addr show scope global 2>/dev/null | awk '{print $4}' | cut -d/ -f1 | head -n1)"
[ -n "$IP" ] || IP="$(hostname -i 2>/dev/null | awk '{print $1}')"
[ -n "$IP" ] || IP="this-computer"
echo
say "Done."
for s in $(svc_names); do
  if [ "$s" = txd ]; then echo "  Transmitter: http://$IP:$TX_PORT"; else echo "  Receiver:    http://$IP:$RX_PORT"; fi
done
echo "  Settings and status: $DATA    Logs: $([ "$INIT" = openrc ] && echo "$LOGS" || echo "journalctl -u radiomoon-mpx-rxd / -txd")"
echo "  Sound cards: aplay -l / arecord -l (the 192 kHz card for the MPX)."
echo "  Open the stream ports (e.g. UDP 5004) and the web ports in a firewall, if one runs."
