# Receiver Log

The Receiver keeps the last 300 events: in the window's **Log** tab (UTC, *Copy* button), on the web page's **Log** tab (local time of the browser), and — headless — in the system log (`/var/log/radiomoon-mpx/rxd.log` on Alpine, `journalctl -u radiomoon-mpx-rxd` with systemd).

Events:
- Receiver started; settings changed (restarted)
- Stream from *address*: *format* (with the time without stream); stream now from another address / format
- **Stream lost**: no packets for 1 s
- *N* packets lost (*ms* concealed), *M* restored by FEC — summed up every 10 s
- packets rejected (wrong password), unencrypted packets ignored, encrypted stream without password, blocked senders
- **Backup on air** / stream back on air
- SFN locked / not locked; sound card state; TCP connection from / closed
