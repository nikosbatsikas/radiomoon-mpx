# AM Mode

RadioMoon MPX can also feed **AM transmitters** (medium wave, long wave, short wave). An AM exciter takes plain **mono audio**, not the FM multiplex; the Transmitter makes this audio for every IP output set to an AM format, next to its FM outputs.

## The Transmitter

Tab **IP outputs** → the output of the AM site → **Format**:

| Format | Bandwidth | Use |
|---|---|---|
| **AM · FLAC** | about 0.3 Mbit/s | lossless, LAN / good links |
| **AM · Opus** 96 kbit/s | about 0.1 Mbit/s | WiFi / internet (64 kbit/s on very weak links) |

Then for the AM output:

| Setting | Value |
|---|---|
| **AM bandwidth** | **4.5 kHz** for 9 kHz channel spacing (Europe, Africa, Asia) · 5 kHz · 9 kHz · **10 kHz (NRSC)** for 10 kHz spacing (America) |
| **+peak** (positive peak) | **125 %** usual (100 - 150 %); negative peaks are always held at **-100 %** |

What the Transmitter does with the audio:
1. mono: (left + right) / 2,
2. takes the FM pre-emphasis out again (the shared processing is made for FM),
3. high-pass at 50 Hz: no DC or rumble on the carrier,
4. steep low-pass (8th order) at the AM bandwidth: the signal stays inside the channel,
5. asymmetric peak limiter and clipper: never below -100 % (the carrier would cut off and splatter), positive peaks up to the set limit.

The output line shows the program's modulation peaks, e.g. *AM peaks -100 % / +123 %*.

The **Signal** tab (stereo, pilot, RDS) does not apply to AM outputs; audio source, level and limiter do.

## The Receiver

Nothing to set: the Receiver (1.14.0 or newer) sees *AM audio (FLAC / Opus)* and plays it on its **Output** sound card. Connect the card to the **audio input** of the AM exciter (not an MPX input).

Level: **-100 % modulation = -0.8 of full scale**, +125 % = full scale. Set the exciter's audio input once with a tone: e.g. a sine at 90 % modulation should read 90 % on the exciter's modulation meter.

The sound card must play 192 kHz (as for FM), because the Receiver's output chain works at 192 kHz.

## Everything else works as for FM

UDP or TCP, [[Protection]] (password, FEC, allowed senders), [[Backup Source]] (an off-air or analog backup), the [[Receiver Log]], and [[SFN]]:
**Synchronous AM** (several AM transmitters on one frequency) needs far less timing accuracy than FM — the *Transmitter's clock* reference over the stream link is usually enough. Use the same SFN delay at all sites.

## FM and AM from one Transmitter

Each IP output has its own format: FLAC or Compact Opus to the FM sites, AM · Opus to the AM sites — all from the same program at the same time.
