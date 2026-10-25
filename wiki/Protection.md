# Protection

Per IP output in the Transmitter, and in Input → *Protection* in the Receiver. Needs 1.10.0 or newer at both ends.

## Password (encryption)
ChaCha20-Poly1305; the key comes from the password (PBKDF2-HMAC-SHA256, 100 000 rounds). Every packet is authenticated, so without the password nobody can listen or send a false stream. A Receiver **with** a password plays only that stream and ignores unencrypted ones; **without** a password it plays only unencrypted streams. Works over UDP and TCP, all formats, with SFN. Cost: 39 bytes per packet (about 0.25 Mbit/s with L24 / FLAC). The header shows **ENCRYPTED**.

## FEC (UDP)
One recovery packet for every 20, 10 or 5 packets (+5 / +10 / +20 %) restores one lost packet of the group. *Burst protection* 2 / 4 / 8 interleaves the groups, so that many packets lost in a row are restored too. Extra delay = n × burst × packet length (shown next to the setting): FLAC 1/10 ≈ +20 ms; Compact Opus 1/5 ≈ +100 ms.

## Allowed senders (Receiver)
Addresses or networks, e.g. `192.168.1.10, 10.0.0.0/8, 2001:db8::/32`; others are refused and counted (see [[Receiver Log]]). Empty = everyone.
