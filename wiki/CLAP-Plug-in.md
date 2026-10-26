# CLAP Plug-in

"RadioMoon MPX Transmitter" for programs that load CLAP plug-ins: audio programs (REAPER, Bitwig Studio, FL Studio, Studio One ...) and **radiomoon-server** (the RadioMoon automation server). On the master track the audio passes through unchanged and the plug-in sends the multiplex with the Transmitter's settings (IP targets, card, RDS, Compact MPX, SFN, protection). Any host sample rate.

## Files

| System | Plug-in file | Where to put it |
|---|---|---|
| Windows | in the setup (option *Install the CLAP plug-in*) and in the zip (`CLAP` folder) | `C:\Program Files\Common Files\CLAP` |
| macOS | `RadioMoon MPX Transmitter.clap` in the dmg | `~/Library/Audio/Plug-Ins/CLAP` |
| Linux (glibc: Debian 10+, Ubuntu 20.04+, AlmaLinux / RHEL 8+, Fedora, Raspberry Pi OS) | `RadioMoon-MPX-Transmitter-linux-<arch>.clap`, also in the Transmitter and headless packages | `~/.clap` or `/usr/lib/clap` |
| Alpine Linux (musl) | `RadioMoon-MPX-Transmitter-alpine-<arch>.clap`, also in the Alpine package | `/usr/lib/clap` |

A plug-in must be built for the same system and C library as the program that loads it: a glibc plug-in does not load in a musl program (Alpine) and the other way round. The headless installer puts the right one into `/usr/lib/clap` when it installs the Transmitter.

## Settings

The plug-in uses the first of:
1. `mpx-transmitter.json` next to the plug-in file or in the folder above it (portable zip);
2. the folder in the environment variable `RADIOMOON_MPX_DATA`;
3. on Linux with the **headless Transmitter** installed: its settings in `/var/lib/radiomoon-mpx` — set up on its web page (`http://<address>:8080`);
4. the settings of the Transmitter window (`C:\ProgramData\RadioMoon`, `~/.local/share/RadioMoon`).

The Transmitter window or web page shows **PLUG-IN** and the plug-in's live status. While the plug-in sends, the Transmitter service waits (the MPX does not go out twice) and sends again when the plug-in stops. Settings changes reach the plug-in at once (levels, RDS) or restart its engine (outputs, card, SFN).

## radiomoon-server on Alpine Linux

1. Install the headless Transmitter on the server computer (it brings the plug-in and the web page for its settings):
   ```sh
   wget -qO- https://raw.githubusercontent.com/nikosbatsikas/radiomoon-mpx/main/packaging/headless/install.sh | sh -s -- transmitter
   ```
   The installer puts the plug-in into `/usr/lib/clap/RadioMoon-MPX-Transmitter.clap`.
2. Open `http://<address>:8080`, set the IP outputs (Receivers), RDS, signal and SFN → **Save and apply**. (The audio source tab does not matter for the plug-in: its audio comes from radiomoon-server.)
3. In radiomoon-server: scan the CLAP plug-ins (it searches `/usr/lib/clap`) or upload `RadioMoon-MPX-Transmitter-alpine-<arch>.clap`, and add **RadioMoon MPX Transmitter** to the output chain.
4. The Transmitter's web page now shows **PLUG-IN**; the Receivers get the MPX of the automation.

Both programs run as the user `radiomoon`, so the plug-in can read and write the Transmitter's folder. If radiomoon-server runs as another user, add that user to the group `audio` and give the group write access: `chmod 770 /var/lib/radiomoon-mpx`.
