# First Steps

A first link: the Transmitter on the studio PC sends to the Receiver at the transmitter site (for a test both can run on **one** computer: address `127.0.0.1`).

## 1. Receiver (site)

1. Start **RadioMoon MPX Receiver**.
2. Tab **Input:** *UDP (RTP)*, port **5004** (or another free port).
3. Tab **Output:** the 192 kHz sound card that feeds the exciter's MPX input (for a test without card: *No sound card (test output)*). Buffer 250 ms is fine on a LAN; use 500 - 1000 ms over the internet.
4. **▶ Start.** The header shows *NO SIGNAL* until a stream arrives.

![Receiver – Input](images/screens/receiver-input.png)

## 2. Transmitter (studio)

1. Start **RadioMoon MPX Transmitter**.
2. Tab **Audio source:** an internet stream (e.g. your Icecast URL – the song titles become the RadioText), a sound input, AES67 / Dante, or a ready MPX input.
3. Tab **IP outputs → + Add receiver:** the site's **address** and **port 5004**, protocol **UDP**, format:
   - LAN / fibre: **FLAC lossless** (about 3.7 Mbit/s, bit-exact composite)
   - WiFi / internet: **Compact · Opus 256** (about 0.3 Mbit/s) with **FEC 1 per 5, bursts of 2** – see [[WiFi and Internet Links]]
   - optional **Password** (the Receiver needs the same one: tab Input → Protection)
4. Tab **RDS:** PI code, station name (PS), RadioText, AF list, ECC.
5. **▶ Start.**

![Transmitter – IP outputs](images/screens/transmitter-ip.png)

## 3. Check

- Receiver header: **RECEIVING**; the Link card shows format, Mbit/s, buffer and lost packets; the RDS tab the station name.
- The analyzer shows the pilot (19 kHz), the stereo band (23 - 53 kHz) and RDS (57 kHz); deviation peak below 100 % (75 kHz).
- Tab **Log**: *Stream from …*.

![Receiver – RDS](images/screens/receiver-rds.png)

## 4. For unattended use

- Both programs: *Start … when the program starts* and **Service, tray & window → Install as service** ([[Service and Tray]]).
- Servers without desktop: the [[Headless and Web Interface]] editions.
