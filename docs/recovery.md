# Recovery and restore notes

Firmware and device modifications can fail. This document collects tested
recovery precautions and troubleshooting directions for SpotUI-related
experiments on the HiBy R3 Pro II.

The current device-tested development checkpoint is
`0.1.0-beta.5-test.4`. The latest publicly packaged tester prerelease remains
`0.1.0-beta.2`.

## Before modifying anything

- Make sure the device battery is charged.
- Keep official stock recovery firmware for the exact device model.
- Keep verified backups of every `/usr/data` file you replace.
- Record hashes before replacing working binaries or scripts.
- Keep a known-good SpotUI firmware/runtime pair when testing a newer one.
- Do not flash firmware intended for another HiBy model.
- Do not test unverified builds unless you are prepared to recover the device.

## General restore approach

The safest full restore path is usually to return to official stock firmware
using the normal HiBy firmware update process for the exact device.

General approach:

1. obtain official firmware for the exact device model;
2. copy it to the SD card using the filename expected by the stock updater;
3. enter the normal firmware update flow;
4. reflash stock firmware;
5. inspect `/usr/data` afterward and remove experimental SpotUI runtime files
   only when that cleanup is actually required.

Exact button combinations, filenames, and update behavior may vary by model.
Confirm the recovery process before experimenting.

## Restore the previous UI and daemon binaries

Incremental development deployments should keep one previous compatible copy
of each binary on `/usr/data`:

```text
/usr/data/spotui-ui-poc.previous
/usr/data/spotui_daemon.previous
```

To restore them:

```fish
adb shell '
set -e

if [ -f /usr/data/spotui-ui-poc.previous ]; then
    cp -p \
        /usr/data/spotui-ui-poc.previous \
        /usr/data/spotui-ui-poc
    chmod 755 /usr/data/spotui-ui-poc
fi

if [ -f /usr/data/spotui_daemon.previous ]; then
    cp -p \
        /usr/data/spotui_daemon.previous \
        /usr/data/spotui_daemon
    chmod 755 /usr/data/spotui_daemon
fi

sync
reboot
'
```

Use a reboot after restoring binaries when a clean cold-start regression is
needed.

Do not attempt to recover a failed handoff by manually starting another
`hiby_player` process. The validated current design suspends and later resumes
the original stock-player process.

## If SpotUI does not launch

The current launcher accepts the request through a lightweight prestarted
broker and waits for guarded readiness conditions before suspending the stock
player.

The readiness checks include:

- minimum system uptime;
- a valid `hiby_player` process;
- expected stock-player initialization state;
- framebuffer availability;
- required ALSA controls;
- stable mixer state.

On a cold boot, an early request should display the native
**Preparing SpotUI...** notice while the request remains pending. Repeated
tapping is not required.

Do not shorten or bypass the readiness gate merely to reduce launch latency.
Taking control before codec and mixer initialization has previously produced
silent headphone output until reboot.

Inspect the startup state with:

```fish
adb shell '
echo "=== Wrapper log ==="
cat /tmp/start_spotui.wrapper.log 2>/dev/null || true

echo
echo "=== UI log ==="
cat /tmp/spotui-ui.log 2>/dev/null || true

echo
echo "=== Daemon log ==="
cat /tmp/daemon.log 2>/dev/null || true

echo
echo "=== Provision log ==="
cat /tmp/spotui-provision.log 2>/dev/null || true
'
```
If SpotUI reaches its own startup screen but stalls at a later stage, use the
retry action for the stage being shown:

- **Retry Wi-Fi** renews association and DHCP;
- **Retry Spotify** restarts the supervised daemon attempt;
- **Retry Library** repeats the saved-track request.

Automatic recovery continues even when the retry action is not used.

Check the relevant processes and socket:

```fish
adb shell '
ps | grep -E "spotui|aplay|hiby_player" | grep -v grep
ls -l /tmp/spotui.sock 2>/dev/null
'
```

If SpotUI never appears, confirm first that the provisioned runtime and
launcher files match the expected build before changing readiness logic.

## If SpotUI does not return cleanly to HiBy

Normal SpotUI exit should:

1. shut down the SpotUI UI;
2. stop its supervised daemon and playback process;
3. resume the suspended stock `hiby_player`;
4. allow pending input to drain briefly;
5. re-arm the SpotUI launch broker.

Inspect:

```fish
adb shell '
echo "=== Return log ==="
cat /tmp/return_to_hiby.log 2>/dev/null || true

echo
echo "=== Wrapper log ==="
cat /tmp/start_spotui.wrapper.log 2>/dev/null || true

echo
echo "=== Processes ==="
ps | grep -E "spotui|aplay|hiby_player" | grep -v grep
'
```

A normal return should not reboot the device.

If the stock interface does not recover, do not start a second
`hiby_player` manually. Reboot the device and return to a known-good build if
needed.

## If a stock tile is selected after SpotUI exits

The current UI exclusively grabs the touchscreen while SpotUI is active so
touch events generated during the SpotUI session are not later consumed by
the suspended stock interface.

If a stock tile is unexpectedly selected after return:

1. record `/tmp/return_to_hiby.log`;
2. record `/tmp/start_spotui.wrapper.log`;
3. confirm the active UI binary matches the expected build;
4. confirm the SpotUI UI logged successful exclusive touchscreen ownership;
5. repeat the test only after preserving the first failure evidence.

Do not immediately add longer sleeps or input-clearing workarounds without
first determining whether exclusive input grabbing was actually active.

## If SpotUI immediately relaunches after exit

The launch broker should be re-armed only after the current SpotUI session has
ended and the short input-drain delay has completed.

If SpotUI relaunches unexpectedly:

```fish
adb shell '
echo "=== Wrapper log ==="
cat /tmp/start_spotui.wrapper.log 2>/dev/null || true

echo
echo "=== Return log ==="
cat /tmp/return_to_hiby.log 2>/dev/null || true

echo
echo "=== Runtime files ==="
ls -lah \
    /usr/data/spotui-ui-poc \
    /usr/data/start_spotui.sh \
    /usr/data/start_spotui.real.sh \
    /usr/data/return_to_hiby.sh \
    2>/dev/null
'
```

Confirm that the UI, launcher, and return script all belong to the same tested
runtime before changing the broker or handoff behavior.

## If the returned stock interface shows a stale SpotUI frame

One intermittent stale-framebuffer return was observed during earlier
development testing. It was not reproduced during final
`0.1.0-beta.5-test.4` validation, so no speculative framebuffer-page fix was
added.

If the condition reappears, collect evidence before pressing the power button
or rebooting when practical:

```fish
adb shell '
echo -n "pan: "
cat /sys/class/graphics/fb0/pan 2>/dev/null
echo

echo "=== Return log ==="
cat /tmp/return_to_hiby.log 2>/dev/null || true
'
```

The framebuffer is double-buffered. During prior investigation the stock HiBy
interface was observed using the second visible page while SpotUI writes its
own frame to the first page.

Do not assume a particular pan value is the cause unless a reproduced failure
supports it.

If the panel remains unusable, reboot and return to a known-good build.

## If Search shows Reconnecting or Nothing Playing

A Spotify session can close after a result list has been fetched but before
the selected track obtains its audio key. The daemon exits after repeated
unavailable-track failures so its supervisor can establish a fresh session.

Starting with SpotUI `0.1.0-beta.1`, the UI invalidates visible results when
the daemon becomes unavailable, briefly returns to the active-search screen,
and automatically reruns the saved query after reconnection.

Wait for the refreshed results page before tapping a track again.

If results do not refresh:

1. return to Search and press `Go` again;
2. confirm `/tmp/spotui.sock` exists and the daemon is running;
3. inspect `/tmp/spotui-ui.log` and `/tmp/daemon.log`;
4. reboot if the supervised daemon does not recover.

## If SpotUI launches but audio does not play

Possible causes include:

- SpotUI took control before stock codec/mixer initialization completed;
- the backend daemon is not running;
- `aplay` is absent while Spotify reports active playback;
- the output jack route is incorrect;
- WiFi is disconnected;
- `/usr/data` is full;
- the active runtime does not match the firmware-side integration.

`aplay` normally exits while playback is paused or stopped, so its absence is
only meaningful while playback should be active.

Useful checks:

```fish
adb shell '
df -h /usr/data

ps | grep -E "spotui|aplay|librespot|wpa|hiby_player" | grep -v grep

cat /sys/class/switch/headset/state 2>/dev/null
cat /sys/class/switch/balance/state 2>/dev/null

amixer -c 0 cget numid=9 2>/dev/null
'
```

Also inspect:

```fish
adb shell '
cat /tmp/start_spotui.wrapper.log 2>/dev/null
echo
cat /tmp/daemon.log 2>/dev/null
'
```

If the failure occurs only after an unusually early launch attempt, preserve
the readiness logs before changing anything.

## If playback stops after a short time

Check free space under `/usr/data`:

```fish
adb shell '
df -h /usr/data
ls -lah /usr/data/tmp
'
```

SpotUI uses temporary files during playback. Stale temp files can fill the
small persistent partition.

The launcher should clean stale:

```text
/usr/data/tmp/.tmp*
```

before starting the daemon.

Do not delete active temporary files while playback is running.

Pause playback before large hashes, pulls, or storage audits because
maintenance I/O can compete with real-time audio and create test-only
underruns.

## If the screen goes black

The display and backlight behavior on the R3 Pro II is delicate. SpotUI
depends on framebuffer refreshes and panel/backlight control while active.

Possible causes include:

- the UI failed during framebuffer takeover;
- a development build stopped refreshing the framebuffer;
- the panel did not wake correctly;
- brightness was set too low;
- a mismatched UI/runtime was installed.

If ADB remains available, try restoring a clearly visible brightness value:

```fish
adb shell '
echo 100 > /sys/class/backlight/backlight_pwm0/brightness
'
```
The tested raw maximum is `100`. Do not write `101`.

Then inspect the UI and wrapper logs.

If the panel does not recover, reboot the device and return to a known-good
build.

## If provisioning fails

The current firmware-side provisioner supports managed runtime installation
and upgrades.

Inspect:

```fish
adb shell '
cat /tmp/spotui-provision.log 2>/dev/null || true
echo
df -h /usr/data
'
```

Expected broad states are:

- matching managed runtime: leave it in place;
- older verified managed runtime: back it up and upgrade it;
- fresh managed installation: install the matched SD-card runtime;
- unrecognized or unsafe runtime: refuse to overwrite automatically.

If the provisioner refuses an unrecognized runtime state, treat that refusal
as a safety feature.

Do not delete or overwrite the unknown state merely to make the installer
continue.

Before any manual cleanup:

1. archive the existing runtime privately;
2. record hashes and version markers;
3. confirm the candidate firmware/runtime pair is matched;
4. preserve credentials and unrelated `/usr/data` data.

## If the runtime upgrade fails

A managed upgrade should preserve the previously recognized runtime as its
rollback copy before activating the new one.

If an upgrade fails:

1. stop before launching SpotUI;
2. preserve `/tmp/spotui-provision.log`;
3. inspect free space;
4. identify the active, rollback, and staging runtime state;
5. compare all available hashes with the expected manifests;
6. restore only a verified compatible runtime.

Do not mix launcher scripts, UI binaries, daemon binaries, or runtime metadata
from unrelated checkpoints.

## Qobuz and the dedicated SpotUI tile

The current validated development integration does **not** repurpose Qobuz.

SpotUI has its own dedicated launcher tile, and the stock Qobuz tile remains
available independently.

Therefore, restoring Qobuz is not part of normal SpotUI rollback.

If a much older experimental firmware build replaced the Qobuz entry, use the
historical documentation for that build or reflash a known-good/current
firmware image. Do not apply the old Qobuz-restoration procedure to the current
dedicated-tile integration.

## Restore the complete stock firmware

Use official stock firmware when you need to remove the firmware-side SpotUI
integration completely or when the experimental firmware state is no longer
trusted.

A full stock reflash is preferable to piecemeal restoration when:

- the player binary state is uncertain;
- firmware resources no longer match the expected build;
- the device fails during normal boot;
- multiple experimental firmware revisions have been mixed;
- the OTA/rootfs state cannot be verified confidently.

Afterward, inspect `/usr/data` separately if you also want to remove persistent
SpotUI runtime files or credentials.

Do not assume a stock firmware flash necessarily erases every persistent
`/usr/data` file.

## Known-good backup practice

Before replacing a working build, preserve:

- UI binary;
- daemon binary;
- musl loader;
- launcher scripts;
- return-to-HiBy script;
- runtime version and manifest metadata;
- current source commit or tag;
- known-good firmware/runtime pair and checksums;
- recovery firmware.

Keep private backups outside the public repository.

Do not commit:

- device snapshots;
- Spotify credentials;
- WiFi configuration;
- librespot cache contents;
- proprietary firmware;
- extracted proprietary binaries;
- unsanitized private logs.

Pause playback before creating or verifying large device-side backups.

## Public issue reports

When reporting problems publicly, provide the smallest useful sanitized
evidence.

Useful information can include:

- SpotUI checkpoint or release;
- source commit or tag;
- device model and firmware base;
- whether the test followed a cold reboot;
- whether launch was requested before readiness;
- exact reproduction steps;
- sanitized relevant log excerpts.

Do not include:

- WiFi passwords or network secrets;
- Spotify credentials or tokens;
- `librespot-cache/` contents;
- account identifiers;
- private device backups;
- proprietary firmware or extracted proprietary binaries;
- unsanitized logs containing personal information.

For structured tester reporting, see [Beta testing SpotUI](testing.md).
