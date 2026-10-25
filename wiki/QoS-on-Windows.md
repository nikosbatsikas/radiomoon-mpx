# QoS on Windows

The Transmitter marks every packet of an IP output with a **DSCP** value (IP output → *QoS*, default **EF = 46**), so routers and switches with QoS give the MPX priority. Linux and macOS write the mark as asked. **Windows does not:** it clears the mark of every program unless a **QoS policy** allows it. Without a policy the packets leave with DSCP 0, whatever the program sets.

Make one policy for each program that sends the MPX:

| What sends | Program in the policy |
|---|---|
| Transmitter window or service | `RadioMoon-MPX-Transmitter.exe` (setup: `C:\Program Files\RadioMoon MPX\`) |
| CLAP plug-in | the audio program that loads it, e.g. `reaper.exe`, `Bitwig Studio.exe`, `FL64.exe` |

The Receiver only receives, so it needs no policy (with TCP its acknowledgements are not important).

## Way 1: PowerShell (Windows 10 / 11, Server 2016 and newer)

Open **PowerShell as administrator** and run:

```powershell
New-NetQosPolicy -Name "RadioMoon MPX Transmitter" -AppPathNameMatchCondition "RadioMoon-MPX-Transmitter.exe" -DSCPAction 46 -NetworkProfile All
```

- `-DSCPAction` = the same value as in the Transmitter's *QoS* setting (46 EF, 34 AF41, 40 CS5, 48 CS6).
- Only the MPX and not the program's other traffic: add `-IPProtocolMatchCondition UDP` (or `TCP`), and with a fixed destination port `-IPDstPortMatchCondition 5004`.
- For the CLAP plug-in use the audio program's file name, e.g. `-AppPathNameMatchCondition "reaper.exe"`.

Check: `Get-NetQosPolicy` · remove: `Remove-NetQosPolicy -Name "RadioMoon MPX Transmitter"`.

Policies made with PowerShell do not appear in the Group Policy editor (and the other way round).

## Way 2: Group Policy editor (Windows Pro / Enterprise / Education)

1. Press **Win + R**, type `gpedit.msc`, Enter.
2. Go to **Computer Configuration → Windows Settings → Policy-based QoS**.
3. Right-click **Policy-based QoS → Create new policy…**
4. *Policy name* `RadioMoon MPX Transmitter`; tick **Specify DSCP Value** and enter **46**; leave *Throttle rate* off → **Next**.
5. **Only applications with this executable name:** `RadioMoon-MPX-Transmitter.exe` → **Next**.
6. Source / destination IP: *Any* (or the receivers' addresses) → **Next**.
7. Protocol: **UDP** (or TCP, or *TCP and UDP*); ports: *From any* / *To any* (or the destination port, e.g. 5004) → **Finish**.
8. Apply now: `gpupdate /force` in a command prompt as administrator (or restart).

In a domain, the same policy can be made in a GPO (Group Policy Management) for all transmitter computers.

## Computers not in a domain

On many computers that are not joined to a domain, Windows applies DSCP policies only with this registry value:

1. `regedit` → `HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Services\Tcpip\QoS` (create the key **QoS** if it is missing).
2. New **DWORD (32-bit)** value named `Do not use NLA`, value **1**.
3. Restart the computer.

Or as administrator:

```powershell
New-Item -Path "HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\QoS" -Force | Out-Null
New-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\QoS" -Name "Do not use NLA" -PropertyType DWord -Value 1 -Force
```

## Windows Home

Windows Home has no Group Policy editor: use **Way 1** (PowerShell) and the registry value above. If the marks still do not appear, Windows Home does not apply the policy on that computer; mark the traffic in the router or switch instead (by the Transmitter's address and the destination port).

## Check that it works

- **Wireshark** on the Transmitter computer (or a mirror port of the switch): filter `ip.dsfield.dscp == 46` while the Transmitter sends. The packets to the receiver must appear; without the policy the field shows 0 (*CS0*).
- The Transmitter's IP output shows *QoS EF* next to each output: that is the value the program asks for; Wireshark shows what really leaves the computer.

## The network

DSCP marks only help where the network trusts them: set the switches / routers on the way to **trust DSCP** (or map EF to the priority queue). Internet providers usually clear or ignore the marks; on the internet, [[Protection]] (FEC) and a larger buffer help more. See also [[IP Transport and Formats]].
