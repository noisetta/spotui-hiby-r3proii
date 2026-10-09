# HMOD v1.5 tester installer

> [!CAUTION]
> This installer and its output remain experimental. Public testers should use
> only the exact archive attached to a clearly marked SpotUI GitHub prerelease,
> verify its published outer SHA-256 and bundled `SHA256SUMS`, and read the
> included recovery guidance before flashing. Locally rebuilt output must not
> be distributed until it completes the same review and device-validation gate.

The current device-tested development checkpoint is
`0.1.0-beta.5-test.4`. The latest publicly packaged tester prerelease remains
`0.1.0-beta.2`.

The tester installer converts the exact published HiBy Mods v1.5 firmware for
the HiBy R3 Pro II into a two-file SpotUI installer. Testers copy the firmware
and its matched runtime archive to the SD card. The builder does not download
firmware, flash a device, or include Spotify credentials, WiFi configuration,
cache contents, or device-specific files.

## Supported firmware input

Only this upstream image is accepted:

```text
Project:  hiby-modding/hiby-mods
Release:  v1.5
Filename: r3proii-v1.5-hmod.upt
SHA-256:  631af685977877f65288e371d49f3b2839681ee4ca4713234f498519e2ab33f2
Device:   HiBy R3 Pro II
```

The builder checks the complete firmware, rootfs, kernel, player, player
wrapper, backup player, and localization hashes. It refuses similar or
modified inputs.

HiBy Mods tooling, documentation, and original project assets have their own
license. The firmware contains proprietary HiBy components that are not
covered by that MIT license. Preserve the upstream license, firmware notice,
credit, and warranty language when preparing any SpotUI release.

## What the installer adds

The generated image:

- preserves the verified HMOD v1.5 kernel and inherited player modifications;
- applies the reviewed SpotUI integration to the known `hiby_player` binary;
- signals a prestarted lightweight launch broker without forking the stock
  player;
- retains the original backup player;
- adds a dedicated SpotUI launcher tile while preserving the existing Qobuz
  service tile;
- adds dedicated SpotUI artwork for both stock HiBy themes;
- adds native **Preparing SpotUI...** feedback for launch requests received
  before the guarded readiness gate is satisfied;
- preserves normal stock-player use when SpotUI is not active;
- emits a separate compressed, private-data-free SpotUI runtime archive;
- stores verified runtime manifest metadata and version information in the
  firmware rootfs;
- starts the stock player before launching a low-priority background
  provisioner;
- provisions or upgrades a verified managed SpotUI runtime under `/usr/data`
  without blocking normal boot;
- does not enable SpotUI autostart or force always-on ADB.

The provisioner recognizes three broad runtime states:

1. **Matching managed runtime** — the installed runtime already matches the
   packaged manifest, so it is left in place.
2. **Older verified managed runtime** — the existing installation is validated,
   backed up, and upgraded to the matched packaged runtime.
3. **Unrecognized or unsafe runtime state** — the provisioner refuses to
   overwrite it automatically.

Unrelated `/usr/data` contents, Spotify credentials, WiFi configuration, and
user-specific files are not part of the managed runtime upgrade.

SD-card waiting, hashing, validation, backup, and extraction happen in a
low-priority background worker after `hiby_player` starts. The worker records
its result in:

```text
/tmp/spotui-provision.log
```

SpotUI launch itself is readiness-gated. Once the stock player and audio
hardware are ready, the launcher suspends the existing `hiby_player` process
for the SpotUI session.

Normal SpotUI exit shuts down the SpotUI UI, daemon, and playback process,
then resumes the same stock player instead of rebooting the device. SpotUI
also takes exclusive ownership of the touchscreen while active so touch input
is not replayed into the suspended stock interface after return.

## Build inputs

The builder requires locally reviewed release binaries:

- `spotui-ui-poc`;
- `spotui_daemon`;
- `ld-musl-mipsel-sf.so.1`.

The builder verifies that the UI and daemon are 32-bit little-endian MIPS PIE
executables using the expected musl interpreter. It follows a loader symlink
and packages the actual loader file.

From the SpotUI repository, a Fish-compatible invocation is:

```fish
tools/installer/build_spotui_tester_upt.sh \
    --base /path/to/r3proii-v1.5-hmod.upt \
    --ui engine/ui/target/mipsel-unknown-linux-musl/release/spotui-ui-poc \
    --daemon /path/to/librespot/target/mipsel-unknown-linux-musl/release/examples/spotui_daemon \
    --loader /path/to/ld-musl-mipsel-sf.so.1 \
    --output /path/to/r3proii-hmod-1.5-spotui-0.1.0-beta.5-test.4.upt \
    --runtime-output /path/to/spotui-runtime.tar.xz
```

Neither output path may already exist. The builder:

1. verifies and extracts HMOD v1.5;
2. applies the reviewed SpotUI player, localization, launcher, artwork,
   provisioner, and handoff integration;
3. builds a reproducible compressed runtime archive for the SD card;
4. adds its verified metadata and guarded background provisioner to the
   firmware;
5. rebuilds the LZO SquashFS within the root partition limit;
6. regenerates the OTA chunk chain and manifest;
7. packages the `.upt`;
8. re-extracts the result and verifies every critical hash;
9. confirms that the kernel is unchanged and prints release checksums.

The runtime uses an explicit XZ preset-4 dictionary and CRC32 checksum. The
device has no swap and only about 56 MB of RAM; desktop-oriented XZ preset 9
requires too much decompression memory for its BusyBox decoder.

Generated `.upt` images, runtime archives, and runtime binaries must not be
committed to the Git-tracked source tree. An exact, device-tested
firmware/runtime pair may instead be attached to a clearly marked GitHub
prerelease with its hashes, provenance, notices, and recovery warning.

The current `0.1.0-beta.5-test.4` checkpoint is a device-tested development
build, not a reviewed public release bundle.

## SpotUI 0.1.0-beta.5-test.4 development-checkpoint validation

The `beta.5-test.4` development checkpoint adds the dedicated SpotUI launcher,
native preparation feedback, guarded startup readiness, suspend/resume return
to the stock player, managed runtime upgrades, and exclusive touchscreen input
ownership.

The exact packaged development pair completed repeated launch/exit, cold-boot,
playback, handoff, provisioning, touchscreen, and integrity testing on the
maintainer HiBy R3 Pro II on 2026-10-08:

```text
Firmware SHA-256: 0cb6a9a2556906cf55d7f68e7963ea8b615c36f148d6d58623dfaf2054895617
Firmware MD5:     0e6e35bf5ccb36202c328790137b934d
Runtime SHA-256:  fb617b091da44a2bb71b78cbf73ebec35707021c826bfe29f7101f0c13758b9b
Rootfs SHA-256:   a34106879bac9bf6c4ef4fffeeafca8b62a036b8d44fbb784f7ab353bbe98f6c
Rootfs MD5:       5ccc8bd6c7b6698d83b2ffde71fc6c08
Rootfs size:      37376000 bytes
UI SHA-256:       813a17788376b04ae20cbae7a2efeb9ca6e06d964e6b31432d9601d15c4b4226
Daemon SHA-256:   e368c922075c251f527a5dc2e6a9fc2b72b8f84f0f8647d7257dd9966aee0ed5
Loader SHA-256:   ad3247d5c5a22ee0076c28c5e80b841ea24c604687a855997b9eb8aa77db4d37
Return SHA-256:   a7ef050ed7c193d863a7103204e733170fb2cb73da3c6620b47ac76e21020248
Player SHA-256:   b3e787645d86bcf887699f53810e456bb8b7e9cd975014b46e421996255ae520
```

The packaged runtime identifies itself as:

```text
spotui-0.1.0-beta.5-test.4+hmod-1.5
```

The provisioner successfully recognized the prior managed runtime, preserved a
rollback copy, and installed the matched test.4 runtime.

The dedicated SpotUI tile and stock Qobuz tile remained available
independently.

During final packaged-build testing, repeated SpotUI launch/exit cycles
returned directly to the resumed stock HiBy interface without rebooting.
Exclusive touchscreen grabbing prevented SpotUI touch events from being
replayed into the suspended stock interface, and no unintended immediate
SpotUI relaunches were observed during the final test cycles.

An intermittent stale-framebuffer return condition seen once during earlier
development testing could not be reproduced during final test.4 validation.
No speculative framebuffer-page workaround was therefore added.

This checkpoint is a device-tested development build, not a privacy-reviewed
public tester release. The latest packaged public prerelease remains
`0.1.0-beta.2`.

## Historical validation records

The following sections preserve validation results for earlier exact builds.
Descriptions of launcher, provisioning, or exit behavior in these records
apply to those builds and should not be interpreted as the current installer
behavior described above.

## Maintainer device-validation record

The following unpublished pair completed the controlled test sequence on one
HiBy R3 Pro II on 2026-07-27:

```text
Firmware SHA-256: 7dd7dc29b165e579f17199d0435d4666b9e689fedd3b19a581fec9f99ba0213c
Firmware MD5:     1381cae3dd08567ce649889b09e2a91d
Runtime SHA-256:  38c48bf1896838d836e082e9c67901ad5a8bc484aa268cbfd8ece79414015364
Rootfs SHA-256:   09e99ee33bc6237a23a0a8179ee406d52f1fd0ea3149dfbd8f4444647f6b2da4
Rootfs size:      37322752 bytes
Runtime profile:  XZ preset 4, CRC32, 4 MiB dictionary
```

Stage 1 preserved an existing matching SpotUI runtime, reached the stock HiBy
interface in about 10 seconds, launched SpotUI, and passed playback, controls,
queue, search, sleep/wake, and exit/reboot testing. Stage 2 installed the same
runtime from an SD card after all six runtime targets were absent. Every
installed hash and permission matched the packaged manifest. Rapid taps still
coalesced to the newest request, search playback remained correct, audio used
the expected `aplay` subprocess, and the exit path rebooted in about 10
seconds. The following boot detected matching installed metadata and skipped
extraction without leaving a staging directory or lock.

This record establishes the known-good maintainer build; it is not by itself
authorization to publish the artifacts. Public testing still requires the
credential onboarding, release packaging, notices, and recovery guidance
described in this document.

## SpotUI 0.1.0-beta.2 release-candidate validation

The beta.2 pair adds complete Library playlist discovery and the prestarted
stock-player launch broker. It passed reproducibility, privacy, installation,
cold launch, Library, playback, active-stock handoff, paused-stock handoff,
and normal-exit testing on the maintainer HiBy R3 Pro II on 2026-08-05:

```text
Firmware SHA-256: 4821a75376ad9f624b3dbd6392ead76cff41fa3b1594e00e1261a4894eb51f6c
Firmware MD5:     7860dde40285936473090aced1a1e9b7
Runtime SHA-256:  2fd5c1fddb9d1f18ecef9569c0f1bd5abfa4f43c02554ebe5ed09da1d3802944
Rootfs SHA-256:   e34d001295ce57326c277e557a8724fb123d227503f70dcc3f542df54688e3b6
Rootfs size:      37322752 bytes
UI SHA-256:       3ca6c63bd953b8ee477ebd20b5d945e5259a39d5dfbb0d3713a2da93355067be
Daemon SHA-256:   e368c922075c251f527a5dc2e6a9fc2b72b8f84f0f8647d7257dd9966aee0ed5
Runtime profile:  XZ preset 4, CRC32, 4 MiB dictionary
```

The provisioner initially refused to overwrite the prior beta.1 runtime,
preserving its safe no-overwrite behavior. After the prior runtime and stale
rollback copies were archived privately and removed from the constrained
device storage, the provisioner installed beta.2 from the SD card. All six
runtime hashes, version metadata, permissions, and staging cleanup passed.

## Privacy-remapped release-candidate validation

The public-tester candidate was rebuilt with neutral Rust source paths and a
bounded background extraction settling/retry policy. The following exact pair
completed both existing-runtime and fresh-provision testing on the maintainer
device on 2026-07-27:

```text
Firmware SHA-256: b4e78ab3eb7154f68ffc333a9fdd3770de5e16b1a65752a908812e3c7cfe6df0
Firmware MD5:     1a9afde2f694c11ae12ad9f9e7e70ca4
Runtime SHA-256:  f16ff95b69400ee360e27c860e690835e3bea8898742dd65b3b362048d0e8da8
Rootfs SHA-256:   c5448b6716d71702ec810988572d24fba1eb3c319b22cbefd6903f8af6dd06f1
Rootfs size:      37322752 bytes
UI SHA-256:       e8111b160f8daad4bf2ebae94beaba5fe0f67ad29a5e4596334c4789e4bb573e
Daemon SHA-256:   f7e8cbcedbb918900cf85aa7a4589c18ec4a9031db6754ac915f13daf00c7769
Runtime profile:  XZ preset 4, CRC32, 4 MiB dictionary
```

The stock interface appeared in approximately 5–10 seconds and was not
blocked by provisioning. With all six runtime targets absent, the final
firmware waited 30 seconds for startup I/O to settle and installed the runtime
on its first extraction attempt. Every installed hash and permission matched,
metadata matched the firmware manifest, no staging directory or lock remained,
and private device credentials retained mode 0600 inside a mode-0700 cache.
Liked Songs, Search, pause/resume, Next, audio, highlighting, and metadata then
passed on the freshly provisioned runtime.

Two intermediate privacy-remapped firmware images are retired because their
fresh-boot extraction windows were too short. Do not distribute images with
these SHA-256 values:

```text
28de738dc6aee77e1298b160876f011b62949adafcac6d524c670bd515af49b7
46ef2c45add56850f48640e83cb6b0b9f72f8de5803287878d45a97f7fa7fa25
```

Both failed safely without activating partial runtime files or changing
credentials. The final candidate supersedes them.

## Retired embedded-payload prototype

The first unpublished prototype embedded the compressed runtime in the rootfs
and ran its verifier synchronously before `hiby_player`. Its updater reported
success, but the tested R3 Pro II remained at the HiBy boot logo. The device
was recovered with the preserved working firmware and all persistent SpotUI
data remained intact.

The failed prototype has SHA-256:

```text
9c3f551d0567ea31dda8bc6a0efffea1d3b662c69417aeaba7154088c8eba758
```

Do not flash, distribute, or use that image as a future build base. The
two-file design intentionally keeps the multi-megabyte runtime out of the
rootfs and makes provisioning unable to delay the stock UI boot path.

An intermediate two-file build booted normally but its fresh-install worker
rejected the valid runtime archive as corrupt because that archive used the
64 MiB XZ preset-9 dictionary. It failed before creating any partial runtime
files. The incompatible runtime archive has SHA-256:

```text
e5b6a25dc196679298aceecaf47638b9d870c6c5cd898a225c01b1184501a1cc
```

Do not distribute that archive or either firmware/runtime pair built around
it. The builder now fixes the archive to the device-tested low-memory profile.

## Authentication boundary

The image intentionally contains no account data. A new tester enables ADB
from the stock HMOD interface and runs the separately built desktop OAuth
helper before launching SpotUI. The tested helper creates the reusable
librespot credential in a private temporary directory, stages and verifies it
over ADB, preserves one device-local rollback copy, and removes the temporary
host copy. See [Spotify credential onboarding](credential-onboarding.md).

Authentication onboarding passed its complete device test on 2026-07-27. It
does not make the firmware or runtime archive account-specific, configure
WiFi, or enable ADB permanently.

## Controlled device-test sequence

Use the exact firmware/runtime pair being evaluated. Keep a verified copy of
the currently working runtime and official recovery firmware before changing
the device.

### Stage 1: existing matching managed runtime

1. Pause playback and confirm the SpotUI playback subprocess has exited.
2. Archive the active UI, daemon, loader, launcher scripts, and runtime
   metadata together with their hashes.
3. Record the active firmware, player, rootfs, kernel, runtime version, and
   available `/usr/data` space.
4. Keep official recovery firmware for the exact R3 Pro II available.
5. Copy the verified candidate firmware to the SD card as `r3proii.upt` and
   its matched runtime archive as `spotui-runtime.tar.xz`.
6. Flash it with the normal R3 Pro II firmware update procedure.
7. Let the device boot into the stock HiBy interface; do not launch SpotUI yet.
8. Enable ADB manually from HMOD when required for validation.
9. Inspect `/tmp/spotui-provision.log` and confirm that the existing runtime
   is recognized as matching the packaged managed runtime.
10. Verify the active runtime, player, kernel, provisioner, version metadata,
    and critical packaged hashes.
11. Tap the dedicated SpotUI tile.
12. If the readiness gate is still pending, confirm that native
    **Preparing SpotUI...** feedback appears.
13. Run the complete beta regression.
14. Exit SpotUI normally and confirm that the existing stock HiBy interface
    returns visibly and responsively without rebooting.
15. Confirm that no stale SpotUI frame remains, no stock tile receives an
    unintended touch, and SpotUI does not immediately relaunch.
16. Launch SpotUI again and complete at least one additional launch/exit cycle.

### Stage 2: managed runtime upgrade

Perform this stage only when the candidate is intended to upgrade an older
recognized SpotUI runtime.

1. Begin with a verified older managed SpotUI runtime.
2. Record its version and manifest hashes before flashing the candidate.
3. Keep the matched candidate `spotui-runtime.tar.xz` on the SD-card root.
4. Flash and boot the candidate firmware.
5. Inspect `/tmp/spotui-provision.log`.
6. Confirm that the existing runtime is recognized as managed before any
   replacement occurs.
7. Confirm that the previous managed runtime is backed up and the candidate
   runtime is installed successfully.
8. Verify the installed runtime version, permissions, manifest hashes, and
   staging cleanup.
9. Confirm Spotify credentials, WiFi configuration, and unrelated `/usr/data`
   contents are unchanged.
10. Run the full launch, playback, return-to-HiBy, touchscreen-isolation, and
    second-launch regression.

If the provisioner reports an unrecognized runtime state, stop and investigate
rather than forcing an overwrite.

### Stage 3: fresh provisioning

Perform this only after the currently working runtime has been archived and
Stages 1 and 2, when applicable, have passed.

1. Pause playback and make a final verified archive of the active runtime.
2. Remove only the managed SpotUI runtime targets required to simulate a fresh
   installation; do not remove credentials, WiFi configuration, or unrelated
   `/usr/data` content.
3. Keep the matched `spotui-runtime.tar.xz` at the SD-card root.
4. Boot the candidate firmware and allow the background provisioner to run
   after the stock player starts.
5. Inspect `/tmp/spotui-provision.log` and confirm that the fresh runtime is
   installed without activating a partial staging state.
6. Verify the provisioned version, permissions, manifest hashes, and cleanup
   before launching SpotUI.
7. Confirm private credentials and unrelated persistent data remain unchanged.
8. Run the complete cold-launch, playback, queue, Search, controls, sleep/wake,
   headphone, handoff, touchscreen-isolation, second-launch, and recovery
   regression.

Do not publish a candidate if any required stage needs an unexplained manual
repair.
