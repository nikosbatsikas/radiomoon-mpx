#!/bin/sh
# CI: builds the headless editions on Alpine, then installs them with the installer under OpenRC and checks the
# web interfaces (login, settings, status).
set -e
sh packaging/headless/build-alpine.sh
mkdir -p /tmp/pkg
cp target/alpine/release/radiomoon-mpx-txd target/alpine/release/radiomoon-mpx-rxd packaging/headless/install.sh /tmp/pkg/
apk add --no-cache openrc curl >/dev/null
# OpenRC in a container: no real boot, so mark the system as a container and enter the default runlevel
sed -i 's/^#\?rc_sys=.*/rc_sys="docker"/' /etc/rc.conf
mkdir -p /run/openrc && touch /run/openrc/softlevel
openrc default >/dev/null 2>&1 || true
RM_PASSWORD=secret-pass-1 sh /tmp/pkg/install.sh both || {
  echo "== install failed; OpenRC state:"; ls -R /run/openrc | head -60
  rc-service -v radiomoon-mpx-txd start 2>&1 | tail -20
  ls -l /var/log/radiomoon-mpx; exit 1; }
sleep 3
echo "== results"
rc-status -a 2>&1 | grep -i radiomoon || true
echo "-- rxd.log"; cat /var/log/radiomoon-mpx/rxd.log || true
echo "-- txd.log"; cat /var/log/radiomoon-mpx/txd.log || true
ldd /usr/local/bin/radiomoon-mpx-rxd
curl -s -o /dev/null -w "rx page %{http_code}\n" http://127.0.0.1:8081/
curl -s -c /tmp/cj -H "X-RadioMoon: 1" -X POST -d '{"user":"admin","password":"secret-pass-1"}' http://127.0.0.1:8081/api/login | grep -q ok
curl -s -b /tmp/cj http://127.0.0.1:8081/api/settings | grep -q '"proto"'
curl -s -b /tmp/cj http://127.0.0.1:8081/api/status | head -c 200; echo
curl -s -c /tmp/cj2 -H "X-RadioMoon: 1" -X POST -d '{"user":"admin","password":"secret-pass-1"}' http://127.0.0.1:8080/api/login | grep -q ok
curl -s -b /tmp/cj2 http://127.0.0.1:8080/api/status | head -c 200; echo
sh /tmp/pkg/install.sh uninstall
echo "Alpine install test passed"
