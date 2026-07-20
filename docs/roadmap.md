# Roadmap

SpotUI is currently an early developer preview source release for the HiBy R3 Pro II. This roadmap is intentionally conservative: stability, documentation, and recovery come before convenience features.

## Current status

- Early developer preview with a source-only public repository.
- Tested primarily on the HiBy R3 Pro II.
- No ready-to-flash firmware builds are provided in this repository.
- Manual build, patching, and device setup are still required.
- The current UI is designed around large, simple touch rows for reliable use on the device screen.

## Recently completed

- Added queued playback for Liked Songs, playlists, and search results.
- Added latest-tap-wins track selection and responsive, tap-safe Search.
- Added queue-aware Previous, Next, and Up Next controls.
- Added a dedicated Now Playing screen with larger metadata, elapsed and
  remaining time, seeking, playback controls, and direct queue access.
- Added persistent shuffle and repeat modes.
- Added touchscreen seeking and hardware volume feedback.
- Added ten redesigned themes with adaptive ambient motion.
- Added a staged startup status screen and supervised daemon recovery.
- Added live WiFi, Spotify, audio, output, and queue diagnostics.
- Added persistent 30-second, 60-second, 2-minute, 5-minute, and Never
  screen-sleep settings with safe touch wake while audio continues.
- Tested early stock-side launch feedback and retained the manual, audio-safe
  handoff after framebuffer contention made the prototype unsuitable.
- Verified cold-start playback and automatic 3.5 mm/4.4 mm routing.
- Added a guarded local firmware builder with pre-build and packaged-image integrity checks.
- Documented the verified local firmware build workflow.
- Added final stock-matched SpotUI launcher artwork for both HiBy themes.
- Changed the visible Qobuz launcher caption to SpotUI while preserving internal widget and localization keys.
- Verified early-launch readiness, SpotUI startup, and audio playback on-device.
- Added persistent brightness settings across SpotUI launches.
- Documented the tested UI and daemon cross-build and deployment workflow.
- Expanded recovery, rollback, and common failure troubleshooting procedures.
- Added a developer preview installation guide with prerequisites, validation, limitations, and rollback guidance.

## Current development priorities

- Investigate whether hardware power-button wake can be supported safely.
- Keep public setup, recovery, and feature documentation synchronized with
  tested milestones.

## Medium-term goals

- Add automated installation preflight and compatibility checks.
- Document device-specific assumptions, such as framebuffer size, input devices, audio routing, and backlight behavior.

## Possible future goals

- Support additional HiBy models if tested by device owners.
- Add a settings screen for brightness and other controls.
- Improve UI polish while keeping the interface readable and touch-friendly.
- Investigate safer fallback behavior if the backend daemon fails.
- Explore a cleaner patch-only installation workflow.
- Consider optional bring-your-own-client-ID OAuth support for library writes,
  without making it a standard installation requirement.

## Current service limitation

SpotUI's librespot playback authentication can read the user's Liked Songs but
cannot request Spotify's separate `user-library-modify` Web API permission.
Like and unlike controls are therefore intentionally not included. A future
implementation would require a separate OAuth flow and user-supplied Spotify
developer application configuration.

## Launcher limitation

The stock HiBy player must finish initializing the codec and mixer before
SpotUI takes over. A cold manual launch can therefore leave the stock interface
visible for tens of seconds before SpotUI's own loading page appears. The
nonblocking launcher accepts the request once and ignores repeated taps while
it waits.

A tested framebuffer-only preparation page was not retained: the proprietary
stock interface continuously flips and redraws its framebuffer pages, causing
the two interfaces to flicker and leaving stale frames visible. Refreshing the
prototype aggressively enough to dominate the display would add load during
the audio-critical initialization period. A clean stock-side status dialog
would require deeper, firmware-specific integration with the proprietary HiBy
interface.

SpotUI also does not take over automatically after every reboot. Automatic
takeover would interrupt users who intend to use the stock player, so manual
launching remains the deliberate default.

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
