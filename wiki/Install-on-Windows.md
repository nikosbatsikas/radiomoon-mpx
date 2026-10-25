# Install on Windows

Windows 10 or 11. Choose the file for your processor: **x64** (almost all PCs), **arm64** (Windows on ARM, e.g. Surface Pro X / Snapdragon), **x86** (old 32-bit Windows).

## A. With the setup (recommended)

1. Download `RadioMoon-MPX-<ver>-windows-x64-setup.exe` from [Releases](https://github.com/nikosbatsikas/radiomoon-mpx/releases).
2. Double-click it. **Windows SmartScreen** may say *"Windows protected your PC"* (the program is not signed with a paid certificate): click **More info → Run anyway**.
3. **User Account Control:** *Yes* (the setup writes to `C:\Program Files` and makes the shared settings folder `C:\ProgramData\RadioMoon`).
4. **Select Additional Tasks:**
   - *Create a desktop shortcut* – optional.
   - *Install the CLAP plug-in … Common Files\CLAP* – keep it on if you use REAPER, Bitwig, FL Studio, Studio One … ([[CLAP Plug-in]]).
5. **Install → Finish.** In the Start menu: *RadioMoon MPX → RadioMoon MPX Transmitter* and *… Receiver*.

Update: run the setup of the new version over the old one; the settings stay. Remove: *Settings → Apps → RadioMoon MPX → Uninstall* (the settings in `C:\ProgramData\RadioMoon` stay).

## B. Portable (no installation)

1. Download `RadioMoon-MPX-<ver>-windows-x64-portable.zip`, right-click → **Extract All…** to a folder (e.g. a USB stick).
2. Start `RadioMoon-MPX-Transmitter.exe` / `RadioMoon-MPX-Receiver.exe`. The file `portable.flag` makes them keep their settings in that folder.
3. The CLAP plug-in is in the folder `CLAP`.

## First start

- **Windows Defender Firewall:** when the Receiver starts listening, Windows asks *"Allow RadioMoon MPX Receiver to communicate on these networks?"* – tick **Private networks** (and *Public* only if your link uses a network marked public) → **Allow access**. Without it no stream arrives.
  By hand: *Windows Security → Firewall & network protection → Allow an app through firewall → Change settings → Allow another app…* → `C:\Program Files\RadioMoon MPX\RadioMoon-MPX-Receiver.exe`.
- **Sound card at 192 kHz:** *Control Panel → Sound → Playback →* your card *→ Properties → Advanced → Default Format: 24 bit, 192000 Hz* (if offered), and untick *Allow applications to take exclusive control* only if another program must play at the same time. Then choose this card in the Receiver's *Output* tab.
- Continue with [[First Steps]].

## Run as a service (starts with Windows, without login)

In the program: tab **Service, tray & window → Install as service** (asks once for administrator rights) → **Start service**. The window then shows the service's live status; *Restart (apply settings)* after changes of source or outputs. See [[Service and Tray]].

![Service tab](images/screens/receiver-service.png)

## QoS (priority on the network)

Windows clears the DSCP marks unless a policy allows them: [[QoS on Windows]].
