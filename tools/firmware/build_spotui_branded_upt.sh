#!/bin/bash
set -euo pipefail

PROJECT="$HOME/hiby-r3proii-mod"
TREE="$PROJECT/squashfs-root"
BASE="$PROJECT/known-good/r3proii-spotui-qobuz-direct-working-audio.upt"
OUTPUT="$PROJECT/r3proii-spotui-branded-icon-v5.upt"

EXPECTED_BASE_SHA="533cd090db2219f9dc78c85db6f7319008afc51d4e70a3b6d95aed54465a1ca7"
EXPECTED_PLAYER_SHA="e0cfb5455eb121c04392373c2315956a3ce45a6fcfb61b52ad5e0e1c7640000b"
EXPECTED_PLAYER_SH_SHA="8cb194b4890fcfafef941e4019c53cc823cd4e8d94e4ece75077c3a861e220d5"
EXPECTED_BACKUP_SHA="4df2dcd0b23c233da37a25853b8a1843dc93a218ff9ec251abd88466443a664d"
EXPECTED_THEME1_NORMAL_ICON_SHA="02e2c4b5b10a6525b37bbc8bd84720a22e2782281db320dd4dec38aa48d803d1"
EXPECTED_THEME1_SELECTED_ICON_SHA="b17b65d5b30554ae9778521243b60d94ff4d8c98f141d06ba3b5bf4e2d46289f"
EXPECTED_THEME2_NORMAL_ICON_SHA="0c7594055506ebf0e0bac20d08bbcec7a12d89247222c4d757308cad32382218"
EXPECTED_THEME2_SELECTED_ICON_SHA="0b70d6afc5b6e0e967b893f5efe541be3fd362a3737fb4e5bb7a8dd5355f4d65"

EXPECTED_LABEL_SHAS=(
    "english:393ca4e890e51af95544602b9726f68b232ca055b8f97cd530dfbfd196d82263"
    "french:7092383e54d13fe9092736b4af1dbceb8c4112e0fb019a40f7914b08ded5ecde"
    "german:de0f6ce1e4086cadf566b0917605c400a853fabae9c4d33436ccb2b6a8fae4ca"
    "italy:8f0968e2d90c097af7ec2bc944b54eac202c3b2243e49942e4010c2ddcaeffd3"
    "japanese:bff3f3b66098b2450d9dcefdec477eb94fd7605730ae32a15c143d23d2f68c7f"
    "korean:06a8cc84351055ed3d309b5b932a3efadc19e6b4fb36b90bd29bcc363990b5df"
    "poland:28cf2c15dcd9dd95313e6df77f5651bb05fce72ca51827bf6c3f15bb42725276"
    "russian:23254f37bc9a13a19cd07d63ef42ba2e2218868a3a27de78be97f3b85b64378b"
    "simplified_chinese:a0cb25f256250497a2111befa169318a8d4e0c021ba1c0a929f26afb7908a2cc"
    "spain:255560f3f56a0768da7f9dc646a6fa70e9a29dee24a3b20563e3b47bd68e2539"
    "thai:a59f445f4f0ffc458e0b772c4494f62e03a7796fef80f750eca8ab15b6273d81"
    "traditional_chinese:d0db04412e2c19bc1c3f821d8d93ffff6f8c3750ef98dfdcd6d11bfe14f45fb9"
    "ukrainian:43d06f27e6ee517e00e6754db3d06b2b8f2d1a37175313df28bb109284470d23"
)

OLD_ROOTFS_MD5="6ca3ca937b61f65e3fb338018dbb975d"
OLD_ROOTFS_SIZE="37359616"

WORK="$(mktemp -d)"
trap "rm -rf \"$WORK\"" EXIT

fail() {
    echo "ERROR: $*" >&2
    exit 1
}

hash_file() {
    sha256sum "$1" | cut -d " " -f 1
}

hash_from_image() {
    unsquashfs -cat "$1" "$2" 2>/dev/null |
        sha256sum |
        cut -d " " -f 1
}

for command_name in \
    7z \
    mksquashfs \
    unsquashfs \
    split \
    md5sum \
    sha256sum \
    python3 \
    xorriso
do
    command -v "$command_name" >/dev/null 2>&1 ||
        fail "Missing required command: $command_name"
done

[ -d "$TREE" ] ||
    fail "Prepared filesystem tree not found"

[ -f "$BASE" ] ||
    fail "Known-good firmware not found"

[ ! -e "$OUTPUT" ] ||
    fail "Output already exists: $OUTPUT"

[ -f "$TREE/usr/bin/hiby_player.bak" ] ||
    fail "hiby_player.bak missing from firmware tree"

[ "$(hash_file "$TREE/usr/bin/hiby_player.bak")" = "$EXPECTED_BACKUP_SHA" ] ||
    fail "hiby_player.bak hash mismatch"

[ "$(hash_file "$BASE")" = "$EXPECTED_BASE_SHA" ] ||
    fail "Known-good firmware hash mismatch"

[ "$(hash_file "$TREE/usr/bin/hiby_player")" = "$EXPECTED_PLAYER_SHA" ] ||
    fail "hiby_player hash mismatch"

[ "$(hash_file "$TREE/usr/bin/hiby_player.sh")" = "$EXPECTED_PLAYER_SH_SHA" ] ||
    fail "hiby_player.sh hash mismatch"

check_tree_icon() {
    local icon_path="$1"
    local expected_hash="$2"

    [ "$(hash_file "$icon_path")" = "$expected_hash" ] ||
        fail "SpotUI icon hash mismatch: $icon_path"
}

check_tree_icon     "$TREE/usr/resource/litegui/theme1/stream_media/qobuz.png"     "$EXPECTED_THEME1_NORMAL_ICON_SHA"

check_tree_icon     "$TREE/usr/resource/litegui/theme1/stream_media/qobuz_s.png"     "$EXPECTED_THEME1_SELECTED_ICON_SHA"

check_tree_icon     "$TREE/usr/resource/litegui/theme2/stream_media/qobuz.png"     "$EXPECTED_THEME2_NORMAL_ICON_SHA"

check_tree_icon     "$TREE/usr/resource/litegui/theme2/stream_media/qobuz_s.png"     "$EXPECTED_THEME2_SELECTED_ICON_SHA"

check_tree_label() {
    local language="$1"
    local expected_hash="$2"
    local label_path="$TREE/usr/resource/str/$language/tidal.ini"

    [ -f "$label_path" ] ||
        fail "SpotUI localization file missing: $label_path"

    [ "$(hash_file "$label_path")" = "$expected_hash" ] ||
        fail "SpotUI localization hash mismatch: $label_path"
}

for label_spec in "${EXPECTED_LABEL_SHAS[@]}"
do
    language="${label_spec%%:*}"
    expected_hash="${label_spec#*:}"

    check_tree_label "$language" "$expected_hash"
done

echo "[1/6] Extracting known-good OTA template"

mkdir -p "$WORK/template"

7z x \
    "$BASE" \
    "-o$WORK/template" \
    -y \
    >/dev/null

OTA_TEMPLATE="$WORK/template/ota_v0"

[ -f "$WORK/template/ota_config.in" ] ||
    fail "Template ota_config.in missing"

[ -f "$OTA_TEMPLATE/ota_update.in" ] ||
    fail "Template ota_update.in missing"

grep -qx "img_size=$OLD_ROOTFS_SIZE" "$OTA_TEMPLATE/ota_update.in" ||
    fail "Unexpected template rootfs size"

grep -qx "img_md5=$OLD_ROOTFS_MD5" "$OTA_TEMPLATE/ota_update.in" ||
    fail "Unexpected template rootfs MD5"

echo "[2/6] Building SquashFS"

sudo mksquashfs \
    "$TREE" \
    "$WORK/rootfs.squashfs" \
    -comp lzo \
    -b 131072 \
    -noappend \
    -no-progress \
    -all-root

ROOTFS_MD5="$(md5sum "$WORK/rootfs.squashfs" | cut -d " " -f 1)"
ROOTFS_SHA="$(sha256sum "$WORK/rootfs.squashfs" | cut -d " " -f 1)"
ROOTFS_SIZE="$(stat -c %s "$WORK/rootfs.squashfs")"

echo "Rootfs size:   $ROOTFS_SIZE"
echo "Rootfs MD5:    $ROOTFS_MD5"
echo "Rootfs SHA256: $ROOTFS_SHA"

echo "[3/6] Generating verified 524288-byte chunks"

mkdir -p "$WORK/chunks"

split \
    -b 524288 \
    -a 4 \
    "$WORK/rootfs.squashfs" \
    "$WORK/chunks/chunk."

cat "$WORK"/chunks/chunk.* > "$WORK/reassembled.squashfs"

REASSEMBLED_MD5="$(
    md5sum "$WORK/reassembled.squashfs" |
        cut -d " " -f 1
)"

[ "$REASSEMBLED_MD5" = "$ROOTFS_MD5" ] ||
    fail "Chunk reassembly MD5 mismatch"

echo "[4/6] Updating OTA rootfs files"

mkdir -p "$WORK/iso_root"
cp -a "$WORK/template/." "$WORK/iso_root/"
chmod -R u+w "$WORK/iso_root"

OTA="$WORK/iso_root/ota_v0"

rm -f \
    "$OTA"/rootfs.squashfs.* \
    "$OTA"/ota_md5_rootfs.squashfs.*

sed -i \
    "s/^img_size=$OLD_ROOTFS_SIZE$/img_size=$ROOTFS_SIZE/" \
    "$OTA/ota_update.in"

sed -i \
    "s/^img_md5=$OLD_ROOTFS_MD5$/img_md5=$ROOTFS_MD5/" \
    "$OTA/ota_update.in"

PREVIOUS_MD5="$ROOTFS_MD5"
INDEX=0
: > "$WORK/rootfs-manifest"

for chunk in "$WORK"/chunks/chunk.*
do
    NUMBER="$(printf "%04d" "$INDEX")"

    cp \
        "$chunk" \
        "$OTA/rootfs.squashfs.$NUMBER.$PREVIOUS_MD5"

    CHUNK_MD5="$(
        md5sum "$chunk" |
            cut -d " " -f 1
    )"

    echo "$CHUNK_MD5" >> "$WORK/rootfs-manifest"

    PREVIOUS_MD5="$CHUNK_MD5"
    INDEX=$((INDEX + 1))
done

cp \
    "$WORK/rootfs-manifest" \
    "$OTA/ota_md5_rootfs.squashfs.$ROOTFS_MD5"

echo "Created $INDEX rootfs chunks"

echo "[5/6] Packaging firmware"

xorriso \
    -as mkisofs \
    -V CDROM \
    -J \
    -r \
    -o "$OUTPUT" \
    "$WORK/iso_root" \
    >/dev/null 2>&1

echo "[6/6] Verifying packaged firmware"

mkdir -p "$WORK/verify"

7z x \
    "$OUTPUT" \
    "-o$WORK/verify" \
    -y \
    >/dev/null

VERIFY_OTA="$WORK/verify/ota_v0"
VERIFY_ROOTFS="$WORK/verify-rootfs.squashfs"

cat "$VERIFY_OTA"/rootfs.squashfs.* > "$VERIFY_ROOTFS"

VERIFY_ROOTFS_MD5="$(
    md5sum "$VERIFY_ROOTFS" |
        cut -d " " -f 1
)"

[ "$VERIFY_ROOTFS_MD5" = "$ROOTFS_MD5" ] ||
    fail "Packaged rootfs MD5 mismatch"

grep -qx "img_size=$ROOTFS_SIZE" "$VERIFY_OTA/ota_update.in" ||
    fail "Packaged rootfs size metadata mismatch"

grep -qx "img_md5=$ROOTFS_MD5" "$VERIFY_OTA/ota_update.in" ||
    fail "Packaged rootfs MD5 metadata mismatch"

PREVIOUS_MD5="$ROOTFS_MD5"
INDEX=0
: > "$WORK/expected-manifest"

while IFS= read -r chunk
do
    NUMBER="$(printf "%04d" "$INDEX")"
    EXPECTED_NAME="rootfs.squashfs.$NUMBER.$PREVIOUS_MD5"
    ACTUAL_NAME="$(basename "$chunk")"

    [ "$ACTUAL_NAME" = "$EXPECTED_NAME" ] ||
        fail "Chunk-chain filename mismatch: $ACTUAL_NAME"

    CHUNK_MD5="$(
        md5sum "$chunk" |
            cut -d " " -f 1
    )"

    echo "$CHUNK_MD5" >> "$WORK/expected-manifest"

    PREVIOUS_MD5="$CHUNK_MD5"
    INDEX=$((INDEX + 1))
done < <(
    find "$VERIFY_OTA" \
        -maxdepth 1 \
        -type f \
        -name "rootfs.squashfs.*" |
        sort
)

cmp \
    "$WORK/expected-manifest" \
    "$VERIFY_OTA/ota_md5_rootfs.squashfs.$ROOTFS_MD5" \
    >/dev/null ||
    fail "Rootfs manifest mismatch"

unsquashfs -s "$VERIFY_ROOTFS" |
    grep -q "Compression lzo" ||
    fail "Packaged rootfs is not LZO"

unsquashfs -s "$VERIFY_ROOTFS" |
    grep -q "Block size 131072" ||
    fail "Packaged rootfs block size mismatch"

[ "$(
    hash_from_image \
        "$VERIFY_ROOTFS" \
        usr/bin/hiby_player
)" = "$EXPECTED_PLAYER_SHA" ] ||
    fail "Packaged hiby_player hash mismatch"

[ "$(
    hash_from_image \
        "$VERIFY_ROOTFS" \
        usr/bin/hiby_player.sh
)" = "$EXPECTED_PLAYER_SH_SHA" ] ||
    fail "Packaged hiby_player.sh hash mismatch"

check_packaged_icon() {
    local icon_path="$1"
    local expected_hash="$2"

    [ "$(
        hash_from_image             "$VERIFY_ROOTFS"             "$icon_path"
    )" = "$expected_hash" ] ||
        fail "Packaged SpotUI icon mismatch: $icon_path"
}

check_packaged_icon     usr/resource/litegui/theme1/stream_media/qobuz.png     "$EXPECTED_THEME1_NORMAL_ICON_SHA"

check_packaged_icon     usr/resource/litegui/theme1/stream_media/qobuz_s.png     "$EXPECTED_THEME1_SELECTED_ICON_SHA"

check_packaged_icon     usr/resource/litegui/theme2/stream_media/qobuz.png     "$EXPECTED_THEME2_NORMAL_ICON_SHA"

check_packaged_icon     usr/resource/litegui/theme2/stream_media/qobuz_s.png     "$EXPECTED_THEME2_SELECTED_ICON_SHA"

[ "$(
    hash_from_image         "$VERIFY_ROOTFS"         usr/bin/hiby_player.bak
)" = "$EXPECTED_BACKUP_SHA" ] ||
    fail "Packaged hiby_player.bak hash mismatch"

check_packaged_label() {
    local language="$1"
    local expected_hash="$2"
    local label_path="usr/resource/str/$language/tidal.ini"

    [ "$(hash_from_image "$VERIFY_ROOTFS" "$label_path")" = "$expected_hash" ] ||
        fail "Packaged SpotUI localization mismatch: $label_path"
}

for label_spec in "${EXPECTED_LABEL_SHAS[@]}"
do
    language="${label_spec%%:*}"
    expected_hash="${label_spec#*:}"

    check_packaged_label "$language" "$expected_hash"
done

echo
echo "============================================"
echo "Verified firmware build complete"
echo "============================================"
ls -lh "$OUTPUT"
md5sum "$OUTPUT"
sha256sum "$OUTPUT"

echo
echo "Rootfs metadata:"
grep -A3 "img_name=rootfs.squashfs" \
    "$VERIFY_OTA/ota_update.in"

echo
echo "Relevant packaged files:"
unsquashfs -ll "$VERIFY_ROOTFS" |
    grep -E \
        "usr/bin/hiby_player$|usr/bin/hiby_player.sh$|usr/bin/hiby_player\.bak$|stream_media/qobuz(_s)?\.png$"
