# Transmitter

The header shows the state (*ON AIR*), Start / Stop and *About*; below it the deviation meter, the overview tiles (deviation peak, IP bandwidth, IP outputs, sound card) and the tabs.

## Audio source
![Audio source](images/screens/transmitter-source.png)

- **Internet stream** (Icecast / Shoutcast URL): the song titles become the RDS RadioText. To put RadioMoon on air, use its internet broadcast.
- **Sound input:** any input at 44.1 / 48 kHz.
- **AES67 / Dante:** RTP L24 / L16 at 44.1 / 48 / 96 kHz, multicast or unicast, any channel pair; announced streams (SAP) appear in a list. Dante: AES67 mode in Dante Controller, or Dante Virtual Soundcard as a sound input.
- **MPX input (192 kHz):** a ready composite (a processor's MPX output) on the left or right channel; PCM / FLAC outputs send it bit-exact, Compact MPX decodes it (audio, levels, RDS).
- **Level** in dB.

## IP outputs
![IP outputs](images/screens/transmitter-ip.png)

Any number of receivers (**+ Add receiver**). Per output: name, address, port, UDP / TCP, format (FLAC, PCM, Compact FLAC, Compact Opus + bit rate), bits, QoS (DSCP), **password**, **FEC** and **burst protection**; live Mbit/s and state. See [[IP Transport and Formats]], [[WiFi and Internet Links]], [[Protection]].

## Sound card output
![Sound card output](images/screens/transmitter-card.png)

The MPX on a local 192 kHz card (e.g. an exciter in the same room).

## Signal
![Signal](images/screens/transmitter-signal.png)

Stereo on / off, audio / pilot / RDS levels (1.0 = 75 kHz deviation), pre-emphasis 50 / 75 µs, limiter, output level. The analyzer below shows spectrum 0 - 96 kHz, waveform, the levels of pilot, RDS, L+R, L-R and the deviation.

## RDS
![RDS](images/screens/transmitter-rds.png)

PI, **ECC**, PTY, RadioText (fixed or from the stream), station name (8 characters, or a longer text in pages / scrolling), TP / TA / Music, **AF list** (type `95.7, 101.5` or `957 1015`, sent on Enter), **clock time** with automatic daylight saving. See [[RDS]].

## SFN
![SFN](images/screens/transmitter-sfn.png)

*Send SFN time stamps* and the reference clock (system / PTP / GPS). See [[SFN]].

## Service, tray & window
![Service](images/screens/transmitter-service.png)

Run as a Windows / Linux service, the tray icon, window size. See [[Service and Tray]].
