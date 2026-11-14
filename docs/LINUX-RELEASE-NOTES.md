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

## Credits

Linux port by YoruRaven. Based on the original
[AirFlash](https://github.com/Ding-Kyoma/AirFlash) by Ding-Kyoma.

License: GPLv3 or later, or commercial license · .NET / Avalonia / Rust
