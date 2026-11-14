# AirFlash 0.6.3 · Linux

Linux x86_64 and arm64 (aarch64) · English and Simplified Chinese · AirPlay 2 sender

This is the first Linux release built on the merged upstream 0.6.3 codebase.
It streams Linux desktop audio (PipeWire system capture, with a PulseAudio
fallback) to an AirPlay 2 receiver or an existing HomePod stereo pair.
Includes support for the latest HomePod OS 26 (AirPlay 2).

## What is in this release

Every asset is built for both **x86_64** and **arm64/aarch64** by the
`Linux Release` GitHub Actions workflow, using
`packaging/linux/appimage/build-appimage.sh`:

| Asset | Variant | Contains |
| --- | --- | --- |
| `AirFlash-0.6.3-<arch>.AppImage` | `backend` | `airflash-cli` + the JSONL `airflash-engine` (headless) |
| `AirFlash-0.6.3-<arch>-full.AppImage` | `full` | the above plus the Avalonia control panel |
| `AirFlash-0.6.3-<arch>.tar.gz` | portable tree | the full AppDir as plain files (see the PTP note) |
| `AirFlash-0.6.3-<arch>-cli.tar.gz` | headless CLI | `airflash-engine` + `airflash-cli`, systemd user units and config templates; built on Ubuntu 22.04 (glibc 2.35) so it also runs on Raspberry Pi OS Bookworm — see `docs/HEADLESS.md` |
| `SHA256SUMS.txt` | — | checksums for every asset above |

## Timing note: PTP and the AppImage

AirPlay 2 group playback prefers PTP timing, which needs to bind UDP
319/320 — privileged ports on Linux. File capabilities cannot survive an
AppImage mount, so inside an AppImage the engine detects the missing
permission and falls back to NTP timing automatically (with a warning in
the log) instead of failing the stream. For true PTP sync, use the
portable tarball (or a plain directory install) and grant the engine
binary the capability once:

```sh
sudo setcap cap_net_bind_service=+ep /path/to/airflash-engine
```

## Since the 0.4.x Linux previews

- Merged the upstream 0.6.3 engine and core (adaptive latency, receiver
  ownership/preemption handling, standby on silence, group timing plans).
- Linux live sources (system capture, WAV file, simulated) work through
  the new session supervisor; the settings window switches pages
  correctly, error text is selectable/copyable, and `--tray` autostart
  stays in the tray.
- The update checker and download links now point at this repository
  (yoruraven/AirFlash-Linux).
- Capture is no longer limited to the desktop: any PipeWire source — a
  microphone, a USB audio input, a Bluetooth A2DP source — can be recorded
  directly. `airflash-cli devices` lists the capture devices; pick one
  with `--monitor NAME` or with the new capture-device picker in
  Settings → Audio source. A phone paired over Bluetooth turns the Linux
  machine into a Bluetooth-to-AirPlay bridge for the HomePod.
- New headless `-cli.tar.gz` packages for both architectures, built
  against glibc 2.35 so they run on older userlands such as Raspberry Pi
  OS Bookworm, with a headless guide (`docs/HEADLESS.md`).

## Credits

Linux port by YoruRaven. Based on the original
[AirFlash](https://github.com/Ding-Kyoma/AirFlash) by Ding-Kyoma.

License: GPLv3 or later, or commercial license · .NET / Avalonia / Rust
