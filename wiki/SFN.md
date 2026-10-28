# SFN (single frequency network)

Several transmitters on the same frequency must put the same sample on air at the same moment. The Transmitter stamps every packet with the UTC time of its first sample; every Receiver plays it at *stamp + delay* by its reference clock.

![SFN network](images/sfn-wifi.png)

![Delay budget](images/delay.png)

1. **Transmitter**, SFN tab: *Send SFN time stamps*, choose the reference clock. Every UDP packet carries the UTC time of its first sample (RTP header extension, profile 0x524D).
2. **Every Receiver**, SFN tab: *Play every sample at its time stamp + delay*, the same reference clock and the **same delay** at all sites (longer than the slowest path; see *Arrival margin*). Calibrate each site's *Output correction* once (sound card, exciter, cables).

**Reference clocks:** *System clock* (best when the computer is synchronised by PTP — ptp4l + phc2sys — or GPS PPS with gpsd + chrony), the built-in *PTP* client (IEEE 1588v2 default profile, ports 319 / 320, about ±10–100 µs), or a *GPS receiver* on a serial port (NMEA; with PPS on DCD about ±0.1–1 ms).

**Receiver: *Transmitter's clock*** (from 1.13.0) — no NTP, PTP or GPS needed. The Transmitter sends a small time packet to each Receiver once a second on the same UDP path as the stream, and the Receiver answers; the Receiver then plays by the Transmitter's clock. All sites follow one clock, so computers whose clocks differ by many ms (the warning *the Transmitter's clock is about … ms ahead*) play together. It works through NAT, needs no extra port and is protected by the stream password. Accuracy: half the difference between the two directions of the path — some tens of µs on a LAN, often 0.1 – 2 ms over WiFi or the internet. The Transmitter shows for each IP output *Time: the Receiver follows this clock (round trip … ms)*.

| Reference clock | Typical accuracy between sites | Needs |
|---|---|---|
| Transmitter's clock (Receiver) | LAN: tens of µs; WiFi / internet: 0.1 – 2 ms | nothing (1.13.0 at both ends) |
| System clock with NTP | 1 – 20 ms | a time service at every site |
| PTP, built in | ±10 – 100 µs | a PTP grandmaster in the network |
| GPS with PPS (system clock via gpsd + chrony) | ~1 µs | a GPS receiver at every site |

For a real SFN overlap area (a few µs) use GPS with PPS at every site; the Transmitter's clock is the easy choice for links where ms are enough, or to start.

UDP only. For WiFi / internet links see [[WiFi and Internet Links]].

![Receiver SFN tab](images/screens/receiver-sfn.png)

UDP only. The Receiver follows the stamps with a fine resampler, so its sound card clock does not matter. The [[Backup Source]] plays without time stamps.

## Check
- **Locked** (green): the timing error stays within a few µs.
- *Not playing: …*: the sound card does not play (it must support **192 kHz**). For a test without such a card choose the output *No sound card (test output)*.
- *Arrival margin longer than the delay*: the Transmitter's clock runs ahead of the Receiver's — synchronise the clocks (NTP / chrony at least; GPS PPS or PTP for real SFN).
- *Late*: the packets arrive after their playout time — increase the delay.
- *The clock was stepped …*: a time service sets the clock in steps (e.g. the Windows time service). Every step moves this site against the others; use a service that adjusts smoothly (chrony, ptp4l + phc2sys, Meinberg NTP on Windows) or GPS / PTP as the reference clock.
- *Restarts* with *Last restart: the playout was … ms off*: the timing jumped by more than 20 ms (a clock step, the sound card stopped, or a very busy computer).
