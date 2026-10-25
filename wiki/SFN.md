# SFN (single frequency network)

1. **Transmitter**, SFN tab: *Send SFN time stamps*, choose the reference clock. Every UDP packet carries the UTC time of its first sample (RTP header extension, profile 0x524D).
2. **Every Receiver**, SFN tab: *Play every sample at its time stamp + delay*, the same reference clock and the **same delay** at all sites (longer than the slowest path; see *Arrival margin*). Calibrate each site's *Output correction* once (sound card, exciter, cables).

**Reference clocks:** *System clock* (best when the computer is synchronised by PTP — ptp4l + phc2sys — or GPS PPS with gpsd + chrony), the built-in *PTP* client (IEEE 1588v2 default profile, ports 319 / 320, about ±10–100 µs), or a *GPS receiver* on a serial port (NMEA; with PPS on DCD about ±0.1–1 ms).

UDP only. The Receiver follows the stamps with a fine resampler, so its sound card clock does not matter. The [[Backup Source]] plays without time stamps.
