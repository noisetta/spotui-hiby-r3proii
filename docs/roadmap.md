# Roadmap

SpotUI is under active development for the HiBy R3 Pro II.

The current device-tested development checkpoint is
`0.1.0-beta.5-test.4`. The latest publicly packaged tester prerelease remains
`0.1.0-beta.2`.

This roadmap is intentionally conservative: stability, recovery, documentation,
and reproducible device testing take priority over convenience features.

## Current status

- Public source repository with separately packaged experimental tester
  prereleases.
- Tested primarily on the HiBy R3 Pro II.
- Current source development is ahead of the latest packaged public prerelease.
- The tester installation uses a two-file SD-card firmware/runtime pair plus a
  separate desktop Spotify authorization step; it is not a one-click installer.
- Firmware and runtime artifacts are release assets, not Git-tracked source
  files.
- SpotUI launches manually from its own dedicated stock-interface tile while
  preserving Qobuz.
- Early launch requests receive native **Preparing SpotUI...** feedback while a
  guarded readiness check waits for the stock player and audio hardware.
- Normal SpotUI exit resumes the existing stock HiBy player instead of
  rebooting the device.
- The UI remains designed around large, simple touch targets for reliable use
  on the compact player screen.

## Recently completed

### Playback and library

- Added queued playback for Liked Songs, playlists, and search results.
- Added latest-tap-wins track selection and responsive, tap-safe Search.
- Added persistent recent-search history with newest-first ordering,
  deduplication, one-tap reruns, and clear-history control.
- Expanded Library discovery beyond the previous eight-entry limit and added
  complete playlist-name resolution.
- Added queue-aware Previous, Next, and Up Next controls.
- Added a dedicated Now Playing screen with larger metadata, elapsed and
  remaining time, seeking, playback controls, and direct queue access.
- Expanded Now Playing with themed track artwork, queue and playback-mode
  context, output status, larger controls, and clearer time information.
- Added persistent shuffle and repeat modes.
- Added touchscreen seeking and hardware volume feedback.
- Preserved the active queue and retried the current track after a first
  transient Spotify unavailable event.
- Added automatic search-result invalidation and refresh after a supervised
  daemon restart.

### Interface and device behavior

- Added ten redesigned themes with adaptive ambient motion.
- Added a staged startup status screen and supervised daemon recovery.
- Added elapsed startup timing and stage-specific WiFi, Spotify, and library
  retry controls.
- Added live WiFi, Spotify, audio, output, and queue diagnostics.
- Added an on-device version identifier in Diagnostics.
- Added persistent brightness settings.
- Added persistent 30-second, 60-second, 2-minute, 5-minute, and Never
  screen-sleep settings with safe touch and power-button wake while audio
  continues.
- Verified cold-start playback and automatic 3.5 mm/4.4 mm routing.

### Stock HiBy integration

- Replaced the stock player's unreliable fork-based launch callback with a
  prestarted lightweight SpotUI launch broker.
- Added a dedicated SpotUI launcher tile while preserving the stock Qobuz
  service tile.
- Added dedicated SpotUI artwork for both HiBy themes.
- Added native **Preparing SpotUI...** feedback for early launch requests.
- Added a guarded readiness gate that waits for the stock player, framebuffer,
  ALSA controls, mixer stability, and minimum initialization state before
  handing control to SpotUI.
- Replaced the normal reboot-on-exit path with a suspend/resume handoff that
  preserves the existing `hiby_player` process.
- Added exclusive touchscreen grabbing while SpotUI is active so touch events
  are not replayed into the suspended stock interface after return.
- Added delayed broker re-arming after return as an additional safeguard
  against queued-input races.
- Verified repeated SpotUI launch/exit cycles and return to the stock HiBy
  interface without rebooting.

### Installation, recovery, and release engineering

- Added a guarded local firmware builder with pre-build and packaged-image
  integrity checks.
- Documented the verified local firmware build workflow.
- Added and device-tested a guarded two-file HMOD v1.5 installer builder with
  low-memory runtime extraction and exact-input validation.
- Added managed runtime upgrades for verified existing SpotUI installations,
  while retaining refusal behavior for unrecognized runtime state.
- Added and device-tested private desktop OAuth onboarding with atomic ADB
  credential installation, strict permissions, and device-local rollback.
- Documented tested UI and daemon cross-build and deployment workflows.
- Expanded recovery, rollback, and common failure troubleshooting procedures.
- Added a developer beta installation guide with prerequisites, validation,
  limitations, and rollback guidance.
- Completed repeated cold-boot, launch/exit, playback, handoff, touchscreen,
  provisioning, and integrity validation for the
  `0.1.0-beta.5-test.4` development checkpoint.

## Current development priorities

- Continue small, device-tested playback, navigation, and interface
  improvements.
- Keep setup, testing, recovery, release, and roadmap documentation synchronized
  with validated development checkpoints.
- Preserve a clear distinction between device-tested development checkpoints
  and privacy-reviewed public release bundles.
- Continue validating the suspend/resume stock-player handoff before making
  additional low-level framebuffer or launcher changes.
- Preserve reproducible hashes and release provenance without adding
  credentials or user-specific device files.

## Next planned improvements

### Library and navigation polish

- Add a compact page or position indicator for long Liked Songs, playlist,
  Search, and Up Next lists.
- Review return paths and per-view browsing position so moving through Search,
  playlists, Now Playing, and the library preserves useful context.
- Make the difference between the active playback queue and the currently
  browsed source clearer without shrinking touch targets.

### Diagnostics and recovery clarity

- Distinguish network, Spotify-session, daemon-restart, and audio-output
  recovery states more clearly in Diagnostics and logs.
- Record concise, privacy-safe troubleshooting steps that testers can include
  in issue reports without exposing account or network credentials.
- Continue targeted recovery testing when a real, reproducible failure is
  found rather than adding speculative workarounds.
- If the intermittent stale-framebuffer return condition reappears, capture
  framebuffer pan state and handoff logs before introducing a page-restoration
  change.

### Installation and release safety

- Add host and device preflight checks for model, storage, expected files,
  binary architecture, and rollback readiness.
- Define a compatibility matrix as additional firmware revisions or devices are
  tested by owners.
- Explore a patch-only workflow based on a user-supplied stock image without
  redistributing proprietary firmware.
- Prepare future public tester releases through the privacy-gated release
  workflow and validate the exact archive intended for distribution.

### Platform integration research

- Profile the guarded HiBy handoff and investigate whether launch latency can
  be reduced without bypassing the codec and mixer readiness checks that
  protect headphone audio.
- Evaluate Bluetooth audio-output support, including pairing assumptions,
  routing, reconnection, status reporting, and playback behavior.
- Investigate whether HiBy MSEB tuning can be exposed safely inside SpotUI,
  including control discovery, persistent settings, output compatibility, and
  coexistence with the stock player's audio configuration.

## Possible future goals

- Support additional HiBy models if tested by device owners.
- Improve UI polish while keeping the interface readable and touch-friendly.
- Continue strengthening recovery behavior for reproducible backend failures.
- Evaluate optional album artwork only if a bounded prototype meets the
  device's memory, network, and framebuffer-performance limits.
- Consider optional bring-your-own-client-ID OAuth support for library writes,
  without making it a standard installation requirement.

## Current service limitation

SpotUI's librespot playback authentication can read the user's Liked Songs but
cannot request Spotify's separate `user-library-modify` Web API permission.

Like and unlike controls are therefore intentionally not included. A future
implementation would require a separate OAuth flow and user-supplied Spotify
developer application configuration.

## Launcher and handoff behavior

The stock HiBy player must finish initializing the codec and mixer before
SpotUI takes control.

If SpotUI is requested early after a cold boot, the stock interface displays a
native **Preparing SpotUI...** message while the launcher waits for its guarded
readiness conditions. The request is retained; repeated tapping is not
required.

Once ready, SpotUI suspends the existing `hiby_player` process rather than
terminating it. On normal exit, SpotUI shuts down its own UI, daemon, and
playback process, then resumes the suspended stock player.

SpotUI exclusively grabs the touchscreen while active so touches made inside
SpotUI are not later delivered to the stock interface when it resumes.

Automatic SpotUI takeover after reboot remains deliberately disabled. Users
can therefore continue using the stock HiBy player normally unless they
explicitly launch SpotUI.

## Not planned right now

- One-click installer.
- Committing ready-to-flash firmware builds to the Git source tree.
- Paid or paywalled features.
- Guaranteed device support.
- Support for devices that have not been tested by owners.

## Contribution priorities

Helpful contributions include:

- Testing on the HiBy R3 Pro II.
- Careful reports from other HiBy models.
- Documentation improvements.
- Recovery notes.
- Build reproducibility improvements.
- Small, readable UI improvements that do not reduce touch target size.
