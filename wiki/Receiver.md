# Receiver

On top the **Link** card: bandwidth, format, buffer, lost packets (with the ms hidden), data received, the bandwidth history, the sender's address and the sound card state. Header lights: *RECEIVING / NO SIGNAL*, *ON BACKUP*, *ENCRYPTED*, *SFN LOCKED*, *SERVICE*.

## RDS
![RDS](images/screens/receiver-rds.png)

Station name, RadioText, PI, ECC, PTY, TP / TA / music, AF list and clock time, decoded from the received signal; the analyzer below.

## Input
![Input](images/screens/receiver-input.png)

- UDP (RTP) or TCP and the port; the format is detected by itself.
- **Protection:** password of an encrypted stream, *Allowed senders* ([[Protection]]).
- **Backup source:** MPX from a sound card input when the stream stops ([[Backup Source]]).

## Output
![Output](images/screens/receiver-output.png)

The 192 kHz sound device into the exciter (or *No sound card (test output)*), the buffer, *Start receiving when the program starts*.

## SFN
![SFN](images/screens/receiver-sfn.png)

Playout at time stamp + delay, output correction, live: state, timing error, rate trim, arrival margin, stamp jitter, card jitter, restarts; the reference clock. See [[SFN]].

## Log
![Log](images/screens/receiver-log.png)

Stream lost / back, lost and restored packets, refused senders, backup, SFN lock and restarts, clock steps, sound card ([[Receiver Log]]). *Copy* puts it on the clipboard.

## Service, tray & window
![Service](images/screens/receiver-service.png)

Lost packets are concealed by repeating the last 2 ms (a whole number of cycles of 19, 38 and 57 kHz: pilot, stereo and RDS stay in phase).
