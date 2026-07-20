# Developer preview installation

SpotUI is currently an **early developer preview** for the HiBy R3 Pro II. It is not a one-click installer, an end-user firmware package, or a supported consumer release.

> [!WARNING]
> Installing SpotUI currently requires cross-compiling software, modifying a private firmware tree, flashing custom firmware, and maintaining a recovery path. A mistake can make the player temporarily unbootable. Continue only if you are comfortable recovering the exact device with official stock firmware.

## Intended audience

This workflow is intended for developers and experienced device modders who can:

- use Linux command-line tools;
- work with ADB and a root shell;
- build Rust software for a custom MIPS target;
- inspect and modify an extracted SquashFS filesystem;
- verify file hashes and packaged firmware contents;
- recover the HiBy R3 Pro II by reflashing official firmware.

It is not yet suitable for someone looking for a normal application installation.

## Supported configuration

The currently tested configuration is:

- **Device:** HiBy R3 Pro II
- **Target:** `mipsel-unknown-linux-musl`
- **Host:** Linux development environment
- **Playback engine:** librespot-based SpotUI daemon
- **Launcher integration:** repurposed stock Qobuz tile
- **Installation type:** private, locally built custom firmware
- **Account requirement:** user-supplied Spotify Premium account
- **Network requirement:** user-supplied device WiFi configuration

Do not flash the generated firmware on another HiBy model.

## Current installation model

There are two related development workflows:

1. **ADB development deployment**
   - Build and replace the UI or daemon binaries on an already prepared SpotUI device.
   - Use this for incremental testing.
   - Follow [Build and deploy SpotUI](build.md).

2. **Firmware integration**
   - Place the tested SpotUI components into a privately prepared firmware tree.
   - Package and verify a custom `.upt` image.
   - Flash the resulting image on the HiBy R3 Pro II.
   - Follow [Verified firmware build workflow](firmware-build.md).

The firmware workflow is required for durable launcher artwork, the visible SpotUI caption, and the firmware-side Qobuz integration.

## What the repository does not provide

The public repository does not provide:

- official or modified HiBy firmware images;
- a ready-to-flash SpotUI `.upt` release;
- proprietary HiBy binaries;
- an automatically extracted and patched firmware tree;
- a complete MIPS cross-toolchain;
- Spotify credentials, tokens, or cache files;
- WiFi credentials;
- a guided authentication wizard;
- a one-command installation or uninstall process.

The guarded firmware builder packages and verifies an **already prepared** private firmware tree. It does not download stock firmware, extract it, or apply every proprietary binary modification automatically.

## Required local components

Before beginning, confirm that you have:

- a HiBy R3 Pro II;
- official stock recovery firmware for that exact model;
- a charged device;
- a reliable microSD card;
- a working ADB connection;
- a Linux build host;
- Rust nightly with `rust-src`;
- the working `mipsel-unknown-linux-musl` cross-build environment;
- a local librespot source tree compatible with the SpotUI daemon;
- the required firmware build tools listed in [Verified firmware build workflow](firmware-build.md);
- enough local storage for private extracted firmware trees and backups;
- a Spotify Premium account and a locally prepared librespot credential cache;
- working WiFi configuration on the device.

Stop here if any recovery or model-specific requirement is uncertain.

## Before modifying the device

Read [Recovery and restore notes](recovery.md) completely.

At minimum:

1. Preserve official stock firmware.
2. Confirm how to enter the updater on the exact device.
3. Back up every replaced `/usr/data` file.
4. Keep known-good UI, daemon, launcher, and firmware copies.
5. Confirm that ADB works before flashing experimental changes.
6. Never test a build intended for another model.

The current integration replaces the stock Qobuz launcher entry. Restoring the original artwork, caption, and behavior requires a rebuilt firmware image or an official stock reflash.

## Step 1: build the UI and daemon

Follow [Build and deploy SpotUI](build.md) to:

- build `spotui-ui-poc`;
- copy the canonical daemon source into the local librespot examples tree;
- build `spotui_daemon`;
- verify both output binaries;
- understand their tested device paths.

Do not proceed with binaries that fail the target or hash checks.

## Step 2: prepare authentication and device configuration

The current daemon expects a user-prepared librespot credential cache on the persistent device partition. Credentials are not stored in this repository.

The device must also have:

- working WiFi;
- correct time and network access;
- sufficient free space under `/usr/data`;
- the tested loader and launcher files;
- the required audio and framebuffer environment.

Authentication setup is not yet a polished public workflow. Treat it as a development prerequisite rather than an installation step that the repository automates.

## Step 3: choose the deployment path

### Incremental ADB testing

Use the ADB deployment sections in [Build and deploy SpotUI](build.md) when the device already contains a working SpotUI firmware base.

Always:

1. back up the installed UI and daemon;
2. push to a temporary `.new` path;
3. compare local and device hashes;
4. set executable permissions;
5. atomically replace the installed file;
6. reboot and test;
7. inspect logs before committing source changes.

This path does not create the initial firmware integration by itself.

### Custom firmware build

Use [Verified firmware build workflow](firmware-build.md) for the complete firmware-side integration.

The private prepared tree must already contain the tested:

- HiBy player modifications;
- SpotUI UI binary;
- SpotUI daemon;
- musl loader;
- real launcher;
- nonblocking launcher wrapper;
- init and startup integration;
- four SpotUI launcher images;
- localization updates for the visible SpotUI caption.

The public builder deliberately refuses unexpected inputs by checking known hashes and package structure. Do not bypass failed checks merely to produce an image.

## Step 4: verify the firmware image

Before flashing, record and inspect:

- firmware MD5;
- firmware SHA-256;
- rootfs MD5 and SHA-256;
- rootfs compression and block size;
- kernel hash;
- packaged launcher scripts;
- packaged UI and daemon;
- SpotUI launcher artwork;
- localization files;
- chunk chain and manifest results.

The verified builder prints these results when all stages pass.

A successful build command is not, by itself, proof that an image is safe to flash. Compare the output with the tested build record in [Verified firmware build workflow](firmware-build.md).

## Step 5: flash and perform the first test

Use only the normal firmware update procedure confirmed for the HiBy R3 Pro II.

During the first test:

1. keep the player connected to a computer with ADB available;
2. confirm that the device boots normally;
3. confirm the stock interface remains responsive before opening SpotUI;
4. open the Stream media screen;
5. confirm the SpotUI tile and caption render correctly;
6. launch SpotUI;
7. confirm that the UI starts;
8. confirm that liked songs load;
9. start playback;
10. verify audio through the intended output;
11. test pause and resume;
12. test brightness;
13. exit SpotUI;
14. reboot and repeat the launch and playback test.

Do not assume a successful first launch proves cold-boot reliability. Reboot testing is required.

## Logs and validation

The primary runtime files are:

```text
/tmp/start_spotui.wrapper.log
/tmp/spotui-ui.log
/tmp/daemon.log
/tmp/spotui.sock
```

Use the commands in [Build and deploy SpotUI](build.md) and [Recovery and restore notes](recovery.md) to inspect startup, daemon, audio, and socket state.

A valid test should confirm:

- the wrapper returns control to the HiBy interface immediately;
- SpotUI waits for device readiness rather than killing the stock player too early;
- the UI owns and refreshes the framebuffer after takeover;
- the daemon and `aplay` remain supervised;
- audio works after a cold boot;
- the device can be recovered by rebooting if the interface freezes.

Do not manually restart `hiby_player` after SpotUI has taken over the framebuffer. That has not been a reliable recovery path.

## Rollback

For a development binary regression, restore the `.previous` UI and daemon copies using [Recovery and restore notes](recovery.md), then reboot.

For firmware-side launcher, artwork, localization, or boot problems:

1. use a known-good privately built firmware image; or
2. reflash official stock firmware for the exact HiBy R3 Pro II.

Do not depend on copying files into `/usr/resource` at runtime. That filesystem is read-only in normal operation.

## Current limitations

The developer preview currently has these installation limitations:

- only the HiBy R3 Pro II has been validated;
- the stock Qobuz tile is repurposed;
- firmware preparation still contains manual and private steps;
- no ready-to-flash public image is provided;
- no automated stock-firmware extraction workflow is included;
- no public preflight utility checks the device model and firmware revision;
- no guided Spotify authentication flow is included;
- Like/unlike library writes are unavailable because they require a separate
  Spotify Web API OAuth authorization;
- screen sleep supports persistent 30-second, 60-second, 2-minute, 5-minute,
  and Never options with safe touch or physical power-button wake;
- the stock launcher handoff may take tens of seconds before SpotUI can draw
  its own loading page;
- the delay protects codec and mixer initialization; stopping `hiby_player`
  earlier has produced silent headphone output until reboot;
- the launcher remains manual so a normal boot does not unexpectedly interrupt
  use of the stock player;
- stock-side framebuffer progress is not used because HiBy continuously
  redraws and flips its own pages, causing contention and stale frames;
- no automatic updater or uninstaller exists;
- recovery requires familiarity with HiBy firmware flashing;
- functionality and file formats may change during development.

## When to stop

Do not flash when:

- the device model is not exactly the tested model;
- the stock recovery image is unavailable;
- the prepared-tree hashes do not match the builder expectations;
- the kernel differs unexpectedly;
- the rootfs metadata or chunk chain fails validation;
- proprietary or credential files would be committed publicly;
- the user does not understand the recovery procedure;
- the device cannot be reached through ADB during development testing.

## Release status

Completing this guide does not make SpotUI generally installable. It provides a safe entry point for technically experienced preview testers and connects the existing build, firmware, and recovery documents.

A future public beta should first add:

- a reproducible preparation or patch-only workflow from a user-supplied stock image;
- automated host and device preflight checks;
- clearer authentication setup;
- versioned release artifacts or patches;
- an explicit compatibility matrix;
- an uninstall or stock-restore workflow;
- testing by additional HiBy R3 Pro II owners.
