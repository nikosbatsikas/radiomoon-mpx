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

## The plug-in's window

Click the plug-in's editor button (REAPER: the *UI* button of the FX window; Bitwig: the window icon of the device; Studio One: the editor button): the **Transmitter window opens in plug-in mode** — title *RadioMoon MPX Transmitter - plug-in in …*, the same look as the program, with the tabs **Level** (of the track's audio), **IP outputs**, **Sound card**, **Signal**, **RDS** and **SFN**, the **PLUG-IN** state, the deviation meter and the analyzer. Changes are saved at once and reach the plug-in (levels and RDS at once; outputs, card and SFN restart its engine). Close it like any window.

The window is the Transmitter program (it opens as a separate, floating window), so the window program must be installed:

| System | The plug-in finds the program |
|---|---|
| Windows | setup: `C:\Program Files\RadioMoon MPX\RadioMoon-MPX-Transmitter.exe`; portable zip: one folder above `CLAP` |
| macOS | `/Applications/RadioMoon MPX Transmitter.app` (from the dmg) |
| Linux | next to the plug-in, `~/.local/bin`, `/usr/local/bin` or the PATH (`radiomoon-mpx-transmitter`) |
| any | the file in the environment variable `RADIOMOON_MPX_TRANSMITTER` |

On a Linux desktop that has only the headless Transmitter, the window is its web page (`http://127.0.0.1:8080`) in the browser. On a server without a desktop (e.g. radiomoon-server on Alpine) there is no window: use the Transmitter's web page from another computer.

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
