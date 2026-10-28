# WiFi and Internet Links

Most STL links to the sites are WiFi links or the internet: packets get lost (often several in a row), arrive late or in a different order. The right settings:

![SFN network over WiFi and internet](images/sfn-wifi.png)

## Transmitter – the IP output of each such site

| Setting | Value | Why |
|---|---|---|
| Format | **Compact · Opus 256** | about 0.3 Mbit/s instead of 3.7 (FLAC); a lost packet is filled by Opus itself |
| Protocol | **UDP** | SFN needs UDP; TCP waits for every lost packet (pauses) |
| FEC | **1 per 5 packets** | one extra packet per 5 restores a lost one (+20 % bandwidth, still only about 0.35 Mbit/s) |
| Burst protection | **bursts of 2** (WiFi: 4) | also restores 2 (4) packets lost in a row |
| QoS | **EF (46)** | if your WiFi / routers honour DSCP |
| Password | set one | the stream goes over a public network: nobody can listen or send a false stream |

FEC delay = n × burst × 20 ms (Opus packet): 1/5 × 2 = **200 ms**, 1/5 × 4 = 400 ms. It is shown next to the setting.

## Receiver at that site

| Setting | Value |
|---|---|
| Input → Protection → Password | the same password |
| SFN delay (all sites the **same**) | **1500 - 2000 ms** to start with |
| SFN → Reference clock | **Transmitter's clock** (no NTP needed), or GPS with PPS for µs accuracy |
| Without SFN: Output → Buffer | 500 - 1000 ms |

![Delay budget](images/delay.png)

The SFN delay must be longer than: the slowest network path + the FEC delay + the largest jitter + a safety margin. Watch the **Arrival margin** in the SFN tab: it must always stay clearly above 0 (300 ms or more is comfortable). If it gets close to 0, packets come too late (gaps): raise the delay – at **all** sites.

## Testing a bad network on purpose

**Windows – clumsy** (free: <https://jagt.github.io/clumsy/>), run as administrator:
1. Filter: `udp and udp.DstPort == 5004` (the port of the output).
2. Start with *Lag* 50 ms, *Drop* 2 %, *Out of order* 1 % → **Start**.
3. Harder: *Drop* 5 - 10 %, *Lag* 300 ms, *Throttle* for bursts.

**Linux – netem** (on the computer that sends or receives; `lo` when both run on one computer):
```sh
sudo tc qdisc add dev lo root netem delay 80ms 30ms loss 3% 25%   # 80 ms ± 30 ms, 3 % loss in bursts
sudo tc qdisc change dev lo root netem delay 300ms 100ms loss 8% 50%
sudo tc qdisc del dev lo root                                       # back to normal
```

## What to watch

- **Log** (every 10 s): *N packets lost (… ms concealed), M restored by FEC*. Most lost packets should be restored; the rest is concealed.
- **SFN tab:** stays **Locked**; *Restarts* does not climb; *Arrival margin* positive.
- **Link card:** lost packets and *ms hidden*.

What to expect:
- Loss of a few %: with FEC 1/5 × 2 almost nothing is heard.
- A WiFi drop-out longer than about 100 ms cannot be restored: Opus fills it (a short break), the SFN timing stays. Only a gap longer than the SFN delay makes the playout start anew.

## Several sites, different links
Each site is its own IP output with its own format: e.g. FLAC to a fibre site, Compact Opus with FEC to WiFi sites. With SFN all of them use the **same delay** in their Receivers, chosen for the slowest link.
