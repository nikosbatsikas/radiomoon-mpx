# Transmitter

Tabs: **Audio source**, **IP output**, **Sound card output**, **Signal**, **RDS**, **SFN**, **Service**.

## Audio source
- **Internet stream** (Icecast / Shoutcast): the song titles become the RDS RadioText. To put RadioMoon on air, use its internet broadcast URL.
- **Sound input**: any input (44.1 / 48 kHz ...).
- **AES67 / Dante**: RTP L24 / L16, 44.1 / 48 / 96 kHz, multicast or unicast, any channel pair. Announced streams (SAP) are listed. Dante: AES67 mode in Dante Controller, or Dante Virtual Soundcard as a sound input.
- **MPX input (192 kHz)**: a ready composite (e.g. a processor's MPX output) on the left or right channel. PCM / FLAC outputs send it bit-exact; Compact MPX decodes it (audio, levels, RDS) and the receiver rebuilds it.

## IP output
Any number of receivers, each with address, port, UDP / TCP, format, bits, QoS (DSCP) and [[Protection]] (password, FEC). See [[IP Transport and Formats]].

## Signal
Stereo on / off, audio, pilot and RDS levels (1.0 = 75 kHz deviation), pre-emphasis 50 / 75 µs, limiter, output level. The analyzer shows the spectrum (0–96 kHz), the waveform and the pilot / RDS / L+R / L-R levels and the deviation.

## RDS, SFN
See [[RDS]] and [[SFN]].
