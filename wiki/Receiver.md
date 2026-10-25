# Receiver

The **Link** card on top shows bandwidth, format, buffer, lost packets and the history. Tabs: **RDS**, **Input**, **Output**, **SFN**, **Log**, **Service**.

- **Input:** UDP (RTP) or TCP and the port; the format (L16 / L20 / L24, FLAC, Compact MPX) is detected by itself. *Protection*: password and allowed senders ([[Protection]]). *Backup source*: [[Backup Source]].
- **Output:** the 192 kHz sound device into the exciter, and the buffer (default 250 ms).
- **RDS:** station name, RadioText, PI, PTY, TP / TA / music, AF list and clock time, decoded from the received signal.
- **SFN:** playout at the time stamps ([[SFN]]).
- **Log:** stream lost / back, lost packets, refused senders, backup on / off ([[Receiver Log]]).

Header lights: RECEIVING / NO SIGNAL, ON BACKUP, ENCRYPTED, SFN LOCKED, SERVICE.

Lost packets are concealed by repeating the last 2 ms — a whole number of cycles of 19, 38 and 57 kHz, so pilot, stereo and RDS stay in phase.
