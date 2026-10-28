# IP Transport and Formats

| Format | Bandwidth* | Quality |
|---|---|---|
| L24 / L20 / L16 uncompressed | 4.9 / 4.0 / 3.3 Mbit/s | 24 / 20 / 16-bit, 192 kHz |
| FLAC 24 / 20 / 16-bit | about 3.7 / 3.0 / 2.2 Mbit/s | bit-exact |
| Compact · FLAC | about 0.95 Mbit/s | lossless audio 16-bit 48 kHz; the receiver rebuilds the composite |
| Compact · Opus 320 / 256 / 192 / 128 | about 0.35 / 0.28 / 0.21 / 0.15 Mbit/s | 256 is transparent |
| AM · FLAC / AM · Opus 96 | about 0.3 / 0.1 Mbit/s | mono audio for an AM transmitter, see [[AM Mode]] |

\* including IP/UDP headers. 16 and 20 bits use TPDF dither.

**Protocols:** UDP / RTP (payload types L24 97, FLAC 98, L16 99, L20 100, Compact 101) or TCP (stream starts `RMF1` FLAC, `RMP1`+bits PCM, `RMC1` Compact; 24-bit PCM raw). Protected streams: see [[Protection]].

**Compact MPX** is like μMPX but for RadioMoon only (not compatible with Telos / Omnia μMPX): the processed audio plus levels and RDS; the receiver uses the same stereo encoder and RDS generator. Use PCM / FLAC when the exact composite must arrive (e.g. after an external composite clipper).

**QoS:** DSCP EF (46, default), AF41 (34), CS5, CS6, off, or 0–63. Windows needs a policy-based QoS rule for the program to mark packets: see [[QoS on Windows]].

**Ports / firewall:** open the chosen UDP or TCP port at the Receiver (default 5004).
