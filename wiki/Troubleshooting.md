# Troubleshooting

| Problem | What to check |
|---|---|
| Receiver: NO SIGNAL | Port and protocol the same at both ends; firewall; *Allowed senders*; the [[Receiver Log]] |
| "An encrypted stream arrives" | Enter the stream's password in Input → Protection |
| "Packets rejected" | The password differs (case matters) |
| "Unencrypted stream ignored" | A password is set at the Receiver but not at the Transmitter's output |
| Lost packets | Use FEC (UDP) or TCP; a larger buffer; QoS on the network |
| Sound card errors | The card must support 192 kHz; on Linux check `aplay -l`, the user in group `audio` |
| SFN not locked, "Waiting for time stamps" / "Not playing" | The Receiver's sound card must play 192 kHz (Output tab); for a test choose *No sound card (test output)*. Same reference clock and delay at all sites; *Arrival margin* positive and smaller than the delay |
| Web page: no login | Headless: `radiomoon-mpx-rxd --data /var/lib/radiomoon-mpx --set-login admin` (as the service user), then restart |
| Windows: QoS not set (Wireshark shows DSCP 0) | A policy-based QoS rule for the program, and on computers outside a domain the registry value "Do not use NLA": see [[QoS on Windows]] |
