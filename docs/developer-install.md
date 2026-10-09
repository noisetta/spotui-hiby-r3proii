# Developer installation

SpotUI is under active development for the HiBy R3 Pro II.

The current device-tested development checkpoint is
`0.1.0-beta.5-test.4`. The latest publicly packaged tester prerelease remains
`0.1.0-beta.2`.

SpotUI is not a one-click installer, an end-user firmware package, or a
supported consumer release.

> [!WARNING]
> Installing or developing SpotUI currently requires cross-compiling software,
> modifying or rebuilding firmware, flashing custom firmware, using ADB, and
> maintaining a recovery path. A mistake can make the player temporarily
> unbootable. Continue only if you are comfortable recovering the exact device
> with official stock firmware.

## Intended audience

This workflow is intended for developers and experienced device modders who can:

- use Linux command-line tools;
- work with ADB and a root shell;
- build Rust software for a custom MIPS target;
- inspect firmware and SquashFS contents;
- verify file hashes and packaged firmware contents;
- maintain private backups and rollback copies;
- recover the HiBy R3 Pro II by reflashing official firmware.

It is not yet suitable for someone looking for a normal application
installation.

## Supported configuration

The currently tested configuration is:

- **Device:** HiBy R3 Pro II
- **Current development checkpoint:** `0.1.0-beta.5-test.4`
- **Latest public tester prerelease:** `0.1.0-beta.2`
- **Firmware base:** HiBy Mods v1.5 for the R3 Pro II
- **Target:** `mipsel-unknown-linux-musl`
- **Host:** Linux development environment
- **Playback engine:** librespot-based SpotUI daemon
- **Launcher integration:** dedicated SpotUI tile alongside the stock services
- **Qobuz:** preserved as its own stock service tile
- **Installation model:** matched custom firmware plus managed runtime archive
- **Account requirement:** user-supplied Spotify Premium account
- **Network requirement:** user-supplied device WiFi configuration

Do not flash generated firmware on another HiBy model.

## Current installation model

There are three related development workflows.

### 1. Incremental ADB deployment

Use this when the device already has a working SpotUI firmware/runtime base.

Typical uses include:

- replacing the UI binary;
- replacing the daemon;
- testing launcher or return scripts;
- collecting logs;
- validating small source changes.

Follow [Build and deploy SpotUI](build.md).

### 2. Guarded HMOD tester installer

The current integrated firmware/runtime workflow is based on the exact
published HiBy Mods v1.5 firmware for the R3 Pro II.

The guarded installer builder:

- verifies the expected HMOD input;
- applies the reviewed SpotUI firmware-side integration;
- packages the matched SpotUI runtime separately;
- adds the dedicated SpotUI launcher while preserving Qobuz;
- includes the native **Preparing SpotUI...** launch feedback;
- installs or upgrades only recognized managed SpotUI runtime state;
- verifies the final firmware and runtime outputs.

See [HMOD v1.5 tester installer](tester-installer.md).

### 3. Lower-level firmware development

The repository also documents the lower-level firmware build and inspection
process used while developing the integration.

Use [Verified firmware build workflow](firmware-build.md) when working directly
with a privately prepared firmware tree or investigating firmware-side changes.

Do not treat older Qobuz-replacement examples or historical firmware
iterations as the current launcher design.

## Source tree and release boundary

The Git-tracked source tree does not provide:

- official or modified HiBy firmware images;
- proprietary HiBy binaries;
- an automatically extracted stock firmware tree;
- a complete MIPS cross-toolchain;
- Spotify credentials, tokens, or cache files;
- WiFi credentials;
- device backups;
- a one-command installation or uninstall process.

A clearly marked GitHub prerelease may separately attach an exact reviewed
tester bundle.

The latest publicly packaged tester prerelease is currently
`0.1.0-beta.2`.

Newer development checkpoints, including `0.1.0-beta.5-test.4`, are not
automatically public release bundles. A development build must separately pass
the privacy-remapped release workflow and exact-archive testing before public
distribution.

See [Tester release bundle workflow](release-bundle.md).

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
- the required firmware build tools;
- enough local storage for private firmware trees and backups;
- a Spotify Premium account;
- the desktop authentication helper built from source or obtained as a matched
  release artifact;
- working WiFi configuration on the device.

Stop if any recovery or model-specific requirement is uncertain.

## Before modifying the device

Read [Recovery and restore notes](recovery.md) completely.

At minimum:

1. Preserve official stock recovery firmware.
2. Confirm how to enter the updater on the exact device.
3. Back up every replaced `/usr/data` file.
4. Keep known-good UI, daemon, launcher, return script, runtime, and firmware
   copies.
5. Record hashes before replacing device files.
6. Confirm that ADB works before flashing experimental changes.
7. Never test a build intended for another model.

The current integration does **not** replace Qobuz. SpotUI has its own launcher
tile and the stock Qobuz tile remains available.

## Step 1: build the UI and daemon

Follow [Build and deploy SpotUI](build.md) to:

- build `spotui-ui-poc`;
- copy the canonical daemon source into the local librespot examples tree;
- build `spotui_daemon`;
- verify both output binaries;
- understand their tested device paths;
- preserve a protocol-compatible rollback pair.

Do not proceed with binaries that fail target, architecture, or hash checks.

For a development checkpoint, record the exact source commit and binary hashes.
Do not infer the checkpoint version solely from the UI's compiled Diagnostics
label unless the source package version has also been intentionally updated.

## Step 2: prepare authentication and device configuration

The daemon expects a reusable librespot credential cache on the persistent
device partition.

Credentials are never stored in this repository, firmware, or runtime archive.

Use [Spotify credential onboarding](credential-onboarding.md) to authorize in a
browser and install the private credential atomically over ADB.

The device must also have:

- working WiFi;
- correct time and network access;
- sufficient free space under `/usr/data`;
- the tested loader and launcher files;
- the return-to-HiBy script;
- the required audio, framebuffer, and input environment.

Keep SpotUI closed at the stock HiBy interface while onboarding.

The helper refuses an active SpotUI/audio stack, retains one private device
rollback copy, and removes its temporary host credential after installation.

After onboarding:

1. launch SpotUI;
2. confirm Liked Songs or another authenticated library view loads;
3. start playback;
4. test pause, resume, Next, and Search;
5. exit SpotUI normally and confirm the stock HiBy interface returns;
6. reboot once and confirm the credential still works after a cold start.

The reboot in step 6 tests persistence. It is not the normal SpotUI exit path.

## Step 3: choose the deployment path

### Incremental ADB testing

Use the ADB deployment sections in [Build and deploy SpotUI](build.md) when the
device already contains a working SpotUI installation.

Always:

1. stop or exit SpotUI cleanly before replacing active binaries;
2. back up the installed UI, daemon, and any script being changed;
3. push replacements to temporary `.new` paths;
4. compare local and device hashes;
5. set the expected permissions;
6. atomically activate the replacement;
7. reboot when a clean cold-start test is required;
8. run the applicable launch, playback, handoff, and recovery regression;
9. inspect logs before committing source changes.

For UI/daemon protocol changes, stage and verify both compatible binaries
before activating either one.

### Guarded firmware/runtime build

Use [HMOD v1.5 tester installer](tester-installer.md) for the current integrated
firmware/runtime path.

The builder requires the exact supported HMOD base and reviewed SpotUI inputs.

The resulting pair consists of:

```text
r3proii.upt
spotui-runtime.tar.xz
```

The firmware and runtime are matched artifacts. Do not mix a firmware image
from one build with a runtime archive from another.

The builder and post-build audit should verify the exact expected:

- firmware base;
- player patch state;
- launcher integration;
- localization changes;
- SpotUI artwork;
- provisioner;
- runtime payload;
- rootfs;
- kernel;
- OTA chunk chain and manifest.

Do not bypass failed checks merely to produce an image.

## Step 4: verify the firmware/runtime pair

Before flashing, record and inspect the applicable:

- firmware MD5;
- firmware SHA-256;
- runtime SHA-256;
- rootfs MD5 and SHA-256;
- rootfs size, compression, and block size;
- kernel hash;
- patched player hash;
- launcher and return scripts;
- provisioner;
- packaged UI and daemon hashes;
- SpotUI launcher artwork;
- localization files;
- runtime version metadata;
- chunk-chain and OTA manifest results.

A successful build command is not, by itself, proof that an image is safe to
flash.

Compare the output with the exact checkpoint or release record being tested.

For `0.1.0-beta.5-test.4`, see
[development checkpoint notes](releases/0.1.0-beta.5-test.4.md).

## Step 5: flash and validate provisioning

Use only the normal firmware update procedure confirmed for the HiBy R3 Pro II.

Copy the exact matched firmware/runtime pair to the SD card as required by the
tester-installer workflow.

After flashing:

1. allow the device to boot into the stock HiBy interface;
2. do not immediately launch SpotUI;
3. enable ADB manually when required for development validation;
4. inspect `/tmp/spotui-provision.log`;
5. confirm that the provisioner reports the expected runtime state;
6. verify the installed runtime version and critical hashes;
7. confirm that unrelated `/usr/data` contents and private credentials remain
   intact.

The provisioner can distinguish among:

- an already matching managed runtime;
- an older recognized managed runtime that can be backed up and upgraded;
- a fresh installation with no managed runtime;
- an unrecognized or unsafe runtime state that must not be overwritten
  automatically.

Stop and investigate unexpected provisioner behavior rather than forcing an
overwrite.

## Step 6: first launch and handoff test

During the first launch test:

1. confirm that the stock HiBy interface is responsive;
2. open the Stream media screen;
3. confirm that both the dedicated SpotUI tile and Qobuz tile are present;
4. tap SpotUI;
5. if the readiness gate is still pending, confirm that the native
   **Preparing SpotUI...** notice appears;
6. allow the retained launch request to continue without repeated tapping;
7. confirm that SpotUI starts;
8. open Diagnostics and record the displayed version and status;
9. confirm Liked Songs and playlists load;
10. start playback;
11. verify audio through the intended output;
12. test pause, resume, Previous, Next, seeking, and automatic advancement;
13. test Search and playlist playback;
14. test brightness and configured screen sleep/wake;
15. test the intended headphone-removal and reconnection behavior;
16. exit SpotUI normally;
17. confirm that the existing stock HiBy interface returns without rebooting;
18. confirm no stock tile is selected unexpectedly;
19. confirm SpotUI does not immediately relaunch;
20. launch SpotUI again and complete another launch/exit cycle.

The launcher intentionally waits for the stock player and audio hardware to
reach a safe state before suspending `hiby_player`. Taking control too early
has previously caused silent headphone output until reboot.

Do not remove or shorten the readiness gate merely to make launch appear
faster.

## Current handoff model

The validated development handoff works as follows:

1. the dedicated SpotUI tile sends a request to the prestarted launch broker;
2. an early request receives native **Preparing SpotUI...** feedback;
3. the launcher waits for its guarded readiness conditions;
4. the existing `hiby_player` process is suspended rather than killed;
5. SpotUI takes control of the framebuffer, audio path, and touchscreen;
6. the UI exclusively grabs the touchscreen while active;
7. normal Exit shuts down SpotUI's UI, daemon, and playback process;
8. the existing stock player is resumed;
9. the launch broker is re-armed after a short input-drain delay.

The exclusive touchscreen grab prevents touches generated during the SpotUI
session from later being replayed into the suspended stock interface.

Automatic SpotUI takeover after reboot remains disabled.

## Logs and validation

The primary runtime and handoff logs are:

```text
/tmp/start_spotui.wrapper.log
/tmp/spotui-ui.log
/tmp/daemon.log
/tmp/return_to_hiby.log
/tmp/spotui-provision.log
```

The runtime control socket is:

```text
/tmp/spotui.sock
```

Use the commands in [Build and deploy SpotUI](build.md) and
[Recovery and restore notes](recovery.md) to inspect startup, daemon, audio,
handoff, provisioning, and socket state.

A valid current test should confirm:

- the launch request is accepted without blocking the stock callback;
- an early request receives native preparation feedback;
- SpotUI waits for device readiness;
- the stock player is suspended only after the readiness gate passes;
- the UI owns the framebuffer while SpotUI is active;
- the touchscreen is exclusively grabbed during the SpotUI session;
- the daemon remains supervised;
- `aplay` is present during active playback;
- audio works after a cold boot;
- normal Exit resumes the existing stock interface without rebooting;
- no queued SpotUI touch selects a stock tile after return;
- another SpotUI launch works after returning to HiBy.

Pause playback before pulling, copying, or hashing large binaries on the
device. Sustained flash I/O and hashing can compete with real-time audio on
this low-powered target and create test-only underruns.

## Framebuffer return diagnostics

A stale SpotUI framebuffer image was observed once during earlier development
testing but was not reproduced during final `beta.5-test.4` validation.

No speculative framebuffer-page workaround is included.

If the condition reappears, collect the framebuffer pan state and handoff logs
before pressing buttons or rebooting, when practical.

Do not treat an unconfirmed framebuffer hypothesis as a general recovery
procedure.

## Recovery and rollback

For an incremental binary regression:

1. exit SpotUI normally when possible;
2. restore the verified `.previous` UI/daemon pair or other known-good runtime
   files;
3. verify their hashes and permissions;
4. reboot for a clean cold-start regression when appropriate.

For firmware-side launcher, localization, provisioner, or boot problems:

1. use a previously verified compatible SpotUI firmware/runtime pair; or
2. reflash official stock firmware for the exact HiBy R3 Pro II.

Do not depend on copying files into `/usr/resource` at runtime. That filesystem
is read-only during normal operation.

Do not attempt to recover a failed handoff by manually launching a second
`hiby_player` process. The validated current design resumes the original
suspended process.

See [Recovery and restore notes](recovery.md) for the detailed recovery path.

## Current limitations

The development workflow currently has these limitations:

- only the HiBy R3 Pro II has been validated;
- source builds and new firmware preparation still contain manual and private
  steps;
- the HMOD installer supports only its exact validated R3 Pro II/HMOD v1.5
  input;
- no automated stock-firmware extraction workflow is included;
- no public preflight utility checks every device model and firmware revision;
- the authentication helper has currently been validated only on x86-64 Linux
  and still requires a browser, USB, and manually enabled ADB;
- Like/unlike library writes are unavailable because they require a separate
  Spotify Web API authorization flow;
- the safe cold-boot readiness gate intentionally delays SpotUI takeover while
  the stock player initializes its codec and mixer;
- SpotUI launch remains manual so a normal boot does not unexpectedly interrupt
  use of the stock player;
- development checkpoints are not automatically equivalent to reviewed public
  release bundles;
- no automatic updater or general-purpose uninstaller exists;
- recovery requires familiarity with HiBy firmware flashing;
- functionality, packaging, and file formats may continue to change during
  development.

Qobuz replacement is **not** a current limitation: the dedicated SpotUI tile
coexists with the stock Qobuz tile in the validated development checkpoint.

## When to stop

Do not flash when:

- the device model is not exactly the tested model;
- the stock recovery image is unavailable;
- the firmware/runtime pair does not belong to the same build;
- expected input hashes do not match;
- the kernel differs unexpectedly;
- the rootfs metadata or OTA chunk chain fails validation;
- the provisioner would overwrite an unrecognized runtime state;
- proprietary or credential files would be committed publicly;
- the recovery procedure is not understood;
- the device cannot be reached through ADB when ADB is required for the planned
  development test.

## Release status

Completing this developer workflow does not produce an automatically approved
public release.

The current device-tested development checkpoint is
`0.1.0-beta.5-test.4`.

The latest publicly packaged tester prerelease remains
`0.1.0-beta.2`.

Before a development checkpoint becomes a public tester release, the exact
candidate must pass the privacy-remapped build, bundle audit, checksum,
recovery, and device-validation workflow described in
[Tester release bundle workflow](release-bundle.md).

A public release should identify the exact source commit or tag and publish the
checksum of the exact archive distributed to testers.

Before installation can be generalized further, useful future work includes:

- a reproducible patch-only workflow from a user-supplied stock image;
- automated host and device preflight checks;
- versioned authentication-helper artifacts for additional tested host
  platforms;
- reproducible patches or artifacts that contain no proprietary firmware;
- an explicit compatibility matrix;
- a documented uninstall or stock-restore path;
- testing by additional HiBy R3 Pro II owners.
