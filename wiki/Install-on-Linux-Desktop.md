# Install on Linux (desktop)

For computers with a desktop (GNOME, KDE, Xfce …). Servers and Raspberry Pi without desktop: [[Headless and Web Interface]].

## 1. Libraries
```sh
# Debian / Ubuntu / Linux Mint / Raspberry Pi OS (desktop)
sudo apt install libasound2 libgtk-3-0 libxkbcommon0 libxkbcommon-x11-0 libgl1   # Ubuntu 24.04: libasound2t64
# Fedora / AlmaLinux / Rocky
sudo dnf install alsa-lib gtk3 libxkbcommon libxkbcommon-x11 mesa-libGL
```

## 2. Download and unpack
```sh
cd ~/Downloads
tar xzf RadioMoon-MPX-Receiver-<ver>-linux-x64.tar.gz       # arm64 on Raspberry Pi
tar xzf RadioMoon-MPX-Transmitter-<ver>-linux-x64.tar.gz
mkdir -p ~/.local/bin
cp RadioMoon-MPX-Receiver/radiomoon-mpx-receiver RadioMoon-MPX-Transmitter/radiomoon-mpx-transmitter ~/.local/bin/
```
Start: `radiomoon-mpx-receiver` / `radiomoon-mpx-transmitter` (or double-click the file).

## 3. Menu entry (optional)
```sh
cat > ~/.local/share/applications/radiomoon-mpx-receiver.desktop <<'EOF'
[Desktop Entry]
Type=Application
Name=RadioMoon MPX Receiver
Exec=/home/YOUR-USER/.local/bin/radiomoon-mpx-receiver
Categories=AudioVideo;Audio;
EOF
```

## 4. Sound card at 192 kHz
- Your user must be in the group **audio** on some systems: `sudo usermod -aG audio $USER`, then log out and in.
- **PipeWire** (Ubuntu 22.10+, Fedora, Debian 12) plays at 48 kHz by default. Allow 192 kHz:
  ```sh
  mkdir -p ~/.config/pipewire/pipewire.conf.d
  printf 'context.properties = { default.clock.allowed-rates = [ 44100 48000 96000 192000 ] }\n' > ~/.config/pipewire/pipewire.conf.d/192k.conf
  systemctl --user restart pipewire pipewire-pulse
  ```
  Or choose the card's own ALSA device (`hw:CARD=…` / `plughw:…`) in the Receiver's *Output* list: then nothing resamples.
- Check: `aplay -l` lists the cards; the Receiver's *Output* shows *192 kHz* when the card plays.

## 5. Run as a service
Tab **Service, tray & window → Install as service**: a systemd *user* service (`~/.config/systemd/user/radiomoon-mpx-*.service`) with `loginctl enable-linger`, so it starts at boot. See [[Service and Tray]].

## 6. Firewall
`sudo ufw allow 5004/udp` (Ubuntu) or `sudo firewall-cmd --permanent --add-port=5004/udp && sudo firewall-cmd --reload` (Fedora / Alma) at the Receiver.

Continue with [[First Steps]].

If the program does not start and says *Library libxkbcommon-x11.so could not be loaded*: install `libxkbcommon-x11-0` (Debian / Ubuntu) or `libxkbcommon-x11` (Fedora / Alma).
