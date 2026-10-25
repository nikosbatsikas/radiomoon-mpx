RadioMoon MPX - headless editions (no desktop, web interface)

  radiomoon-mpx-txd   Transmitter, web interface on port 8080
  radiomoon-mpx-rxd   Receiver,    web interface on port 8081

Install (as root):   sh install.sh receiver      (or: transmitter, both, uninstall)
The installer adds the ALSA packages, a service user "radiomoon" (groups audio, dialout), the services
(OpenRC on Alpine, systemd on Debian / Ubuntu / Raspberry Pi OS) and asks for the password of the web page.

Without the installer:
  ./radiomoon-mpx-rxd --data /var/lib/radiomoon-mpx --listen 0.0.0.0:8081
  ./radiomoon-mpx-rxd --data /var/lib/radiomoon-mpx --set-login admin     (sets the web password)
On the first start without a login, a random password is printed and kept in web-initial-password.txt.

The settings files are the same as those of the desktop programs (mpx-transmitter.json, mpx-receiver.json).
Documentation: https://github.com/nikosbatsikas/radiomoon-mpx/wiki
