# AirFlash headless (CLI) guide

The `AirFlash-<version>-<arch>-cli.tar.gz` release asset is the headless
package: just `airflash-engine` + `airflash-cli`, no desktop and no
AppImage. It runs on any 64-bit Linux with glibc 2.35 or newer (Ubuntu
22.04+, Debian 12 "Bookworm", Raspberry Pi OS Bookworm or later) — a
Raspberry Pi is a first-class target.

## What's in the package

- `airflash-engine`, `airflash-cli` — the sender and its control surface
- `airflash-cli.service`, `airflash-cli@.service` — systemd user units
- `cli.json.example`, `cli.env.example` — configuration templates
- `README.md` — this guide

## Install

```sh
tar -xzf AirFlash-<version>-<arch>-cli.tar.gz
cd AirFlash-<version>-<arch>-cli
sudo install -m755 airflash-engine airflash-cli /usr/bin/
```

Capture needs PipeWire (`pw-record`, `pw-link`, `pw-dump`) or PulseAudio
(`parec`, `pactl`) in `PATH`, and the receiver must be reachable on the
local network (mDNS/AirPlay).

For precise PTP timing the sender binds UDP 319/320, which are privileged
ports. Grant the capability once:

```sh
sudo setcap cap_net_bind_service=+ep /usr/bin/airflash-engine
sudo setcap cap_net_bind_service=+ep /usr/bin/airflash-cli
```

or enable the `AmbientCapabilities` lines in the systemd unit instead.
Without it the engine warns and falls back to NTP timing automatically.

## First run

```sh
airflash-cli discover                       # find receivers
airflash-cli devices                        # list capture devices
airflash-cli start --device "Living Room" --daemon
airflash-cli status
airflash-cli stop
```

If a receiver asks for a PIN or password, pair once with
`airflash-cli pair --host <ip>`; credentials are stored per receiver.

## Capture devices: more than the desktop

`--monitor NAME` (or `"monitor"` in the config file) accepts any device
from `airflash-cli devices`, addressed by node name, description or id:

- A **sink** is recorded through its monitor — the classic system audio.
- A **source** is recorded directly. In PipeWire terms that includes:
  - a **microphone** — announcements, a scanner feed, a baby monitor;
  - a **USB audio input** — a USB turntable, a mixer, any line-level
    source through a USB sound adapter becomes an AirPlay source;
  - a **Bluetooth A2DP source** — pair a phone with this machine
    (BlueZ + PipeWire) and it shows up as a source: the Linux box becomes
    a Bluetooth-to-AirPlay bridge and gives the HomePod a Bluetooth input.

Without `--monitor`, the default sink's monitor is recorded.

## Running as a service: the Raspberry Pi music streamer

```sh
mkdir -p ~/.config/airflash ~/.config/systemd/user
cp cli.json.example ~/.config/airflash/cli.json   # set host / monitor / …
cp airflash-cli.service ~/.config/systemd/user/
systemctl --user enable --now airflash-cli.service
```

Point any player on the box (MPD, mopidy, a browser, …) at its default
output and the monitor is streamed: the Pi is now an always-on AirPlay
sender for the HomePod. The template unit (`airflash-cli@<name>.service`
with `cli-<name>.json`) serves several receivers at once.

Notes:

- USB and Bluetooth sources appear and disappear as devices connect. A
  session whose source vanishes ends; start it again when the device is
  back (or let the unit's restart policy retry).
- AirPlay adds a few hundred milliseconds of latency. That is fine for
  music and announcements, not for live monitoring; Bluetooth forwarding
  adds its own latency on top, so it is not suited to video lip-sync.
