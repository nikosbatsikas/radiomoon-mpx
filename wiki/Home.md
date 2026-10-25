# RadioMoon MPX

Two programs for FM broadcasting over IP:

- **RadioMoon MPX Transmitter** – takes audio (internet stream, sound input, AES67 / Dante, or a ready MPX signal), builds the FM multiplex (stereo, 19 kHz pilot, RDS) and sends it to any number of receivers over IP and / or to a 192 kHz sound card.
- **RadioMoon MPX Receiver** – at the transmitter site: receives the stream and plays the MPX on a 192 kHz sound card into the exciter's MPX input.

| Edition | For | Pages |
|---|---|---|
| Desktop (window) | Windows, macOS, Linux desktop | [[Installation]], [[Transmitter]], [[Receiver]] |
| Headless (web interface) | Linux servers, Raspberry Pi, **Alpine Linux** | [[Headless and Web Interface]], [[Alpine Linux]], [[Raspberry Pi]] |
| CLAP plug-in | REAPER, Bitwig, FL Studio, Studio One ... | [[CLAP Plug-in]] |

Topics: [[IP Transport and Formats]] · [[Protection]] (encryption, FEC, allowed senders) · [[SFN]] · [[RDS]] · [[Backup Source]] · [[Receiver Log]] · [[Service and Tray]] · [[Troubleshooting]]

Downloads: [Releases](https://github.com/nikosbatsikas/radiomoon-mpx/releases) · Changes: [CHANGELOG](https://github.com/nikosbatsikas/radiomoon-mpx/blob/main/CHANGELOG.md)
