# Verified firmware build workflow

This document describes the currently verified local firmware build workflow for SpotUI on the HiBy R3 Pro II.

The repository does not contain proprietary HiBy firmware images, extracted firmware files, ready-to-flash update images, credentials, or device snapshots. You must supply and prepare those files locally from firmware and hardware that you are authorized to use.

> [!WARNING]
> Custom firmware can make a device temporarily unbootable. This workflow has only been tested on the HiBy R3 Pro II. Do not use the generated image on another model.

## Current integration

The current firmware integration repurposes the stock Qobuz entry on the Stream media screen:

- the tile launches SpotUI instead of Qobuz;
- the four Qobuz image resources are replaced by the SpotUI launcher artwork;
- the visible Qobuz caption is changed to SpotUI in all supported localization files;
- internal Qobuz widget and localization key names remain unchanged;
- the original HiBy player binary is retained as `hiby_player.bak`.

The final launcher artwork is stored in:

```text
engine/launcher/resources/spotui/
```

The original Qobuz artwork is retained for restoration in:

```text
engine/launcher/resources/qobuz-original/
```

## Builder scope

The verified builder is:

```text
tools/firmware/build_spotui_branded_upt.sh
```

It packages and verifies an already prepared firmware tree. It does not:

- download HiBy firmware;
- extract or prepare a stock firmware tree;
- patch proprietary binaries automatically;
- install SpotUI binaries into the tree;
- create credentials or WiFi configuration;
- flash the device.

The script intentionally fails when the supplied base image, player files, icon files, or localization files do not match the exact tested inputs.

## Expected local layout

The builder currently expects:

```text
$HOME/hiby-r3proii-mod/
├── known-good/
│   └── r3proii-spotui-qobuz-direct-working-audio.upt
├── squashfs-root/
│   ├── usr/bin/hiby_player
│   ├── usr/bin/hiby_player.sh
│   ├── usr/bin/hiby_player.bak
│   ├── usr/resource/litegui/theme1/stream_media/
│   │   ├── qobuz.png
│   │   └── qobuz_s.png
│   ├── usr/resource/litegui/theme2/stream_media/
│   │   ├── qobuz.png
│   │   └── qobuz_s.png
│   └── usr/resource/str/<language>/tidal.ini
└── r3proii-spotui-branded-icon-v5.upt
```

The output filename is a firmware-build iteration name. It is not the SpotUI application version.

## Required host tools

The builder checks for these commands before starting:

```text
7z
mksquashfs
unsquashfs
split
md5sum
sha256sum
python3
xorriso
```

Typical Linux packages include:

- 7-Zip or p7zip;
- squashfs-tools;
- coreutils;
- Python 3;
- xorriso.

Some filesystem packaging operations may request administrator privileges so that ownership and permissions are preserved correctly.

## Prepared firmware tree

Before running the builder, the private `squashfs-root` tree must already contain the tested SpotUI integration.

### Player files

The builder verifies exact SHA-256 hashes for:

```text
usr/bin/hiby_player
usr/bin/hiby_player.sh
usr/bin/hiby_player.bak
```

This protects against accidentally packaging an unrelated or partially modified player binary.

### Launcher artwork

Copy the four final assets from this repository into the private firmware tree:

```text
engine/launcher/resources/spotui/theme1/qobuz.png
engine/launcher/resources/spotui/theme1/qobuz_s.png
engine/launcher/resources/spotui/theme2/qobuz.png
engine/launcher/resources/spotui/theme2/qobuz_s.png
```

Destination paths:

```text
usr/resource/litegui/theme1/stream_media/qobuz.png
usr/resource/litegui/theme1/stream_media/qobuz_s.png
usr/resource/litegui/theme2/stream_media/qobuz.png
usr/resource/litegui/theme2/stream_media/qobuz_s.png
```

The artwork contains the icon and stock-matched card background. It does not contain the SpotUI caption.

### Visible launcher caption

The visible caption is rendered by HiBy from the `qobuz` localization key in each language copy of `tidal.ini`.

The prepared tree must retain the internal key while changing only its displayed value:

```xml
<qobuz>SpotUI</qobuz>
```

There are 13 checked localization files:

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

These files use UTF-16 little-endian encoding with a byte-order mark and CRLF line endings. Any editing process must preserve that format.

## Run the build

From the SpotUI repository:

```bash
tools/firmware/build_spotui_branded_upt.sh
```

The configured output is:

```text
$HOME/hiby-r3proii-mod/r3proii-spotui-branded-icon-v5.upt
```

The script refuses to overwrite an existing output file. Rename or remove an older output only after confirming that it is safely backed up.

## Build stages

The builder performs six stages:

1. Extract the known-good OTA template.
2. Build a new LZO-compressed SquashFS with a 131072-byte block size.
3. Split the root filesystem into verified 524288-byte chunks.
4. Update rootfs metadata, chunk names, and the chained MD5 manifest.
5. Package the OTA structure as a `.upt` image.
6. Re-extract the generated image and verify its contents.

## Integrity checks

Before packaging, the builder verifies:

- the known-good base firmware SHA-256;
- `hiby_player`;
- `hiby_player.sh`;
- `hiby_player.bak`;
- all four final SpotUI icon hashes;
- all 13 patched localization hashes.

After packaging, it verifies again:

- the reconstructed rootfs MD5;
- rootfs size and MD5 metadata;
- chunk-chain filenames;
- the complete chunk manifest;
- LZO compression;
- the 131072-byte SquashFS block size;
- all three player files;
- all four icon files;
- all 13 localization files.

A successful run ends with:

```text
Verified firmware build complete
```

It then prints the firmware MD5, firmware SHA-256, rootfs metadata, and relevant packaged files.

## Kernel verification

The builder inherits the kernel files from the known-good OTA template and replaces only the rootfs portion. Before flashing, independently reconstruct and compare the `xImage.*` chunks from the known-good and generated images.

The currently verified kernel SHA-256 is:

```text
a00fd923f1480861de742a42a038f3f21d7605a9220c3cc86bdf7f4a64fc4541
```

Do not flash if the reconstructed kernel differs.

## Tested build record

The final on-device-tested firmware produced on 2026-07-17 had:

```text
Filename: r3proii-spotui-branded-icon-v5.upt
MD5:      264a2847d5f66467cc8626db8ac73024
SHA-256:  2429a2c2977602dd2e68d777c858aa275908910908797b1e4b7d55b349a03e2a
```

The output was tested for:

- light-theme normal and selected launcher states;
- dark-theme normal and selected launcher states;
- stock-matched card appearance;
- a clean SpotUI caption;
- early-launch readiness behavior;
- successful SpotUI startup;
- working audio playback.

Tool versions, timestamps, or filesystem metadata can affect the final package checksum. The builder content checks are the primary safety mechanism.

## Flashing precautions

Before flashing:

1. Keep official recovery firmware for the exact device model.
2. Verify the generated SHA-256 after copying it to the SD card.
3. Rename the verified file to `r3proii.upt`.
4. Ensure the battery is sufficiently charged.
5. Use the normal HiBy update procedure for the R3 Pro II.
6. Do not interrupt power while the updater is running.

After flashing, test both UI themes, launcher selection, SpotUI startup, and audio playback.

See [Recovery notes](recovery.md) before performing firmware experiments.

## Public repository policy

Do not commit:

- proprietary firmware images;
- extracted proprietary binaries or complete rootfs trees;
- generated `.upt` files;
- Spotify credentials or tokens;
- WiFi credentials;
- device snapshots or private backups.

Only source code, original project assets, checksums, documentation, and tools that do not redistribute proprietary files belong in this repository.
