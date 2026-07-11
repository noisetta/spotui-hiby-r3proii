# Recovery and restore notes

Firmware and device modifications can fail. This document collects basic recovery precautions and known troubleshooting directions for SpotUI-related experiments.

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

## If SpotUI launches but audio does not play

Possible causes:

- The stock player was killed too early before codec initialization completed.
- The backend daemon is not running.
- `aplay` is not running.
- The output jack route is wrong.
- WiFi is not connected.
- `/usr/data` is full.

Useful checks:

```sh
df -h /usr/data
ps | grep -E "spotui|aplay|librespot|wpa" | grep -v grep
cat /sys/class/switch/headset/state 2>/dev/null
cat /sys/class/switch/balance/state 2>/dev/null
amixer -c 0 cget numid=9 2>/dev/null
```

## If playback stops after a short time

Check free space on `/usr/data`:

```sh
df -h /usr/data
ls -lah /usr/data/tmp
```

SpotUI uses temporary files during playback. Stale temp files can fill the small persistent partition. The launcher should clean stale `/usr/data/tmp/.tmp*` files before starting the daemon, but active temp files should not be deleted while playback is running.

## If the screen goes black

The display/backlight behavior on the R3 Pro II is delicate. SpotUI relies on frequent framebuffer refreshes to keep the panel visible.

Possible causes:

- The UI did not start quickly enough after the stock player was stopped.
- The device backlight setting is not configured to stay available during startup.
- A test build stopped refreshing the framebuffer.
- Brightness was set too low.

If ADB is still available, restore full brightness with:

```sh
echo 101 > /sys/class/backlight/backlight_pwm0/brightness
```

If the panel does not recover, reboot the device and return to a known-good build.

## If the device repeatedly launches SpotUI unexpectedly

Remove or disable the SpotUI launcher/integration files, or reflash stock firmware.

Useful files to check on development builds:

```sh
ls -lah /usr/data/spotui-ui-poc /usr/data/start_spotui.sh /usr/data/start_spotui.real.sh /usr/data/return_to_hiby.sh 2>/dev/null
```

## Known-good backup practice

Before replacing a working build, save copies of:

- UI binary
- launcher script
- return/exit script
- daemon binary, if locally built
- current source code

Keep private backups outside the public repository. Do not commit device snapshots, credentials, WiFi configuration, cache files, or firmware images.

## Public issue reports

When reporting problems publicly, do not include:

- WiFi passwords
- Spotify credentials
- `librespot-cache/` contents
- personal logs with account information
- proprietary firmware images or extracted proprietary binaries

Sanitize logs before posting.
