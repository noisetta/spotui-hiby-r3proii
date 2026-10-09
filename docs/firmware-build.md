# Firmware integration and build workflow

This document describes the verified firmware-side SpotUI integration for the
HiBy R3 Pro II and distinguishes the current HMOD-based installer from the
older Qobuz-replacement firmware builder retained in the repository for
historical reference.

The current device-tested development checkpoint is
`0.1.0-beta.5-test.4`. The latest publicly packaged tester prerelease remains
`0.1.0-beta.2`.

The Git-tracked source tree does not contain proprietary HiBy firmware images,
extracted proprietary firmware files, ready-to-flash update images,
credentials, or device snapshots.

> [!WARNING]
> Custom firmware can make a device temporarily unbootable. These workflows
> have only been tested on the HiBy R3 Pro II. Do not use generated firmware on
> another model.

## Current firmware path

The current integrated firmware/runtime builder is:

```text
tools/installer/build_spotui_tester_upt.sh
```

It operates only on the exact published HiBy Mods v1.5 firmware for the
R3 Pro II.

The corresponding rootfs patcher is:

```text
tools/installer/patch_hmod_v15_rootfs.py
```

For end-to-end builder inputs, output handling, provisioning behavior, and
device validation, see
[HMOD v1.5 tester installer](tester-installer.md).

This document focuses on the firmware-side integration and lower-level build
behavior.

## Supported base firmware

The current builder accepts only:

```text
Project:  hiby-modding/hiby-mods
Release:  v1.5
Filename: r3proii-v1.5-hmod.upt
SHA-256:  631af685977877f65288e371d49f3b2839681ee4ca4713234f498519e2ab33f2
Device:   HiBy R3 Pro II
```

The builder verifies the complete input firmware and its expected rootfs,
kernel, player, wrapper, backup player, layouts, and localization state before
packaging SpotUI.

Similar or manually modified base images are rejected.

## Current dedicated launcher integration

The current firmware integration does **not** replace Qobuz.

Instead, the patcher adds a separate SpotUI entry to the stock Stream media
screen while preserving the original Qobuz entry and functionality.

The integration includes:

- a dedicated SpotUI tile in all four checked Stream media layouts;
- dedicated normal and selected SpotUI artwork for both stock themes;
- a separate `<spotui>SpotUI</spotui>` localization entry;
- a native `<spotprep>Preparing SpotUI...</spotprep>` stock-interface notice;
- a dedicated SpotUI callback identity inside the patched `hiby_player`;
- a lightweight prestarted launch broker;
- the guarded SpotUI startup handoff;
- support for resuming the existing stock player after SpotUI exits.

The patcher verifies that the Qobuz registration remains present while adding
the separate SpotUI registration.

## Stream media layouts

The current integration patches these four firmware resources:

```text
usr/resource/layout/theme1/hiby_stream_media.view
usr/resource/layout/theme1/hiby_stream_media_cn.view
usr/resource/layout/theme2/hiby_stream_media.view
usr/resource/layout/theme2/hiby_stream_media_cn.view
```

The verified output hashes for `0.1.0-beta.5-test.4` are:

```text
theme1:
4b92577f25708274dacce31eec24ca3f462d3081f026f7e03cbd4e9a42d39c74

theme1 CN:
318cf393dec22a2b07b776475d59389d178b648ef87a854a8bce236dfe801afa

theme2:
b5c296156c12f3f81ed8fe3fbfaa8d71feda9ae3e3a60ca5054e625b9476b21a

theme2 CN:
ce12d10fe4ad2e015496089e3425d8c4aa8434496e82b55967705d7f69f291ff
```

The patcher derives the additional SpotUI tile from the verified stock layout
structure, changes its internal identity to SpotUI, positions it separately,
and verifies that both Qobuz and SpotUI registrations exist exactly as
expected.

Do not manually reproduce this change against an unknown layout file.

## Launcher artwork

Current SpotUI artwork is stored in the repository under:

```text
engine/launcher/resources/spotui/
```

The four active assets are:

```text
engine/launcher/resources/spotui/theme1/spotui.png
engine/launcher/resources/spotui/theme1/spotui_s.png
engine/launcher/resources/spotui/theme2/spotui.png
engine/launcher/resources/spotui/theme2/spotui_s.png
```

They are installed in the firmware as:

```text
usr/resource/litegui/theme1/stream_media/spotui.png
usr/resource/litegui/theme1/stream_media/spotui_s.png
usr/resource/litegui/theme2/stream_media/spotui.png
usr/resource/litegui/theme2/stream_media/spotui_s.png
```

The stock Qobuz artwork remains in its original Qobuz paths.

Older repository resources whose filenames contain `qobuz` belong to the
historical Qobuz-replacement integration and are not the current dedicated
tile assets.

## Localization

The current integration preserves:

```xml
<qobuz>Qobuz</qobuz>
```

and adds:

```xml
<spotui>SpotUI</spotui>
```

to each checked `tidal.ini`.

The patcher verifies 13 language files:

```text
english
french
german
italy
japanese
korean
poland
russian
simplified_chinese
spain
thai
traditional_chinese
ukrainian
```

It also adds the native preparation label:

```xml
<spotprep>Preparing SpotUI...</spotprep>
```

to the corresponding checked `switch.ini` resources.

These localization files use firmware-specific encoding and line-ending
formats. Use the verified patcher rather than editing them casually with a
text editor.

## Player integration

The patcher accepts only the expected HMOD v1.5 `hiby_player` and applies the
reviewed dedicated-launcher delta.

The verified resulting player SHA-256 is:

```text
b3e787645d86bcf887699f53810e456bb8b7e9cd975014b46e421996255ae520
```

The integration provides:

- a dedicated SpotUI callback;
- native preparation-notice support;
- signaling to the prestarted broker;
- preservation of Qobuz registration;
- compatibility with the guarded launcher and no-reboot return path.

The original HMOD backup player is also verified and preserved.

Do not apply these byte changes to a player binary with a different input
hash.

## Runtime separation

The current firmware does not embed the complete SpotUI runtime payload in the
rootfs.

Instead, the builder creates a matched SD-card archive:

```text
spotui-runtime.tar.xz
```

containing the managed runtime files:

```text
spotui-ui-poc
spotui_daemon
ld-musl-mipsel-sf.so.1
start_spotui.sh
start_spotui.real.sh
return_to_hiby.sh
VERSION
SHA256SUMS
```

This keeps the multi-megabyte runtime outside the constrained firmware rootfs
and allows the firmware-side provisioner to install or upgrade a recognized
runtime under `/usr/data`.

The runtime archive uses the tested low-memory XZ configuration with a CRC32
checksum.

## Firmware-side provisioner

The builder installs the guarded background provisioner as:

```text
/etc/init.d/S99spotui-provision
```

It runs after the stock player has started and uses firmware-stored manifest
metadata to validate the matched SD-card runtime.

The broad supported states are:

- matching managed runtime: leave it unchanged;
- older recognized managed runtime: back it up and upgrade it;
- fresh installation: install the matched runtime;
- unrecognized or unsafe state: refuse to overwrite automatically.

Provisioning is intentionally separated from the stock UI boot path.

Its current log is:

```text
/tmp/spotui-provision.log
```

## Current builder invocation

The current guarded builder requires:

```text
r3proii-v1.5-hmod.upt
spotui-ui-poc
spotui_daemon
ld-musl-mipsel-sf.so.1
```

A Fish-compatible example is:

```fish
tools/installer/build_spotui_tester_upt.sh \
    --base /path/to/r3proii-v1.5-hmod.upt \
    --ui engine/ui/target/mipsel-unknown-linux-musl/release/spotui-ui-poc \
    --daemon /path/to/librespot/target/mipsel-unknown-linux-musl/release/examples/spotui_daemon \
    --loader /path/to/ld-musl-mipsel-sf.so.1 \
    --output /path/to/r3proii-hmod-1.5-spotui-0.1.0-beta.5-test.4.upt \
    --runtime-output /path/to/spotui-runtime-0.1.0-beta.5-test.4.tar.xz
```

Neither output path may already exist.

The builder does not flash or write to a connected device.

## Current build stages

The guarded builder performs nine major stages:

1. verifies and extracts the exact HMOD v1.5 input;
2. extracts and patches the verified rootfs;
3. creates the private-data-free SD-card runtime payload;
4. builds a bounded reproducible LZO SquashFS;
5. generates and verifies the rootfs chunk chain;
6. packages the tester firmware;
7. re-extracts the generated image;
8. verifies the complete SpotUI firmware/runtime integration;
9. copies the verified runtime output and prints final checksums.

The builder refuses output that exceeds its safe rootfs size limit.

## Integrity checks

The current builder verifies, as applicable:

- base firmware SHA-256;
- source rootfs SHA-256, MD5, and size;
- kernel SHA-256;
- expected `hiby_player`;
- expected player wrapper;
- backup player;
- all four Stream media layouts;
- SpotUI artwork;
- preserved Qobuz artwork;
- SpotUI and Qobuz localization state;
- native preparation labels;
- firmware-side provisioner;
- runtime file hashes;
- runtime version metadata;
- rootfs compression and block size;
- rootfs size and MD5 OTA metadata;
- rootfs chunk-chain filenames;
- complete rootfs chunk manifest;
- final re-extracted firmware contents.

Do not bypass a failed verification merely to produce a `.upt`.

## Kernel verification

The current HMOD-based builder does not modify the kernel.

The verified kernel SHA-256 is:

```text
a00fd923f1480861de742a42a038f3f21d7605a9220c3cc86bdf7f4a64fc4541
```

The builder reconstructs the packaged `xImage.*` chunks and rejects the output
if the kernel differs.

## `0.1.0-beta.5-test.4` validated build

The exact development firmware/runtime pair tested on the maintainer
R3 Pro II on 2026-10-08 had:

```text
Firmware SHA-256: 0cb6a9a2556906cf55d7f68e7963ea8b615c36f148d6d58623dfaf2054895617
Firmware MD5:     0e6e35bf5ccb36202c328790137b934d

Runtime SHA-256:  fb617b091da44a2bb71b78cbf73ebec35707021c826bfe29f7101f0c13758b9b

Rootfs SHA-256:   a34106879bac9bf6c4ef4fffeeafca8b62a036b8d44fbb784f7ab353bbe98f6c
Rootfs MD5:       5ccc8bd6c7b6698d83b2ffde71fc6c08
Rootfs size:      37376000 bytes

Kernel SHA-256:   a00fd923f1480861de742a42a038f3f21d7605a9220c3cc86bdf7f4a64fc4541
Player SHA-256:   b3e787645d86bcf887699f53810e456bb8b7e9cd975014b46e421996255ae520
```

The packaged runtime version is:

```text
spotui-0.1.0-beta.5-test.4+hmod-1.5
```

The pair passed packaged-install, managed-upgrade, cold-boot, dedicated-tile,
Qobuz-preservation, playback, readiness, suspend/resume handoff, touchscreen,
return-to-HiBy, and integrity testing.

See
[0.1.0-beta.5-test.4 development checkpoint](releases/0.1.0-beta.5-test.4.md)
for the complete checkpoint record.

This development pair is not automatically a public release bundle.

## Flashing precautions

Before flashing:

1. keep official recovery firmware for the exact R3 Pro II;
2. verify the generated firmware and runtime hashes;
3. confirm the firmware and runtime belong to the same build;
4. copy the matched pair to the SD card using the installer workflow;
5. ensure the battery is sufficiently charged;
6. use only the normal HiBy R3 Pro II update procedure;
7. do not interrupt power while the updater is running.

After flashing, allow the stock interface and background provisioning to
initialize before launching SpotUI.

Inspect:

```text
/tmp/spotui-provision.log
```

before assuming the runtime has been installed or upgraded successfully.

For the complete test sequence, see
[Beta testing SpotUI](testing.md).

## Historical Qobuz-replacement builder

The repository still contains:

```text
tools/firmware/build_spotui_branded_upt.sh
```

This is an older, narrowly pinned firmware-development tool from the
pre-dedicated-tile phase of SpotUI.

It packages a privately prepared firmware tree in which the stock Qobuz tile,
Qobuz artwork paths, and Qobuz localization entry were repurposed for SpotUI.

That behavior is **historical** and is not the current firmware integration.

Do not use this builder to create a current `beta.5-test.4` firmware image.

It remains useful as a record of the earlier firmware research and verified
OTA/rootfs packaging process.

### Historical expected layout

The old builder used a private tree such as:

```text
$HOME/hiby-r3proii-mod/
├── known-good/
│   └── r3proii-spotui-qobuz-direct-working-audio.upt
├── squashfs-root/
│   ├── usr/bin/hiby_player
│   ├── usr/bin/hiby_player.sh
│   ├── usr/bin/hiby_player.bak
│   ├── usr/resource/litegui/theme1/stream_media/qobuz.png
│   ├── usr/resource/litegui/theme1/stream_media/qobuz_s.png
│   ├── usr/resource/litegui/theme2/stream_media/qobuz.png
│   ├── usr/resource/litegui/theme2/stream_media/qobuz_s.png
│   └── usr/resource/str/<language>/tidal.ini
└── r3proii-spotui-branded-icon-v5.upt
```

The Qobuz filenames above describe that older build only.

### Historical tested build record

The final on-device-tested output of that older workflow, produced on
2026-07-17, was:

```text
Filename: r3proii-spotui-branded-icon-v5.upt
MD5:      264a2847d5f66467cc8626db8ac73024
SHA-256:  2429a2c2977602dd2e68d777c858aa275908910908797b1e4b7d55b349a03e2a
```

That image was tested for the then-current Qobuz-replacement launcher artwork,
launcher caption, startup, and audio behavior.

Do not interpret this historical hash or integration model as the current
SpotUI firmware.

## Required host tools

The current HMOD installer builder checks for its required host tools
automatically.

They include standard Linux firmware/archive tooling such as:

```text
7z
awk
cmp
file
find
install
md5sum
mksquashfs
python3
sha256sum
sort
split
stat
tar
touch
unsquashfs
xorriso
xz
```

The builder also requires GNU tar for reproducible runtime creation and
SquashFS tooling with reproducible timestamp support.

## Public repository and release policy

Do not commit:

- proprietary firmware images;
- extracted proprietary binaries or complete rootfs trees;
- generated `.upt` files;
- generated runtime archives;
- compiled MIPS binaries;
- Spotify credentials or tokens;
- WiFi credentials;
- device snapshots or private backups.

Source code, original project assets, checksums, documentation, and tools that
do not redistribute proprietary firmware belong in the repository.

A locally built and device-tested development image is not automatically a
public release candidate.

Before public distribution, follow
[Tester release bundle workflow](release-bundle.md) and validate the exact
privacy-reviewed archive intended for upload.
