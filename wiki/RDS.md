# RDS

Transmitter, RDS tab:
- **PI**, **PTY**, **TP / TA / Music**.
- **ECC (Extended Country Code)**, hex, sent in group 1A about every 3.5 s: with the first digit of the PI it names the country world-wide (Greece **E1** with PI 1xxx, Cyprus **E2** with PI 2xxx). Cars with hybrid radio (RadioDNS) need it, see [[Station Logos in Cars]]. Empty = not sent. The Receiver shows the received ECC next to the PI.
- **Station name (PS):** 8 characters, or a long text shown *in pages* (whole words) or *scrolling*, with seconds per page. A complete name is always sent before the next page.
- **RadioText:** fixed, or from the stream's song titles.
- **Alternative frequencies (AF):** up to 25 frequencies (87.6–107.9 MHz, method A). Type `94.3, 101.5` or without the dot `943 1015`; the list is sent when you press Enter.
- **Clock time (CT):** once a minute, with the local offset; *Daylight saving* Automatic (Europe), Automatic (USA / Canada) or Off. With automatic, set the winter offset (Greece +2).

The Receiver shows the decoded RDS; the station name only when it came complete.

Logos in cars (Android Auto, CarPlay, hybrid radio) do not come from RDS: see [[Station Logos in Cars]].
