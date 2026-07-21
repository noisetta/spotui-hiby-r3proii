# Recovery and restore notes

Firmware and device modifications can fail. This document collects tested recovery precautions and troubleshooting directions for SpotUI-related experiments.

## Before modifying anything

- Make sure the device battery is charged.
- Keep a copy of the official stock firmware for your exact device model.
- Keep backups of any files you replace on `/usr/data`.
- Do not flash firmware intended for a different HiBy model.
- Do not test unverified builds if you are not prepared to recover the device.

## General restore approach

The safest restore path is usually to return to official stock firmware using the normal HiBy firmware update process for the device.

General approach:

1. Obtain the official firmware for your exact device model.
2. Copy it to the SD card using the filename expected by the stock updater.
3. Boot into the normal firmware update flow.
4. Reflash stock firmware.
5. Remove any experimental files from `/usr/data` if needed.

Exact button combinations, filenames, and update behavior may vary by model. Confirm the process for your device before experimenting.

## Restore the previous UI and daemon binaries

Development deployments should keep one previous copy of each binary on `/usr/data`:

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

Use a reboot after restoring the files. Do not try to restart the stock `hiby_player` process manually after SpotUI has taken control of the framebuffer.

## If the launcher fails or SpotUI starts too early

The current launcher uses a nonblocking wrapper and waits for the stock player, framebuffer, ALSA devices, mixer state, and minimum uptime to become ready.

On a cold boot, continuing to see the responsive stock interface for tens of
seconds after tapping SpotUI can be normal. Repeated taps are ignored by the
launch lock. SpotUI cannot safely draw its own progress page until the stock
player has completed audio initialization and released the framebuffer. Do not
shorten the readiness gate merely to make the UI appear sooner: stopping the
stock player early has caused silent headphone output that required a reboot.

Inspect the startup logs:

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
'
```

The startup page shows elapsed waiting time and a retry action for its current
stage. `Retry Wi-Fi` renews association and DHCP, `Retry Spotify` restarts only
the supervised daemon attempt, and `Retry Library` repeats the saved-track
request. Automatic recovery continues even when the button is not used.

Check the relevant processes and socket:

```fish
adb shell '
ps | grep -E "spotui|aplay|hiby_player" | grep -v grep
ls -l /tmp/spotui.sock 2>/dev/null
'
```

If the stock interface freezes on the last framebuffer image, reboot the device. Manually launching `hiby_player` after framebuffer takeover has not been a reliable recovery method.

## If SpotUI launches but audio does not play

Possible causes:

- The stock player was killed too early before codec initialization completed.
- The backend daemon is not running.
- `aplay` is not running while Spotify reports active playback. It normally
  exits while playback is paused or stopped.
- The output jack route is wrong.
- WiFi is not connected.
- `/usr/data` is full.

Useful checks:

```fish
adb shell '
df -h /usr/data
ps | grep -E "spotui|aplay|librespot|wpa" | grep -v grep
cat /sys/class/switch/headset/state 2>/dev/null
cat /sys/class/switch/balance/state 2>/dev/null
amixer -c 0 cget numid=9 2>/dev/null
'
```

## If playback stops after a short time

Check free space on `/usr/data`:

```fish
adb shell '
df -h /usr/data
ls -lah /usr/data/tmp
'
```

SpotUI uses temporary files during playback. Stale temp files can fill the small persistent partition. The launcher should clean stale `/usr/data/tmp/.tmp*` files before starting the daemon, but active temp files should not be deleted while playback is running.

## If the screen goes black

The display and backlight behavior on the R3 Pro II is delicate. SpotUI relies on frequent framebuffer refreshes to keep the panel visible.

Possible causes:

- The UI did not start quickly enough after the stock player was stopped.
- The device backlight setting is not configured to stay available during startup.
- A test build stopped refreshing the framebuffer.
- Brightness was set too low.

If ADB is still available, restore the tested maximum brightness with:

```fish
adb shell '
echo 100 > /sys/class/backlight/backlight_pwm0/brightness
'
```

The tested raw maximum is `100`. Do not write `101`.

If the panel does not recover, reboot the device and return to a known-good build.

## Restore the stock Qobuz launcher entry

The current firmware integration repurposes the stock Qobuz tile. Restoring Qobuz requires all of the following:

- restore the four original Qobuz PNG resources;
- restore the visible localization value from `SpotUI` to `Qobuz`;
- restore the original launcher behavior or reflash official stock firmware.

The original artwork is retained in the public repository at:

```text
engine/launcher/resources/qobuz-original/
```

The firmware resource filesystem under `/usr/resource` is read-only during normal operation. Pushing replacement files there with ADB is not persistent and cannot be used as a complete restore method.

For a durable restore, either:

1. rebuild a firmware image containing the original resources and caption; or
2. reflash the official stock firmware for the exact HiBy R3 Pro II model.

The localization key itself remains `qobuz`. Only its displayed UTF-16LE value changes:

```xml
<qobuz>Qobuz</qobuz>
```

## If the device repeatedly launches SpotUI unexpectedly

Remove or disable the SpotUI launcher integration, or reflash stock firmware.

Useful files to check on development builds:

```fish
adb shell '
ls -lah \
    /usr/data/spotui-ui-poc \
    /usr/data/start_spotui.sh \
    /usr/data/start_spotui.real.sh \
    /usr/data/return_to_hiby.sh \
    2>/dev/null
'
```

Because the current launcher entry is embedded in the firmware-side HiBy integration, a durable return to the original Qobuz behavior requires a rebuilt firmware image or an official stock reflash.

## Known-good backup practice

Before replacing a working build, save copies of:

- UI binary;
- launcher scripts;
- daemon binary;
- loader;
- current source code;
- known-good firmware output and checksums;
- original launcher artwork and localization files.

Keep private backups outside the public repository. Do not commit device snapshots, credentials, WiFi configuration, cache files, or firmware images.

## Public issue reports

When reporting problems publicly, do not include:

- WiFi passwords;
- Spotify credentials;
- `librespot-cache/` contents;
- personal logs with account information;
- proprietary firmware images or extracted proprietary binaries.

Sanitize logs before posting.
