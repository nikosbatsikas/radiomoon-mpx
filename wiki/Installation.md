# Installation

Download from [Releases](https://github.com/nikosbatsikas/radiomoon-mpx/releases).

| System | File | Notes |
|---|---|---|
| Windows x64 / x86 / arm64 | `RadioMoon-MPX-<ver>-windows-<arch>-setup.exe` | Both programs; options: desktop icons, CLAP plug-in. Or the portable `...-portable.zip` (settings next to the programs). |
| macOS (Apple Silicon) | `RadioMoon-MPX-<ver>-macos-arm64.dmg` | Both apps and the CLAP plug-in bundle. |
| Linux desktop x64 / arm64 | `RadioMoon-MPX-Transmitter-<ver>-linux-<arch>.tar.gz`, `...-Receiver-...` | Needs a desktop session and `libasound2`. |
| Linux / Raspberry Pi / AlmaLinux without desktop | `RadioMoon-MPX-Headless-linux-<arch>.tar.gz` | Web interface, systemd service: see [[Headless and Web Interface]], [[AlmaLinux]]. |
| Alpine Linux | `RadioMoon-MPX-Headless-alpine-<arch>.tar.gz` | Web interface, OpenRC service: see [[Alpine Linux]]. |

A **192 kHz** sound card is needed for the MPX output (Receiver) and for the MPX input / backup source. The IP side needs no special hardware.

## Settings files
`mpx-transmitter.json` and `mpx-receiver.json` live in `ProgramData\RadioMoon` (Windows), `~/.local/share/RadioMoon` (Linux / macOS desktop), `/var/lib/radiomoon-mpx` (headless editions), or next to the programs with a `portable.flag` file. The headless and desktop editions use the same file format, so a file can be copied between them.
