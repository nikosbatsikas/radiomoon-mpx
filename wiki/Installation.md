# Installation

Download from [Releases](https://github.com/nikosbatsikas/radiomoon-mpx/releases) (the newest is at the top) and follow the page for your system:

| System | File | Step by step |
|---|---|---|
| Windows 10 / 11 (x64, x86, arm64) | `RadioMoon-MPX-<ver>-windows-<arch>-setup.exe` or `...-portable.zip` | [[Install on Windows]] |
| macOS 11+ (Apple Silicon) | `RadioMoon-MPX-<ver>-macos-arm64.dmg` | [[Install on macOS]] |
| Linux with a desktop (x64, arm64) | `RadioMoon-MPX-Transmitter-<ver>-linux-<arch>.tar.gz`, `...-Receiver-...` | [[Install on Linux Desktop]] |
| Linux server / Raspberry Pi without desktop | `RadioMoon-MPX-Headless-linux-<arch>.tar.gz` (installer downloads it) | [[Headless and Web Interface]], [[Raspberry Pi]] |
| Alpine Linux | `RadioMoon-MPX-Headless-alpine-<arch>.tar.gz` (installer downloads it) | [[Alpine Linux]] |
| AlmaLinux / Rocky / RHEL / Fedora | `RadioMoon-MPX-Headless-linux-<arch>.tar.gz` (installer downloads it) | [[AlmaLinux]] |
| CLAP plug-in | in the Windows setup, the zip, the Linux transmitter package and the dmg | [[CLAP Plug-in]] |

`<arch>`: **x64** = normal PCs (Intel / AMD), **x86** = old 32-bit Windows, **arm64** = Raspberry Pi 3 / 4 / 5 (64-bit system), Windows on ARM, Apple Silicon.

## What you need
- **Transmitter:** any PC; a sound card only for a sound input or the MPX sound card output.
- **Receiver:** a sound card that plays **192 kHz** into the exciter's MPX input (most USB / PCIe pro cards; many built-in cards do only 48 / 96 kHz). For tests without such a card choose the output *No sound card (test output)*.
- **Network:** the Receiver's UDP (or TCP) port open, default **5004**. Bandwidth: see [[IP Transport and Formats]] (Compact Opus 0.15 - 0.35 Mbit/s, FLAC about 3.7 Mbit/s).

## Where the settings are
| Edition | Folder |
|---|---|
| Windows (setup) | `C:\ProgramData\RadioMoon` |
| Windows (portable zip) | next to the programs (`portable.flag`) |
| macOS / Linux desktop | `~/.local/share/RadioMoon` |
| Headless editions | `/var/lib/radiomoon-mpx` |

Files: `mpx-transmitter.json`, `mpx-receiver.json` (same format in all editions: a file can be copied from one to another), `web-login.json` (headless).
