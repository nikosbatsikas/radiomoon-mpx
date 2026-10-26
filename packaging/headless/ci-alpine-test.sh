#!/bin/sh
# CI: installs the headless editions (built before by build-alpine.sh) with the installer under OpenRC on a CLEAN
# Alpine (nothing but the base system, as on a new installation) and checks the web interfaces.
set -e
mkdir -p /tmp/pkg
cp target/alpine/release/radiomoon-mpx-txd target/alpine/release/radiomoon-mpx-rxd target/alpine/release/RadioMoon-MPX-Transmitter.clap packaging/headless/install.sh /tmp/pkg/
apk add --no-cache openrc curl python3 >/dev/null   # (curl and python3 only for this test)
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
# the CLAP plug-in: in /usr/lib/clap, loads with musl as the service user (like radiomoon-server)
ls -l /usr/lib/clap/
su -s /bin/sh radiomoon -c "python3 packaging/headless/clap-load-test.py /usr/lib/clap/RadioMoon-MPX-Transmitter.clap"
# while the plug-in sends (its live status in the Transmitter's status file), the Transmitter service waits
su -s /bin/sh radiomoon -c "python3 -c '
import json,time
for i in range(16):
    open(\"/var/lib/radiomoon-mpx/mpx-transmitter-status.json\",\"w\").write(json.dumps({\"time_ms\":int(time.time()*1000),\"status\":\"test\",\"engine\":\"plugin\"}))
    time.sleep(0.2)
'"
grep -q "plug-in sends with these settings" /var/log/radiomoon-mpx/txd.log
sleep 7
grep -q "plug-in stopped" /var/log/radiomoon-mpx/txd.log
echo "Transmitter service waited for the plug-in and sent again"
sh /tmp/pkg/install.sh uninstall
test ! -e /usr/lib/clap/RadioMoon-MPX-Transmitter.clap
echo "Alpine install test passed"
