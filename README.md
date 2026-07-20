# spotui-hiby-r3proii

SpotUI for the HiBy R3 Pro II: an experimental standalone, tetherless streaming UI/client with on-device control.

> Early developer preview source release. This is not yet a one-click install or end-user firmware package.

[![Support me on Ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/noisetta)

Optional donations support test hardware, documentation, maintenance, and continued experimentation. Nothing is paywalled.

## Screenshots

<p>
  <img src="docs/images/spotui-interface.png" alt="SpotUI interface screenshot" width="360">
</p>

This project provides source code, scripts, and notes for running a lightweight standalone music client on HiBy devices. It is intended for device owners who want to build, study, and modify their own hardware.

This project is not affiliated with, endorsed by, or supported by HiBy Music or Spotify.

## Status

Experimental. Tested primarily on the HiBy R3 Pro II.

Current device-side features include:

- On-device browsing and queued playback of Liked Songs and playlists
- On-device track search with responsive, tap-safe result loading
- Queue-aware Previous, Next, and Up Next views
- Dedicated Now Playing screen with larger metadata, progress seeking,
  playback controls, and direct Up Next access
- Persistent shuffle, Repeat Off, Repeat All, and Repeat One modes
- Pause, resume, and touchscreen progress seeking
- Fixed bottom toolbar with Exit, Brightness, Pause/Resume, and navigation controls
- Persistent brightness selection
- Persistent screen-sleep settings for 30 seconds, 60 seconds, 2 minutes,
  5 minutes, or Never, with safe touch wake and saved-brightness restore
- Battery percentage display
- Automatic 3.5 mm and 4.4 mm output routing
- Automatic pause when the active headphone output is disconnected
- Header-based paging through track, playlist, search, and queue lists
- Track-name truncation for the compact display
- Ten appearance themes with performance-aware ambient animation
- A staged WiFi, Spotify, and library loading screen
- Supervised playback recovery and reconnect feedback
- Live diagnostics for WiFi, Spotify, audio, output, and queue state

“Tetherless” means playback can be browsed and controlled directly from the HiBy instead of using it only as a receiver controlled by a phone or desktop client.

SpotUI is launched manually from its repurposed stock tile. On a cold boot,
the stock player may remain visible for tens of seconds while its audio
hardware finishes initializing; SpotUI's loading page appears after the safe
handoff begins. This delay is intentional because stopping the stock player
too early can leave the headphone outputs silent until reboot. Automatic
takeover is not enabled, so normal use of the stock player remains available.

Flashing or modifying firmware can brick your device. Use at your own risk.

## What is included

- `engine/ui/` — framebuffer and touchscreen UI written in Rust using embedded-graphics.
- `engine/launcher/` — launcher script for WiFi bring-up, jack routing, UI startup, daemon supervision, and panel keepalive behavior.
- `engine/firmware/` — init scripts and firmware-side integration notes.
- `apps/spotify/daemon/` — Spotify-compatible daemon source using librespot.

## What is not included

This repository does not include:

- HiBy firmware images
- modified `.upt` firmware files
- extracted HiBy binaries
- deployed device snapshots
- Spotify credentials
- `librespot-cache/`
- WiFi credentials
- user-specific device backups
- ready-to-flash firmware builds

This source repository does not include firmware images, ready-to-flash builds, Spotify credentials, WiFi credentials, or user-specific device files. Users are responsible for any firmware, accounts, credentials, and device setup used with their own hardware.

## Repository structure

- `engine/` — reusable core components.
  - `ui/` — framebuffer/touch UI for the device.
  - `firmware/` — init scripts and firmware integration pieces.
  - `launcher/` — startup script for launching the UI and backend daemon.
- `apps/spotify/` — Spotify-compatible app built on top of the core engine.
  - `daemon/` — librespot-based control daemon source.
- `docs/` — setup, build, recovery, and device notes.

## Setup requirements

- A HiBy R3 Pro II device.
- A working ADB connection.
- A local MIPS cross-build environment for `mipsel-unknown-linux-musl`.
- A user-supplied WiFi configuration on the device.
- A user-supplied Spotify Premium account configured locally on the device.
- The HiBy backlight setting should be configured so the panel remains available during startup.

## Spotify note

SpotUI does not use the Spotify Web API. It uses librespot, an open-source Spotify Connect client library, for Spotify-compatible playback.

The current playback login can read Liked Songs but cannot modify the user's
Spotify library. Adding or removing liked tracks would require a separate
Spotify Web API OAuth authorization and is not currently supported.

Do not commit Spotify credentials, cache files, tokens, WiFi credentials, firmware images, or device snapshots to this repository.

## Disclaimer

This is an independent community research/modding project. It is provided without warranty. You are responsible for your own device, accounts, firmware, and compliance with applicable laws and service terms.

## Project documents

- [Developer preview installation](docs/developer-install.md)
- [Build and deploy SpotUI](docs/build.md)
- [Verified firmware build workflow](docs/firmware-build.md)
- [Roadmap](docs/roadmap.md)
- [Recovery notes](docs/recovery.md)

- [Disclaimer](DISCLAIMER.md)
- [Support policy](SUPPORT.md)
- [Third-party components](THIRD_PARTY.md)
- [License](LICENSE)
