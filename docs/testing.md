# Beta testing SpotUI

SpotUI testing is currently limited to experienced HiBy R3 Pro II owners who
already have a working development installation and a recovery path. This guide
does not make SpotUI generally installable and must not be used as a substitute
for the developer installation, build, or recovery documents.

The current device-tested development checkpoint is
`0.1.0-beta.5-test.4`. The latest publicly packaged tester prerelease remains
`0.1.0-beta.2`. Always test the exact version or build named in the test
request.

## Before testing

- Confirm that the device is exactly a HiBy R3 Pro II.
- Keep official stock recovery firmware available.
- Confirm ADB access and back up every replaced device file.
- Read the developer installation and recovery documents completely.
- Use the source, firmware, runtime, and version named in the test request.
- Never publish credentials, WiFi details, cache files, private account logs,
  proprietary firmware, or device backups.

Do not test experimental builds on another HiBy model unless the maintainer
has explicitly agreed to a separate hardware investigation.

## Information to record

- SpotUI version shown in Diagnostics
- Source commit or tag
- Exact device model and firmware revision
- Installation path used
- 3.5 mm, 4.4 mm, or other output under test
- Screen-sleep, theme, shuffle, and repeat settings
- Whether the test started after a full reboot
- Whether SpotUI was requested before or after the startup readiness gate
- Clear reproduction steps and the observed result

## Core beta test

1. Reboot the device and confirm that the stock HiBy interface is responsive.
2. Tap the dedicated SpotUI launcher tile.
3. If the device is still within the guarded startup period, confirm that the
   native **Preparing SpotUI...** message appears and that repeated taps are
   not required.
4. Allow SpotUI to launch and record the visible startup stages and timing.
5. Confirm the Diagnostics version and status tiles.
6. Play several Liked tracks and listen for startup delay or stutter.
7. Rapidly select several Liked tracks and confirm only the final track plays.
8. Repeat normal and rapid selection in a playlist.
9. Run a new Search and play a result.
10. Test Previous, Next, pause, resume, seeking, Now Playing, and Up Next.
11. Confirm automatic advancement and synchronized highlighting and metadata.
12. Test the configured screen sleep and physical power-button wake while
    audio is playing.
13. Test the applicable headphone-removal and reconnection behavior.
14. Confirm settings persist after a full reboot.
15. Exit SpotUI normally and confirm that the existing stock HiBy interface
    returns visibly and responsively without rebooting the device.
16. Confirm that no stale SpotUI frame remains, no unexpected stock tile is
    selected, and SpotUI does not immediately relaunch by itself.
17. Launch SpotUI again from the dedicated tile and complete at least one
    additional launch/exit cycle.

Qobuz should remain available as its own stock service tile throughout the
test.

Only perform failure-injection, daemon-restart, firmware, launcher, runtime
upgrade, Bluetooth, framebuffer, or MSEB tests when the maintainer has agreed
to the exact procedure.

## Logs and privacy

The primary runtime and handoff logs are:

```text
/tmp/start_spotui.wrapper.log
/tmp/spotui-ui.log
/tmp/daemon.log
/tmp/return_to_hiby.log
/tmp/spotui-provision.log
```

Inspect and sanitize logs before sharing them. Remove account names, network
information, credentials, tokens, and any unrelated personal data. Prefer the
smallest log excerpt that shows the failure.

Pause playback before hashing, pulling, or copying large device binaries.
Maintenance I/O can compete with real-time audio and create test-only
underruns.

## Reporting results

Use the repository's **Beta test report** issue form for a passing or mixed
test session. Use **Bug report** for one reproducible defect. Use **Change
proposal** before starting substantial implementation work.

Maintainer capacity is limited. A complete report may not receive an immediate
response, but structured results are still valuable for future triage.
