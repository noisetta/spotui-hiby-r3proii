# Roadmap

SpotUI is currently an early developer preview source release for the HiBy R3 Pro II. This roadmap is intentionally conservative: stability, documentation, and recovery come before convenience features.

## Current status

- Early developer preview with a source-only public repository.
- Tested primarily on the HiBy R3 Pro II.
- No ready-to-flash firmware builds are provided in this repository.
- Manual build, patching, and device setup are still required.
- The current UI is designed around large, simple touch rows for reliable use on the device screen.

## Recently completed

- Added a guarded local firmware builder with pre-build and packaged-image integrity checks.
- Documented the verified local firmware build workflow.
- Added final stock-matched SpotUI launcher artwork for both HiBy themes.
- Changed the visible Qobuz launcher caption to SpotUI while preserving internal widget and localization keys.
- Verified early-launch readiness, SpotUI startup, and audio playback on-device.
- Added persistent brightness settings across SpotUI launches.
- Documented the tested UI and daemon cross-build and deployment workflow.
- Expanded recovery, rollback, and common failure troubleshooting procedures.
- Added a developer preview installation guide with prerequisites, validation, limitations, and rollback guidance.

## Currently working in the development build

- SpotUI launches from the repurposed stock app entry point.
- Track list loads through the backend daemon.
- Audio playback works through the supervised daemon/audio pipeline.
- Exit row requires two taps to reduce accidental reboots.
- Brightness row cycles through safe brightness levels.
- Long track names are truncated for readability.
- The launcher performs WiFi bring-up, output jack routing, daemon supervision, and stale temp-file cleanup.

## Medium-term goals

- Add automated installation preflight and compatibility checks.
- Add clearer logs and diagnostics.
- Document device-specific assumptions, such as framebuffer size, input devices, audio routing, and backlight behavior.

## Possible future goals

- Support additional HiBy models if tested by device owners.
- Add a settings screen for brightness and other controls.
- Improve UI polish while keeping the interface readable and touch-friendly.
- Investigate safer fallback behavior if the backend daemon fails.
- Explore a cleaner patch-only installation workflow.

## Not planned right now

- One-click installer.
- Ready-to-flash firmware builds in this repository.
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
