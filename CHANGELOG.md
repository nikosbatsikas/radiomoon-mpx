# Changelog

Versions follow MAJOR.MINOR.PATCH. The version lives in `Cargo.toml`. Pushing a new version to `main` creates the tag and the release automatically.

## 1.0.0
- RadioMoon MPX Transmitter and MPX Receiver now have their own repository and releases (they were part of RadioMoon up to 0.5.30). RadioMoon itself no longer contains an MPX generator.
- Transmitter: with an internet stream as the source, the song title sent by the server (Icecast / Shoutcast StreamTitle) becomes the RDS RadioText. This is how RadioMoon's song titles reach RDS: use RadioMoon's internet broadcast as the transmitter's source.
- Everything from RadioMoon 0.5.30: L24 or FLAC lossless over UDP (RTP) or TCP, automatic format detection in the receiver, and phase-aligned concealment of lost packets.
- Windows: own setup (RadioMoon MPX) and portable zip for x64, x86 and arm64. Linux: transmitter and receiver packages for x64 and arm64. macOS: one dmg with both apps.
