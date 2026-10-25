# Install on macOS

macOS 11 or newer on Apple Silicon (M1 / M2 / M3 / M4).

1. Download `RadioMoon-MPX-<ver>-macos-arm64.dmg` from [Releases](https://github.com/nikosbatsikas/radiomoon-mpx/releases) and open it.
2. Drag **RadioMoon MPX Transmitter** and **RadioMoon MPX Receiver** to **Applications**.
3. First start: the apps are not notarised by Apple, so macOS says *"… cannot be opened because Apple cannot check it for malicious software"*.
   - **System Settings → Privacy & Security →** scroll down → *"RadioMoon MPX … was blocked"* → **Open Anyway** → confirm with your password. Needed once per app.
   - Or in Terminal: `xattr -dr com.apple.quarantine "/Applications/RadioMoon MPX Receiver.app"` (and the same for the Transmitter).
4. **Microphone access:** when the Transmitter records a sound input (or the Receiver a backup source) macOS asks *"… would like to access the microphone"* → **Allow**. Later: *System Settings → Privacy & Security → Microphone*.
5. **Firewall** (if switched on): *"Do you want the application … to accept incoming network connections?"* → **Allow** (needed for the Receiver).

## Sound card at 192 kHz
*Applications → Utilities → Audio MIDI Setup →* your card → *Format: 192 000 Hz*. Then choose the card in the Receiver's *Output* tab.

## CLAP plug-in
Copy **RadioMoon MPX Transmitter.clap** from the dmg to `~/Library/Audio/Plug-Ins/CLAP` (Finder: *Go → Go to Folder…*; make the folder if it is missing), then let your audio program rescan. See [[CLAP Plug-in]].

Continue with [[First Steps]].
