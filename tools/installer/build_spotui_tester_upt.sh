#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." >/dev/null 2>&1 && pwd)"

PATCHER="$SCRIPT_DIR/patch_hmod_v15_rootfs.py"
PROVISIONER="$REPO_ROOT/engine/firmware/S90spotui-provision"
REAL_LAUNCHER="$REPO_ROOT/engine/launcher/start_spotui.sh"
WRAPPER_LAUNCHER="$REPO_ROOT/engine/launcher/start_spotui_wrapper.sh"
RETURN_LAUNCHER="$REPO_ROOT/engine/launcher/return_to_hiby.sh"

SPOTUI_VERSION="0.1.0-beta.5-test.4"
HMOD_VERSION="1.5"
RUNTIME_VERSION="spotui-${SPOTUI_VERSION}+hmod-${HMOD_VERSION}"

EXPECTED_BASE_SHA256="631af685977877f65288e371d49f3b2839681ee4ca4713234f498519e2ab33f2"
EXPECTED_ROOTFS_SHA256="a39afd4313e8f48b36b0e8beb5ea98ef9f648f9578b5deb7c1034977ee2bf481"
EXPECTED_ROOTFS_MD5="eb377558534224d1f61a1d873aa45374"
EXPECTED_ROOTFS_SIZE="37347328"
EXPECTED_KERNEL_SHA256="a00fd923f1480861de742a42a038f3f21d7605a9220c3cc86bdf7f4a64fc4541"
EXPECTED_PLAYER_SHA256="b3e787645d86bcf887699f53810e456bb8b7e9cd975014b46e421996255ae520"
EXPECTED_PLAYER_SH_SHA256="51146afdfc677778127a268876690c0e1ddece312f459c83bb88eeb75342527c"
EXPECTED_LAYOUT_THEME1_SHA256="4b92577f25708274dacce31eec24ca3f462d3081f026f7e03cbd4e9a42d39c74"
EXPECTED_LAYOUT_THEME1_CN_SHA256="318cf393dec22a2b07b776475d59389d178b648ef87a854a8bce236dfe801afa"
EXPECTED_LAYOUT_THEME2_SHA256="b5c296156c12f3f81ed8fe3fbfaa8d71feda9ae3e3a60ca5054e625b9476b21a"
EXPECTED_LAYOUT_THEME2_CN_SHA256="ce12d10fe4ad2e015496089e3425d8c4aa8434496e82b55967705d7f69f291ff"
EXPECTED_BACKUP_SHA256="4df2dcd0b23c233da37a25853b8a1843dc93a218ff9ec251abd88466443a664d"

BASE_TIMESTAMP="2026-01-15 20:45:38"
BASE_EPOCH=1768509938
ISO_TIMESTAMP=2026011520453800
CHUNK_BYTES=520997
ROOTFS_PARTITION_BYTES=$((0x02d00000))
ROOTFS_SAFETY_MARGIN_BYTES=$((128 * 1024))
MAX_ROOTFS_BYTES=$((ROOTFS_PARTITION_BYTES - ROOTFS_SAFETY_MARGIN_BYTES))

BASE=""
UI=""
DAEMON=""
LOADER=""
OUTPUT=""
RUNTIME_OUTPUT=""

usage() {
    cat <<EOF
Usage:
  $(basename "$0") \\
    --base PATH_TO_r3proii-v1.5-hmod.upt \\
    --ui PATH_TO_spotui-ui-poc \\
    --daemon PATH_TO_spotui_daemon \\
    --loader PATH_TO_ld-musl-mipsel-sf.so.1 \\
    --output PATH_TO_NEW_spotui.upt \\
    --runtime-output PATH_TO_NEW_spotui-runtime.tar.xz

The input must be the exact published HMOD v1.5 image. Neither output path may
already exist. This command builds and verifies the firmware/runtime pair; it
never flashes or writes to a connected device.
EOF
}

fail() {
    echo "ERROR: $*" >&2
    exit 1
}

hash_file() {
    sha256sum "$1" | awk '{ print $1 }'
}

hash_from_rootfs() {
    unsquashfs -cat "$1" "$2" 2>/dev/null |
        sha256sum |
        awk '{ print $1 }'
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --base)
            [ "$#" -ge 2 ] || fail "--base requires a path"
            BASE="$2"
            shift 2
            ;;
        --ui)
            [ "$#" -ge 2 ] || fail "--ui requires a path"
            UI="$2"
            shift 2
            ;;
        --daemon)
            [ "$#" -ge 2 ] || fail "--daemon requires a path"
            DAEMON="$2"
            shift 2
            ;;
        --loader)
            [ "$#" -ge 2 ] || fail "--loader requires a path"
            LOADER="$2"
            shift 2
            ;;
        --output)
            [ "$#" -ge 2 ] || fail "--output requires a path"
            OUTPUT="$2"
            shift 2
            ;;
        --runtime-output)
            [ "$#" -ge 2 ] || fail "--runtime-output requires a path"
            RUNTIME_OUTPUT="$2"
            shift 2
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            fail "unknown argument: $1"
            ;;
    esac
done

[ -n "$BASE" ] || fail "--base is required"
[ -n "$UI" ] || fail "--ui is required"
[ -n "$DAEMON" ] || fail "--daemon is required"
[ -n "$LOADER" ] || fail "--loader is required"
[ -n "$OUTPUT" ] || fail "--output is required"
[ -n "$RUNTIME_OUTPUT" ] || fail "--runtime-output is required"

for input in "$BASE" "$UI" "$DAEMON" "$LOADER"; do
    [ -f "$input" ] || fail "input file not found: $input"
done

[ ! -e "$OUTPUT" ] || fail "output already exists: $OUTPUT"
[ -d "$(dirname "$OUTPUT")" ] || fail "output directory does not exist"
[ ! -e "$RUNTIME_OUTPUT" ] || fail "runtime output already exists: $RUNTIME_OUTPUT"
[ -d "$(dirname "$RUNTIME_OUTPUT")" ] || fail "runtime output directory does not exist"
[ "$OUTPUT" != "$RUNTIME_OUTPUT" ] || fail "firmware and runtime outputs must differ"

for source_file in \
    "$PATCHER" \
    "$PROVISIONER" \
    "$REAL_LAUNCHER" \
    "$WRAPPER_LAUNCHER" \
    "$RETURN_LAUNCHER"
do
    [ -f "$source_file" ] || fail "repository installer source missing: $source_file"
done

for command_name in \
    7z \
    awk \
    cmp \
    file \
    find \
    install \
    md5sum \
    mksquashfs \
    python3 \
    sha256sum \
    sort \
    split \
    stat \
    tar \
    touch \
    unsquashfs \
    xorriso \
    xz
do
    command -v "$command_name" >/dev/null 2>&1 ||
        fail "missing required command: $command_name"
done

tar --version 2>/dev/null | grep -q "GNU tar" ||
    fail "GNU tar is required for a reproducible runtime archive"

if mksquashfs -help-option repro-time >/dev/null 2>&1; then
    SQUASHFS_TIME_ARGS=(-repro-time "$BASE_EPOCH")
elif mksquashfs -help 2>&1 | grep -q -- '-all-time'; then
    SQUASHFS_TIME_ARGS=(-mkfs-time "$BASE_EPOCH" -all-time "$BASE_EPOCH")
else
    fail "mksquashfs lacks reproducible timestamp support"
fi

[ "$(hash_file "$BASE")" = "$EXPECTED_BASE_SHA256" ] ||
    fail "base firmware is not the published HMOD v1.5 image"

file -L "$UI" | grep -q "ELF 32-bit LSB pie executable, MIPS" ||
    fail "UI is not a 32-bit little-endian MIPS PIE executable"
file -L "$UI" | grep -q "interpreter /lib/ld-musl-mipsel-sf.so.1" ||
    fail "UI does not use the expected musl loader"

file -L "$DAEMON" | grep -q "ELF 32-bit LSB pie executable, MIPS" ||
    fail "daemon is not a 32-bit little-endian MIPS PIE executable"
file -L "$DAEMON" | grep -q "interpreter /lib/ld-musl-mipsel-sf.so.1" ||
    fail "daemon does not use the expected musl loader"

file -L "$LOADER" | grep -q "ELF 32-bit LSB shared object, MIPS" ||
    fail "loader is not a 32-bit little-endian MIPS shared object"

WORK="$(mktemp -d /tmp/spotui-tester-upt.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

INPUT="$WORK/input"
INPUT_OTA="$INPUT/ota_v0"
ORIGINAL_ROOTFS="$WORK/rootfs.original.squashfs"
ROOTFS_TREE="$WORK/rootfs"
PAYLOAD="$WORK/payload"
RUNTIME_ARCHIVE="$WORK/spotui-runtime.tar.xz"
NEW_ROOTFS="$WORK/rootfs.squashfs"
ISO_ROOT="$WORK/iso-root"
OUTPUT_OTA="$ISO_ROOT/ota_v0"

echo "[1/9] Verifying and extracting HMOD v1.5"
mkdir -p "$INPUT"
7z x "$BASE" "-o$INPUT" -y >/dev/null

[ -f "$INPUT/ota_config.in" ] || fail "base ota_config.in is missing"
[ -f "$INPUT_OTA/ota_update.in" ] || fail "base ota_update.in is missing"

cat "$INPUT_OTA"/rootfs.squashfs.* > "$ORIGINAL_ROOTFS"

[ "$(hash_file "$ORIGINAL_ROOTFS")" = "$EXPECTED_ROOTFS_SHA256" ] ||
    fail "base rootfs SHA-256 mismatch"
[ "$(md5sum "$ORIGINAL_ROOTFS" | awk '{ print $1 }')" = "$EXPECTED_ROOTFS_MD5" ] ||
    fail "base rootfs MD5 mismatch"
[ "$(stat -c %s "$ORIGINAL_ROOTFS")" = "$EXPECTED_ROOTFS_SIZE" ] ||
    fail "base rootfs size mismatch"

cat "$INPUT_OTA"/xImage.* > "$WORK/kernel"
[ "$(hash_file "$WORK/kernel")" = "$EXPECTED_KERNEL_SHA256" ] ||
    fail "base kernel hash mismatch"

grep -qx "img_size=$EXPECTED_ROOTFS_SIZE" "$INPUT_OTA/ota_update.in" ||
    fail "base rootfs size metadata mismatch"
grep -qx "img_md5=$EXPECTED_ROOTFS_MD5" "$INPUT_OTA/ota_update.in" ||
    fail "base rootfs MD5 metadata mismatch"

unsquashfs -s "$ORIGINAL_ROOTFS" | grep -q "Compression lzo" ||
    fail "base rootfs compression is not LZO"
unsquashfs -s "$ORIGINAL_ROOTFS" | grep -q "Block size 131072" ||
    fail "base rootfs block size is not 131072"

echo "[2/9] Extracting and patching the verified rootfs"
unsquashfs -f -d "$ROOTFS_TREE" "$ORIGINAL_ROOTFS" >/dev/null
python3 "$PATCHER" "$ROOTFS_TREE"

install -m 0644 \
    "$REPO_ROOT/engine/launcher/resources/spotui/theme1/spotui.png" \
    "$ROOTFS_TREE/usr/resource/litegui/theme1/stream_media/spotui.png"
install -m 0644 \
    "$REPO_ROOT/engine/launcher/resources/spotui/theme1/spotui_s.png" \
    "$ROOTFS_TREE/usr/resource/litegui/theme1/stream_media/spotui_s.png"
install -m 0644 \
    "$REPO_ROOT/engine/launcher/resources/spotui/theme2/spotui.png" \
    "$ROOTFS_TREE/usr/resource/litegui/theme2/stream_media/spotui.png"
install -m 0644 \
    "$REPO_ROOT/engine/launcher/resources/spotui/theme2/spotui_s.png" \
    "$ROOTFS_TREE/usr/resource/litegui/theme2/stream_media/spotui_s.png"

echo "[3/9] Creating the private-data-free SD runtime payload"
mkdir -p "$PAYLOAD"
install -m 0755 "$UI" "$PAYLOAD/spotui-ui-poc"
install -m 0755 "$DAEMON" "$PAYLOAD/spotui_daemon"
install -m 0555 "$LOADER" "$PAYLOAD/ld-musl-mipsel-sf.so.1"
install -m 0755 "$WRAPPER_LAUNCHER" "$PAYLOAD/start_spotui.sh"
install -m 0755 "$REAL_LAUNCHER" "$PAYLOAD/start_spotui.real.sh"
install -m 0755 "$RETURN_LAUNCHER" "$PAYLOAD/return_to_hiby.sh"
printf '%s\n' "$RUNTIME_VERSION" > "$PAYLOAD/VERSION"

(
    cd "$PAYLOAD"
    sha256sum \
        spotui-ui-poc \
        spotui_daemon \
        ld-musl-mipsel-sf.so.1 \
        start_spotui.sh \
        start_spotui.real.sh \
        return_to_hiby.sh \
        > SHA256SUMS
)

PAYLOAD_BYTES=0
for payload_file in \
    "$PAYLOAD/spotui-ui-poc" \
    "$PAYLOAD/spotui_daemon" \
    "$PAYLOAD/ld-musl-mipsel-sf.so.1" \
    "$PAYLOAD/start_spotui.sh" \
    "$PAYLOAD/start_spotui.real.sh" \
    "$PAYLOAD/return_to_hiby.sh"
do
    PAYLOAD_BYTES=$((PAYLOAD_BYTES + $(stat -c %s "$payload_file")))
done

(
    cd "$PAYLOAD"
    tar \
        --sort=name \
        --mtime="$BASE_TIMESTAMP UTC" \
        --owner=0 \
        --group=0 \
        --numeric-owner \
        -cf - \
        . |
        xz -4e -T1 --check=crc32 > "$RUNTIME_ARCHIVE"
)

xz -t "$RUNTIME_ARCHIVE" ||
    fail "runtime archive integrity check failed"
file "$RUNTIME_ARCHIVE" | grep -q "checksum CRC32" ||
    fail "runtime archive does not use the required CRC32 checksum"

INSTALLER_DIR="$ROOTFS_TREE/usr/share/spotui-installer"
mkdir -p "$INSTALLER_DIR"
printf '%s  spotui-runtime.tar.xz\n' \
    "$(hash_file "$RUNTIME_ARCHIVE")" \
    > "$INSTALLER_DIR/spotui-runtime.tar.xz.sha256"
install -m 0644 "$PAYLOAD/SHA256SUMS" "$INSTALLER_DIR/SHA256SUMS"
install -m 0644 "$PAYLOAD/VERSION" "$INSTALLER_DIR/VERSION"
printf '%s\n' "$PAYLOAD_BYTES" > "$INSTALLER_DIR/PAYLOAD_BYTES"
install -m 0755 "$PROVISIONER" "$ROOTFS_TREE/etc/init.d/S99spotui-provision"

find "$PAYLOAD" -maxdepth 1 -type f -exec \
    touch -d "$BASE_TIMESTAMP UTC" {} +
find "$INSTALLER_DIR" -maxdepth 1 -type f -exec \
    touch -d "$BASE_TIMESTAMP UTC" {} +
touch -d "$BASE_TIMESTAMP UTC" \
    "$ROOTFS_TREE/etc/init.d/S99spotui-provision" \
    "$ROOTFS_TREE/usr/bin/hiby_player" \
    "$ROOTFS_TREE/usr/bin/hiby_player.sh" \
    "$ROOTFS_TREE/usr/resource/layout/theme1/hiby_stream_media.view" \
    "$ROOTFS_TREE/usr/resource/layout/theme1/hiby_stream_media_cn.view" \
    "$ROOTFS_TREE/usr/resource/layout/theme2/hiby_stream_media.view" \
    "$ROOTFS_TREE/usr/resource/layout/theme2/hiby_stream_media_cn.view" \
    "$ROOTFS_TREE/usr/resource/litegui/theme1/stream_media/spotui.png" \
    "$ROOTFS_TREE/usr/resource/litegui/theme1/stream_media/spotui_s.png" \
    "$ROOTFS_TREE/usr/resource/litegui/theme2/stream_media/spotui.png" \
    "$ROOTFS_TREE/usr/resource/litegui/theme2/stream_media/spotui_s.png"

for language in \
    english french german italy japanese korean poland russian \
    simplified_chinese spain thai traditional_chinese ukrainian
do
    touch -d "$BASE_TIMESTAMP UTC" \
        "$ROOTFS_TREE/usr/resource/str/$language/tidal.ini"
done

echo "[4/9] Building the bounded rootfs"
mksquashfs \
    "$ROOTFS_TREE" \
    "$NEW_ROOTFS" \
    -comp lzo \
    -Xalgorithm lzo1x_999 \
    -Xcompression-level 9 \
    -b 131072 \
    -noappend \
    -no-progress \
    -all-root \
    "${SQUASHFS_TIME_ARGS[@]}" \
    >/dev/null

ROOTFS_SIZE="$(stat -c %s "$NEW_ROOTFS")"
ROOTFS_MD5="$(md5sum "$NEW_ROOTFS" | awk '{ print $1 }')"
ROOTFS_SHA256="$(hash_file "$NEW_ROOTFS")"

[ "$ROOTFS_SIZE" -le "$MAX_ROOTFS_BYTES" ] ||
    fail "rootfs is $ROOTFS_SIZE bytes; safe maximum is $MAX_ROOTFS_BYTES"

echo "Runtime payload: $(stat -c %s "$RUNTIME_ARCHIVE") compressed bytes"
echo "Rootfs:          $ROOTFS_SIZE bytes"
echo "Rootfs SHA-256:  $ROOTFS_SHA256"

echo "[5/9] Generating the verified rootfs chunk chain"
mkdir -p "$WORK/chunks"
split -b "$CHUNK_BYTES" -a 4 "$NEW_ROOTFS" "$WORK/chunks/chunk."
cat "$WORK"/chunks/chunk.* > "$WORK/rootfs.reassembled"
cmp "$NEW_ROOTFS" "$WORK/rootfs.reassembled" >/dev/null ||
    fail "generated rootfs chunks do not reassemble exactly"

cp -a "$INPUT/." "$ISO_ROOT/"
chmod -R u+w "$ISO_ROOT"
rm -f \
    "$OUTPUT_OTA"/rootfs.squashfs.* \
    "$OUTPUT_OTA"/ota_md5_rootfs.squashfs.*

sed -i \
    "s/^img_size=$EXPECTED_ROOTFS_SIZE$/img_size=$ROOTFS_SIZE/" \
    "$OUTPUT_OTA/ota_update.in"
sed -i \
    "s/^img_md5=$EXPECTED_ROOTFS_MD5$/img_md5=$ROOTFS_MD5/" \
    "$OUTPUT_OTA/ota_update.in"

grep -qx "img_size=$ROOTFS_SIZE" "$OUTPUT_OTA/ota_update.in" ||
    fail "new rootfs size metadata was not updated"
grep -qx "img_md5=$ROOTFS_MD5" "$OUTPUT_OTA/ota_update.in" ||
    fail "new rootfs MD5 metadata was not updated"

PREVIOUS_MD5="$ROOTFS_MD5"
INDEX=0
: > "$WORK/rootfs-manifest"

for chunk in "$WORK"/chunks/chunk.*; do
    NUMBER="$(printf '%04d' "$INDEX")"
    cp "$chunk" "$OUTPUT_OTA/rootfs.squashfs.$NUMBER.$PREVIOUS_MD5"
    CHUNK_MD5="$(md5sum "$chunk" | awk '{ print $1 }')"
    echo "$CHUNK_MD5" >> "$WORK/rootfs-manifest"
    PREVIOUS_MD5="$CHUNK_MD5"
    INDEX=$((INDEX + 1))
done

cp \
    "$WORK/rootfs-manifest" \
    "$OUTPUT_OTA/ota_md5_rootfs.squashfs.$ROOTFS_MD5"

echo "[6/9] Packaging the tester firmware"
xorriso \
    -as mkisofs \
    -V CDROM \
    -J \
    -r \
    "--modification-date=$ISO_TIMESTAMP" \
    --set_all_file_dates "$ISO_TIMESTAMP" \
    -o "$OUTPUT" \
    "$ISO_ROOT" \
    >/dev/null 2>&1

echo "[7/9] Re-extracting the packaged image"
VERIFY="$WORK/verify"
VERIFY_OTA="$VERIFY/ota_v0"
VERIFY_ROOTFS="$WORK/verify-rootfs.squashfs"
mkdir -p "$VERIFY"
7z x "$OUTPUT" "-o$VERIFY" -y >/dev/null
cat "$VERIFY_OTA"/rootfs.squashfs.* > "$VERIFY_ROOTFS"

[ "$(hash_file "$VERIFY_ROOTFS")" = "$ROOTFS_SHA256" ] ||
    fail "packaged rootfs SHA-256 mismatch"
[ "$(md5sum "$VERIFY_ROOTFS" | awk '{ print $1 }')" = "$ROOTFS_MD5" ] ||
    fail "packaged rootfs MD5 mismatch"
[ "$(stat -c %s "$VERIFY_ROOTFS")" = "$ROOTFS_SIZE" ] ||
    fail "packaged rootfs size mismatch"

grep -qx "img_size=$ROOTFS_SIZE" "$VERIFY_OTA/ota_update.in" ||
    fail "packaged rootfs size metadata mismatch"
grep -qx "img_md5=$ROOTFS_MD5" "$VERIFY_OTA/ota_update.in" ||
    fail "packaged rootfs MD5 metadata mismatch"

cat "$VERIFY_OTA"/xImage.* > "$WORK/verify-kernel"
[ "$(hash_file "$WORK/verify-kernel")" = "$EXPECTED_KERNEL_SHA256" ] ||
    fail "packaged kernel differs from HMOD v1.5"

PREVIOUS_MD5="$ROOTFS_MD5"
INDEX=0
: > "$WORK/verify-manifest"

while IFS= read -r chunk; do
    NUMBER="$(printf '%04d' "$INDEX")"
    EXPECTED_NAME="rootfs.squashfs.$NUMBER.$PREVIOUS_MD5"
    [ "$(basename "$chunk")" = "$EXPECTED_NAME" ] ||
        fail "packaged rootfs chunk-chain filename mismatch"
    CHUNK_MD5="$(md5sum "$chunk" | awk '{ print $1 }')"
    echo "$CHUNK_MD5" >> "$WORK/verify-manifest"
    PREVIOUS_MD5="$CHUNK_MD5"
    INDEX=$((INDEX + 1))
done < <(
    find "$VERIFY_OTA" -maxdepth 1 -type f -name 'rootfs.squashfs.*' | sort
)

cmp \
    "$WORK/verify-manifest" \
    "$VERIFY_OTA/ota_md5_rootfs.squashfs.$ROOTFS_MD5" \
    >/dev/null || fail "packaged rootfs manifest mismatch"

echo "[8/9] Verifying SpotUI integration and runtime payload"
unsquashfs -s "$VERIFY_ROOTFS" | grep -q "Compression lzo" ||
    fail "packaged rootfs compression is not LZO"
unsquashfs -s "$VERIFY_ROOTFS" | grep -q "Block size 131072" ||
    fail "packaged rootfs block size is not 131072"

[ "$(hash_from_rootfs "$VERIFY_ROOTFS" usr/bin/hiby_player)" = "$EXPECTED_PLAYER_SHA256" ] ||
    fail "packaged SpotUI player hook mismatch"
[ "$(hash_from_rootfs "$VERIFY_ROOTFS" usr/bin/hiby_player.sh)" = "$EXPECTED_PLAYER_SH_SHA256" ] ||
    fail "packaged player wrapper mismatch"
[ "$(hash_from_rootfs "$VERIFY_ROOTFS" usr/bin/hiby_player.bak)" = "$EXPECTED_BACKUP_SHA256" ] ||
    fail "packaged backup player mismatch"
[ "$(hash_from_rootfs "$VERIFY_ROOTFS" etc/init.d/S99spotui-provision)" = "$(hash_file "$PROVISIONER")" ] ||
    fail "packaged background provisioner mismatch"

[ "$(hash_from_rootfs "$VERIFY_ROOTFS" usr/resource/layout/theme1/hiby_stream_media.view)" = "$EXPECTED_LAYOUT_THEME1_SHA256" ] ||
    fail "packaged theme1 Stream Media layout mismatch"
[ "$(hash_from_rootfs "$VERIFY_ROOTFS" usr/resource/layout/theme1/hiby_stream_media_cn.view)" = "$EXPECTED_LAYOUT_THEME1_CN_SHA256" ] ||
    fail "packaged theme1 CN Stream Media layout mismatch"
[ "$(hash_from_rootfs "$VERIFY_ROOTFS" usr/resource/layout/theme2/hiby_stream_media.view)" = "$EXPECTED_LAYOUT_THEME2_SHA256" ] ||
    fail "packaged theme2 Stream Media layout mismatch"
[ "$(hash_from_rootfs "$VERIFY_ROOTFS" usr/resource/layout/theme2/hiby_stream_media_cn.view)" = "$EXPECTED_LAYOUT_THEME2_CN_SHA256" ] ||
    fail "packaged theme2 CN Stream Media layout mismatch"

for icon in \
    theme1/spotui.png \
    theme1/spotui_s.png \
    theme2/spotui.png \
    theme2/spotui_s.png
do
    ICON_PATH="usr/resource/litegui/${icon%/*}/stream_media/${icon##*/}"

    [ "$(hash_from_rootfs "$VERIFY_ROOTFS" "$ICON_PATH")" = \
      "$(hash_file "$REPO_ROOT/engine/launcher/resources/spotui/$icon")" ] ||
        fail "packaged SpotUI icon mismatch: $icon"
done

for icon in \
    theme1/qobuz.png \
    theme1/qobuz_s.png \
    theme2/qobuz.png \
    theme2/qobuz_s.png
do
    ICON_PATH="usr/resource/litegui/${icon%/*}/stream_media/${icon##*/}"

    [ "$(hash_from_rootfs "$VERIFY_ROOTFS" "$ICON_PATH")" = \
      "$(hash_from_rootfs "$ORIGINAL_ROOTFS" "$ICON_PATH")" ] ||
        fail "packaged Qobuz artwork changed unexpectedly: $icon"
done

for installer_file in SHA256SUMS VERSION PAYLOAD_BYTES spotui-runtime.tar.xz.sha256; do
    [ "$(hash_from_rootfs "$VERIFY_ROOTFS" "usr/share/spotui-installer/$installer_file")" = \
      "$(hash_file "$INSTALLER_DIR/$installer_file")" ] ||
        fail "packaged installer metadata mismatch: $installer_file"
done

mkdir -p "$WORK/verify-payload"
xz -dc "$RUNTIME_ARCHIVE" |
    tar -xf - -C "$WORK/verify-payload"
(
    cd "$WORK/verify-payload"
    sha256sum -c SHA256SUMS >/dev/null
)

[ "$(hash_file "$WORK/verify-payload/spotui-ui-poc")" = "$(hash_file "$UI")" ] ||
    fail "packaged UI payload mismatch"
[ "$(hash_file "$WORK/verify-payload/spotui_daemon")" = "$(hash_file "$DAEMON")" ] ||
    fail "packaged daemon payload mismatch"
[ "$(hash_file "$WORK/verify-payload/ld-musl-mipsel-sf.so.1")" = "$(hash_file "$LOADER")" ] ||
    fail "packaged loader payload mismatch"
[ "$(hash_file "$WORK/verify-payload/start_spotui.sh")" = "$(hash_file "$WRAPPER_LAUNCHER")" ] ||
    fail "packaged wrapper launcher mismatch"
[ "$(hash_file "$WORK/verify-payload/start_spotui.real.sh")" = "$(hash_file "$REAL_LAUNCHER")" ] ||
    fail "packaged real launcher mismatch"
[ "$(hash_file "$WORK/verify-payload/return_to_hiby.sh")" = "$(hash_file "$RETURN_LAUNCHER")" ] ||
    fail "packaged return launcher mismatch"

PAYLOAD_FILE_COUNT="$(find "$WORK/verify-payload" -maxdepth 1 -type f | wc -l)"
[ "$PAYLOAD_FILE_COUNT" -eq 8 ] ||
    fail "runtime archive contains unexpected files"

cp "$RUNTIME_ARCHIVE" "$RUNTIME_OUTPUT"

echo "[9/9] Verified tester firmware complete"
echo "Output:          $OUTPUT"
echo "Output MD5:      $(md5sum "$OUTPUT" | awk '{ print $1 }')"
echo "Output SHA-256:  $(hash_file "$OUTPUT")"
echo "Runtime output:  $RUNTIME_OUTPUT"
echo "Runtime SHA-256: $(hash_file "$RUNTIME_OUTPUT")"
echo "Rootfs size:     $ROOTFS_SIZE / $ROOTFS_PARTITION_BYTES bytes"
echo "Runtime version: $RUNTIME_VERSION"
echo
echo "This image has not been flashed. Review and device testing are still required."
