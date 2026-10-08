# spotui-hiby-r3proii

SpotUI for the HiBy R3 Pro II: an experimental standalone, tetherless streaming UI/client with on-device control.

> Current device-tested development checkpoint: `0.1.0-beta.5-test.4`.
> The latest publicly packaged tester prerelease remains `0.1.0-beta.2`.
> Development checkpoints are experimental and are not automatically equivalent
> to reviewed public release bundles.

[![Support me on Ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/noisetta)

Optional donations support test hardware, documentation, maintenance, and continued experimentation. Nothing is paywalled.

## Screenshots

<p>
  <img src="docs/images/spotui-showcase-v1.png" alt="SpotUI theme and interface showcase">
</p>

This project provides source code, scripts, and notes for running a lightweight standalone music client on HiBy devices. It is intended for device owners who want to build, study, and modify their own hardware.

This project is not affiliated with, endorsed by, or supported by HiBy Music or Spotify.

## Status

SpotUI is under active development and is currently tested primarily on the HiBy R3 Pro II.

Current device-side highlights include:

- Dedicated SpotUI launcher tile alongside the stock services, with Qobuz preserved
- Native **Preparing SpotUI...** feedback, guarded startup readiness, and direct
  return to the HiBy player without rebooting
- On-device browsing and queued playback of Liked Songs and playlists
- Responsive track search with persistent recent-search history
- Queue-aware Previous, Next, and Up Next navigation
- Dedicated Now Playing screen with metadata, seeking, playback controls, and
  direct queue access
- Persistent shuffle, repeat, brightness, and screen-sleep settings
- Touch and physical power-button wake with saved-brightness restoration
- Automatic 3.5 mm and 4.4 mm output routing, including pause on headphone
  disconnection
- Paging and compact-display handling for long track, playlist, search, and
  queue views
- Ten appearance themes with performance-aware ambient animation
- Staged startup, supervised playback recovery, reconnect feedback, and live
  diagnostics for WiFi, Spotify, audio, output, and queue state

“Tetherless” means playback can be browsed and controlled directly from the HiBy instead of using it only as a receiver controlled by a phone or desktop client.

SpotUI is launched manually from its dedicated launcher tile. On a cold boot,
an early launch request is acknowledged with a native **Preparing SpotUI...**
message while the stock player finishes initializing its audio hardware.
SpotUI waits for a guarded readiness check before suspending `hiby_player` and
taking over the display and audio path. This delay is intentional because
taking control from the stock player too early can leave the headphone outputs
silent until reboot.

When SpotUI exits normally, the existing `hiby_player` process is resumed
instead of rebooting the device. Automatic takeover is not enabled, so normal
use of the stock player remains available.

Flashing or modifying firmware can brick your device. Use at your own risk.

## What is included

- `engine/ui/` — framebuffer and touchscreen UI written in Rust using embedded-graphics.
- `engine/launcher/` — launcher script for WiFi bring-up, jack routing, UI startup, daemon supervision, and panel keepalive behavior.
- `engine/firmware/` — init scripts and firmware-side integration notes.
- `apps/spotify/daemon/` — Spotify-compatible daemon source using librespot.

## Source tree and release assets

The Git-tracked source tree does not include:

- HiBy firmware images
- modified `.upt` firmware files
- extracted HiBy binaries
- deployed device snapshots
- Spotify credentials
- `librespot-cache/`
- WiFi credentials
- user-specific device backups
- release binaries or ready-to-flash firmware builds

Reviewed [GitHub prereleases](https://github.com/noisetta/spotui-hiby-r3proii/releases)
may provide exact, device-tested firmware/runtime bundles as separate
downloads. The latest publicly packaged tester prerelease is `0.1.0-beta.2`;
newer source and device-tested development checkpoints are not automatically
equivalent to public release bundles.

Release assets remain experimental and may contain proprietary HiBy components
outside SpotUI's license. They never include Spotify credentials, WiFi
credentials, cache data, logs, or user-specific device files. Read every
included notice and recovery document before flashing.

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
- A Spotify Premium account authorized with the desktop onboarding helper.
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

- [0.1.0-beta.5-test.4 development checkpoint](docs/releases/0.1.0-beta.5-test.4.md)
- [0.1.0-beta.2 release notes](docs/releases/0.1.0-beta.2.md)
- [0.1.0-beta.1 release notes](docs/releases/0.1.0-beta.1.md)
- [Beta testing guide](docs/testing.md)
- [Spotify credential onboarding](docs/credential-onboarding.md)
- [Contribution guidelines](CONTRIBUTING.md)
- [Developer beta installation](docs/developer-install.md)
- [Build and deploy SpotUI](docs/build.md)
- [Verified firmware build workflow](docs/firmware-build.md)
- [HMOD v1.5 tester installer and validation](docs/tester-installer.md)
- [Privacy-gated tester release bundle](docs/release-bundle.md)
- [Roadmap](docs/roadmap.md)
- [Recovery notes](docs/recovery.md)

- [Disclaimer](DISCLAIMER.md)
- [Support policy](SUPPORT.md)
- [Third-party components](THIRD_PARTY.md)
- [License](LICENSE)
