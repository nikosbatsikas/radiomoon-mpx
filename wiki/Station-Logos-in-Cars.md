# Station Logos in Cars

**RDS cannot send a logo.** RDS carries about 1.2 kbit/s of text and codes (station name, RadioText, PI, PTY, AF, clock). The logos that cars show come from the **internet**, found with your station's codes or names. Where the logo comes from depends on what the car plays.

## 1. Android Auto and Apple CarPlay

They do **not** use the car's FM tuner: they show the radio **apps** of the phone (TuneIn, radio.net, Simple Radio, myTuner ...). The logo comes from each app's station directory.

What to do: list the station — name, logo, stream URL, genre, city — in these directories. Most have a free "add your station" form for broadcasters. Use the stream of the Transmitter's source (e.g. RadioMoon's Icecast) so the app plays the same programme.

## 2. The car's own FM radio ("hybrid radio")

Many newer cars look the station up online by its RDS codes and download the logo, sometimes also the programme and the song cover.

### RadioDNS (many European carmakers)
1. The car builds a name from **frequency**, **PI** and **country (ECC)**:
   `<frequency>.<pi>.<gcc>.fm.radiodns.org`
   - frequency without the dot, 5 digits: 95.7 MHz → `09570`, 106.2 → `10620`
   - PI in lower case: `1a2b`
   - GCC = first character of the PI + the ECC: Greece (PI starts with 1, ECC E1) → `1e1`
   - example: `09570.1a2b.1e1.fm.radiodns.org`
2. That name points (DNS CNAME) to the broadcaster's domain; an SRV record `_radioepg._tcp.<domain>` gives the metadata server.
3. The car reads `http://<metadata server>/radiodns/spi/3.1/SI.xml`: the station (bearer `fm:1e1.1a2b.09570`) with its names and logos (`<mediaDescription>`), usually 32×32, 112×32, 128×128, 320×240 and 600×600 pixels (PNG / JPG).

What to do: join RadioDNS, publish `SI.xml` and the logo files on your web server, add the DNS records. See the RadioDNS guides below.

### DTS AutoStage (Xperi; e.g. Hyundai, Mercedes and others)
Xperi keeps its own station database; broadcasters give their station data and logos to DTS AutoStage (broadcaster portal / contact).

### Correct codes are needed
Both find the station by its **PI code**, RadioDNS also by the **ECC**:
- **PI:** your real, unique code (in Greece from the regulator, EETT) — not the default `1234`. Transmitter → RDS tab → *PI code*.
- **ECC:** the extended country code (RDS group 1A); Greece = **E1**. Transmitter → RDS → *ECC* (from version 1.12.0); the Receiver shows it next to the PI, so you can check that it is on air.
- Keep the **AF list** complete: the car then knows all frequencies of the station.

## 3. RDS2: a logo over the air

RDS2 (IEC 62106:2018) adds three data subcarriers (about 66.5, 71.25 and 76 kHz) and a file transfer (RFT) that can carry a small image such as the station logo. **Almost no radios or cars receive it yet** — chip makers wait for demand, and existing receivers would need new firmware. RadioMoon MPX does not send RDS2.

## Summary

| Where the logo appears | Source | What the station does |
|---|---|---|
| Android Auto / CarPlay | the phone's radio apps | list the station in the app directories |
| Car FM radio with hybrid radio | RadioDNS, DTS AutoStage | correct PI + ECC, register, publish SI.xml + logos |
| Over the air | RDS2 (rare receivers) | — (not practical today) |

## Sources
- [RadioDNS: finding station logos, step by step](https://radiodns.org/?p=4065)
- [RadioDNS: how to register your station for hybrid radio (PDF)](https://radiodns.org/wp-content/uploads/2019/08/HOWTO-Register-your-station-for-RadioDNS-Hybrid-Radio.pdf)
- [RadioDNS – Wikipedia](https://en.wikipedia.org/wiki/RadioDNS)
- [WorldDAB: station logos (PDF)](https://www.worlddab.org/public_document/file/1091/StationLogosDoc_FINAL_2019.pdf)
- [DTS AutoStage](https://dts.com/?p=6220)
- [Radio World: what's new with RDS2](https://www.radioworld.com/tech-and-gear/whats-new-with-rds2)
- [RDS Forum: RDS2](https://rds.org.uk/pages/rds2.html)
