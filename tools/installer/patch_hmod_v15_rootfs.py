#!/usr/bin/env python3
"""Apply the reviewed SpotUI launcher integration to an HMOD v1.5 rootfs."""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path


HMOD_V15_PLAYER_SHA256 = (
    "b4ca19e33dc79c250115f37b23c445c4eb7f42adf1a2198e26109ef2b79dcb09"
)
SPOTUI_PLAYER_SHA256 = (
    "b3e787645d86bcf887699f53810e456bb8b7e9cd975014b46e421996255ae520"
)
HMOD_V15_PLAYER_SH_SHA256 = (
    "1ed03a80239032c6d363e8bdc9b6485dacac086e2bfe61403866a7c40fd25857"
)
SPOTUI_PLAYER_SH_SHA256 = (
    "51146afdfc677778127a268876690c0e1ddece312f459c83bb88eeb75342527c"
)
BACKUP_PLAYER_SHA256 = (
    "4df2dcd0b23c233da37a25853b8a1843dc93a218ff9ec251abd88466443a664d"
)

# These eight replacements are the complete dedicated SpotUI launcher delta
# from the published HMOD v1.5 player. Qobuz code and registration are left
# untouched. The injected callback first displays a stock native HiBy notice,
# then signals the prestarted lightweight broker with unlink(2). Verified
# zero padding between stock functions holds the small cleanup/return stub.
# Dormant Stream Media AirPlay registration identity storage is reused for
# the dedicated SpotUI tile.
PLAYER_PATCHES = (
    (
        0x153D60,
        bytes.fromhex(
            "c8 ff bd 27 2c 00 b0 af 34 00 bf af 30 00 b1 af "
            "50 00 b1 8c 01 00 10 24 00 41 1d 0c 28 00 24 8e "
            "06 00 40 14 34 00 bf 8f"
        ),
        bytes.fromhex(
            "e8 ff bd 27 14 00 bf af 10 00 a0 af 50 00 a4 8c "
            "82 00 05 3c 52 9e a6 24 d0 9f a5 24 70 52 12 0c "
            "d0 07 07 24 b1 ab 15 08"
        ),
    ),
    (
        0x16AEC4,
        bytes(28),
        bytes.fromhex(
            "94 00 04 3c 10 9c 23 0c 04 23 84 24 14 00 bf 8f "
            "18 00 bd 27 08 00 e0 03 00 00 02 24"
        ),
    ),
    (
        0x419E52,
        bytes(14),
        b"spotprep\x00" + bytes(5),
    ),
    (
        0x532304,
        bytes(28),
        b"/tmp/spotui.launch" + bytes(10),
    ),
    (
        0x53395C,
        b"airplay",
        b"spotui\x00",
    ),
    (
        0x533994,
        bytes.fromhex("80 ae 56 00"),
        bytes.fromhex("60 3d 55 00"),
    ),
    (
        0x533B9C,
        b"airplay",
        b"spotui\x00",
    ),
    (
        0x533BD4,
        bytes.fromhex("80 b0 56"),
        bytes.fromhex("60 3d 55"),
    ),
)

PLAYER_SH_OLD = b"/usr/bin/hiby_player\nsleep 1\nreboot"
PLAYER_SH_NEW = b"""SPOTUI_LAUNCH_MARKER=/tmp/spotui.launch
: > "$SPOTUI_LAUNCH_MARKER"
(
    while [ -e "$SPOTUI_LAUNCH_MARKER" ]; do
        sleep 0.25
    done
    while [ ! -x /usr/data/start_spotui.sh ]; do
        sleep 0.25
    done
    exec sh /usr/data/start_spotui.sh
) &
SPOTUI_LAUNCH_BROKER_PID=$!

/usr/bin/hiby_player
kill "$SPOTUI_LAUNCH_BROKER_PID" 2>/dev/null || true
wait "$SPOTUI_LAUNCH_BROKER_PID" 2>/dev/null || true
rm -f "$SPOTUI_LAUNCH_MARKER"
sleep 1
# SpotUI GUI-launch guard:
# If SpotUI was launched from its dedicated tile, hiby_player may exit intentionally.
# Do not reboot while the SpotUI launcher/UI/daemon is alive.
if ps | grep -q "[s]tart_spotui"; then
    exit 0
fi
if ps | grep -q "[s]potui-ui-poc"; then
    exit 0
fi
if ps | grep -q "[s]potui_daemon"; then
    exit 0
fi

reboot
"""

LABEL_HASHES = {
    "english": (
        "4b823faef431df28f4a9b792b4d13a648827696d0662dc67df7783e46e283d2a",
        "568dcc882cccafc01b52241b2f5264e5fd627ef07b2c77857e889ae02259afe2",
    ),
    "french": (
        "392909df5611a70d0b682729884dfa4bb5a42e28f7b7c80f7c164474276cfcde",
        "6d1b695d3d06dfac881de768db821a8cf69483bfee1ebce49362b54ef0cc55ee",
    ),
    "german": (
        "49b3cf088242bb4cd7e6ae6883afae862e531a5694846ca6dfcd8bb0d58a47f0",
        "df129ac2b1d7b6a96c4abd0035267f42d160b53a32f8de07d03af45b4a6d90fa",
    ),
    "italy": (
        "3fa0272764aa6ed7eb2a2b51d977a6934379c7a661addc0e9407c6abaac5922c",
        "daa37179f5e4dd4d2d33c5785252df4ec5692188d043bb181124d5393efef594",
    ),
    "japanese": (
        "77113fab4396b2c40d6e2e25f51cf30ab3453c93ed4ac059bcfcaa4c733f707b",
        "0121474c2b142c218c86fc7e933182ac2ee1afaa582c742ed8e670bf457465b9",
    ),
    "korean": (
        "91438dc7fba323e767a64b561d2580dbfe567680760eb6bca81f9b20d135884f",
        "addb5b8fe9459aae9418a736d3259aaa676fa03ecff9e6a40b3e309b7e2c9f46",
    ),
    "poland": (
        "e792e47601b6ab8a987ccf10f37936d1f22dd08802661e9bffdec528ef49cd5c",
        "ffec0cfd7f544ca70e57d4afb22f8a0ef0ca09db4ba916f590ba42767851e765",
    ),
    "russian": (
        "f73b81c47299c160966b302145e28175c72bc6b65da6000baac00f7b089bf14b",
        "79457177cb0db31ed41c1f97328c68216cdd1e1b480521504aec37c45140078e",
    ),
    "simplified_chinese": (
        "b84711c2598a6184036558161b8278c2322e0b12e4b417ddd52768c31cdb2b1f",
        "a97b267f9cf058f4ba8c836d43687319d9eba9b5dd1fcea4f3f5b82ba79efc13",
    ),
    "spain": (
        "5722c47a2ea6f63e5566c7e60710339c7955056b31f1a26ae942fc18a277aa90",
        "e52f48ef31eb0130208f8dc12de4591df0f1f6c59e3144c62d26e04443273934",
    ),
    "thai": (
        "4228d4f504418dca63fb94340698ce8ced725ce9cb4b36a8c0bca668f316bb09",
        "7ce168d13d1c6f4f539b773893d32954956f69bc952a11cfe65095f224806bea",
    ),
    "traditional_chinese": (
        "dc77859acfabe1b323f4e30dbaaa23e16f24b04e22875d43940b7f10e2951320",
        "224602bbd54551e5701b0009680b1a16757d97977ebc7c9a554c9cd5b5addd0d",
    ),
    "ukrainian": (
        "b2bcf07b6fd8044447da40e3626578a2a4bd610d6651579709fa8003b532eef1",
        "6d6fa637d239d237cee538dd49017877d06acbe24fa0f04f43eb05dc6763086e",
    ),
}

SWITCH_HASHES = {
    "english": (
        "6f01d06fb5d3fc863cc5e66065af06334c6502c0cb6bd014d526b94ea0a93709",
        "2bd34f45475f162d002773343a5108ec42028a239eb7292e146fb5dd65e9bef6",
    ),
    "french": (
        "6aede071ee2c68b6dc03ee3366fa35152172e6dbc17731778ede4549c971a3d9",
        "c1f0c72fca6dc39495f04966f26e7469794ec8510feb372a09c7f136473258a1",
    ),
    "german": (
        "78a8b1ecfb7969fa85a3c6b4249d7fccfeb2eb8cd3eb8707cfc4b707e82ec528",
        "0cfe50e546d84a7697eb50a5c08b42411028bc2ed835e5c9184b3cde719b1252",
    ),
    "italy": (
        "fe16ca199bccf7080041926833a7e1b79199025384fd08c209eeb60dcc0e4f32",
        "bc323dd6605324b8fccdeabe63431bdff79063efab0e908180f73c4e8ebd8015",
    ),
    "japanese": (
        "7e90a409057e525f2a51fd97798a2c7abce92be60b72aaac366490267cb14b81",
        "89355803612256c9c01d3b1c45e3a0ff3cd68cd1f7fa888e5cba8cb1fd63cfb5",
    ),
    "korean": (
        "9007f9cda7d7d11aa1a7844f1e2da3da591fb83fe92b6de0302b52f6b46a2152",
        "18ab19657cacfeb77a228d7ab6f653c084cb460235903c398ba58a83166d9f26",
    ),
    "poland": (
        "eefffe821c10bf1e89ebdc04c00a4e06b5ae0e4433822ad5d5e007cecb9b218e",
        "66398273d9ccc10ca860cd2942105f677a975e81d1db6dd6b970d87a65664cac",
    ),
    "russian": (
        "3a831a8b70943d022e2eeb9fcb371ba1428d3cdff2bf86e0bb2d8f7be0cf2a89",
        "bff63f469acc63d2f228c97e2b52189461138e64803ed6c578345a263f6bed58",
    ),
    "simplified_chinese": (
        "4f3d54c541eae6a0c22e6db5a7a0f3d2557886474a94e3fc0b9838a23ec9933e",
        "4615660bcf54a8cb157bfc11b598d10c6786a6ba3330bae2386ce04148b891cb",
    ),
    "spain": (
        "d6885b48cad9875864a11bcb9e28d7d75860b1c3596b219875f1bb8d306bcdcd",
        "bcaf7114778145fe5465e8fa40a6e6d110ca9ce7c30ecc65acea5c671ce4f172",
    ),
    "thai": (
        "77ed27df26e434b0dfd0ff127da03da3f7ba27fb4e017b5963d44fc8f6edb774",
        "2dd9f51cf282b6d5c71d74d6fc356d2ec64d5f969052b314edc948d091511d86",
    ),
    "traditional_chinese": (
        "c8f23084f5f964703fbca7d1ba3856bf280d2e059afa2fd5b81d8072f1ddd064",
        "4f73d4401012b88e21a24cc49dbc4a3ea334a3a5123c24ecc28c2a1e82a8ea6d",
    ),
    "ukrainian": (
        "213429c3b172677a874f0bb8306c4e1114ce97d354675432bdc713527cd3834b",
        "60315073772b548bdbb2c51f621c711921a2341067f967a662974485b86b2f9d",
    ),
}


LAYOUT_HASHES = {
    "usr/resource/layout/theme1/hiby_stream_media.view": (
        "8dd382d1a468cc4403d718a6747941f699cd5ce6f6dab4b89309733da21d5f42",
        "4b92577f25708274dacce31eec24ca3f462d3081f026f7e03cbd4e9a42d39c74",
    ),
    "usr/resource/layout/theme1/hiby_stream_media_cn.view": (
        "f1f69be726b586adbea272661ab88353caa7e89586e640d1895a851ad4757feb",
        "318cf393dec22a2b07b776475d59389d178b648ef87a854a8bce236dfe801afa",
    ),
    "usr/resource/layout/theme2/hiby_stream_media.view": (
        "35d5231f3934347e8109e636bb9e235048aedf6c98893b199ecdef4c47c8f325",
        "b5c296156c12f3f81ed8fe3fbfaa8d71feda9ae3e3a60ca5054e625b9476b21a",
    ),
    "usr/resource/layout/theme2/hiby_stream_media_cn.view": (
        "79ff864b51cf58c168654390a3e7aa18bc11dbb4f50ed2edc52c1bad0c744f0b",
        "ce12d10fe4ad2e015496089e3425d8c4aa8434496e82b55967705d7f69f291ff",
    ),
}

def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def require_hash(path: Path, data: bytes, expected: str) -> None:
    actual = sha256(data)
    if actual != expected:
        raise RuntimeError(f"{path}: SHA-256 mismatch: {actual}")


def patch_player(root: Path) -> None:
    path = root / "usr/bin/hiby_player"
    data = bytearray(path.read_bytes())
    require_hash(path, data, HMOD_V15_PLAYER_SHA256)

    for offset, old, new in PLAYER_PATCHES:
        if len(old) != len(new):
            raise RuntimeError(f"internal patch length mismatch at {offset}")
        if data[offset : offset + len(old)] != old:
            raise RuntimeError(f"{path}: unexpected bytes at offset {offset}")
        data[offset : offset + len(old)] = new

    require_hash(path, data, SPOTUI_PLAYER_SHA256)
    path.write_bytes(data)


def patch_player_shell(root: Path) -> None:
    path = root / "usr/bin/hiby_player.sh"
    data = path.read_bytes()
    require_hash(path, data, HMOD_V15_PLAYER_SH_SHA256)
    if data.count(PLAYER_SH_OLD) != 1:
        raise RuntimeError(f"{path}: expected launcher/reboot block not found once")
    patched = data.replace(PLAYER_SH_OLD, PLAYER_SH_NEW)
    require_hash(path, patched, SPOTUI_PLAYER_SH_SHA256)
    path.write_bytes(patched)


def verify_backup_player(root: Path) -> None:
    path = root / "usr/bin/hiby_player.bak"
    require_hash(path, path.read_bytes(), BACKUP_PLAYER_SHA256)



def patch_stream_media_layouts(root: Path) -> None:
    viewgroup_token = "\"viewgroup\":{"
    qobuz_anchor = "\"name\":\"stream_media_iv_qobuz\""
    parent_anchor = "\"name\":\"vg_stream_media_hiby\""
    y_zero = "\"y\":0,"

    for relative, (input_hash, output_hash) in LAYOUT_HASHES.items():
        path = root / relative
        data = path.read_bytes()
        require_hash(path, data, input_hash)

        text = data.decode("utf-8")

        if "stream_media_vg_spotui" in text:
            raise RuntimeError(f"{path}: SpotUI tile already exists")

        if text.count(qobuz_anchor) != 1:
            raise RuntimeError(
                f"{path}: expected exactly one Qobuz image registration"
            )

        qobuz_pos = text.index(qobuz_anchor)

        tile_key_pos = text.rfind(
            viewgroup_token,
            0,
            qobuz_pos,
        )
        if tile_key_pos < 0:
            raise RuntimeError(f"{path}: Qobuz tile start not found")

        tile_start = text.rfind("\n", 0, tile_key_pos) + 1

        next_key_pos = text.find(
            viewgroup_token,
            qobuz_pos + len(qobuz_anchor),
        )
        if next_key_pos < 0:
            raise RuntimeError(f"{path}: next sibling tile not found")

        tile_end = text.rfind("\n", 0, next_key_pos) + 1
        qobuz_tile = text[tile_start:tile_end]

        qobuz_count = qobuz_tile.count("qobuz")
        if qobuz_count != 6:
            raise RuntimeError(
                f"{path}: unexpected Qobuz tile structure; "
                f"found {qobuz_count} qobuz tokens"
            )

        spotui_tile = qobuz_tile.replace("qobuz", "spotui")

        y_pos = spotui_tile.rfind(y_zero)
        if y_pos < 0:
            raise RuntimeError(
                f"{path}: SpotUI parent y coordinate not found"
            )

        spotui_tile = (
            spotui_tile[:y_pos]
            + "\"y\":204,"
            + spotui_tile[y_pos + len(y_zero):]
        )

        if text.count(parent_anchor) != 1:
            raise RuntimeError(
                f"{path}: expected exactly one Stream Media parent"
            )

        parent_pos = text.index(parent_anchor)
        insert_at = text.rfind("\n", 0, parent_pos) + 1

        patched = (
            text[:insert_at]
            + spotui_tile
            + text[insert_at:]
        )

        # Validate syntax without serializing the duplicate-key layout.
        json.loads(patched)

        checks = {
            "stream_media_iv_qobuz": 1,
            "stream_media_tv_qobuz": 1,
            "stream_media_vg_qobuz": 1,
            "stream_media_iv_spotui": 1,
            "stream_media_tv_spotui": 1,
            "stream_media_vg_spotui": 1,
        }

        for token, expected in checks.items():
            actual = patched.count(token)
            if actual != expected:
                raise RuntimeError(
                    f"{path}: expected {expected} {token}, found {actual}"
                )

        patched_data = patched.encode("utf-8")
        require_hash(path, patched_data, output_hash)
        path.write_bytes(patched_data)


def patch_labels(root: Path) -> None:
    qobuz_tag = "<qobuz>Qobuz</qobuz>"
    spotui_tag = "<spotui>SpotUI</spotui>"

    for language, (input_hash, output_hash) in LABEL_HASHES.items():
        path = root / f"usr/resource/str/{language}/tidal.ini"
        data = path.read_bytes()
        require_hash(path, data, input_hash)

        text = data.decode("utf-16le")

        if text.count(qobuz_tag) != 1:
            raise RuntimeError(
                f"{path}: expected exactly one original Qobuz label"
            )

        if spotui_tag in text:
            raise RuntimeError(
                f"{path}: SpotUI label already exists"
            )

        lines = text.splitlines(keepends=True)
        matches = []

        for index, line in enumerate(lines):
            body = line.rstrip("\r\n")
            if body.strip(" \t") == qobuz_tag:
                matches.append(index)

        if len(matches) != 1:
            raise RuntimeError(
                f"{path}: expected one Qobuz label line, "
                f"found {len(matches)}"
            )

        index = matches[0]
        line = lines[index]
        body = line.rstrip("\r\n")
        newline = line[len(body):]

        if not newline:
            raise RuntimeError(
                f"{path}: Qobuz label line has no line ending"
            )

        indent = body[:len(body) - len(body.lstrip(" \t"))]

        lines.insert(
            index + 1,
            indent + spotui_tag + newline,
        )

        patched = "".join(lines)

        if patched.count(qobuz_tag) != 1:
            raise RuntimeError(
                f"{path}: Qobuz label changed unexpectedly"
            )

        if patched.count(spotui_tag) != 1:
            raise RuntimeError(
                f"{path}: SpotUI label insertion failed"
            )

        patched_data = patched.encode("utf-16le")
        require_hash(path, patched_data, output_hash)
        path.write_bytes(patched_data)



def patch_spotui_preparing_labels(root: Path) -> None:
    closing = "</resources>\r\n".encode("utf-16le")
    addition = (
        "  <spotprep>Preparing SpotUI...</spotprep>\r\n"
    ).encode("utf-16le")
    key = "<spotprep>".encode("utf-16le")

    for language, (input_hash, output_hash) in SWITCH_HASHES.items():
        path = root / f"usr/resource/str/{language}/switch.ini"
        data = path.read_bytes()
        require_hash(path, data, input_hash)

        if key in data:
            raise RuntimeError(
                f"{path}: SpotUI preparing label already exists"
            )

        if data.count(closing) != 1:
            raise RuntimeError(
                f"{path}: expected exactly one resources closing tag"
            )

        patched = data.replace(
            closing,
            addition + closing,
            1,
        )

        require_hash(path, patched, output_hash)
        path.write_bytes(patched)


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {Path(sys.argv[0]).name} ROOTFS", file=sys.stderr)
        return 2

    root = Path(sys.argv[1]).resolve()
    if not root.is_dir():
        print(f"error: rootfs directory not found: {root}", file=sys.stderr)
        return 1

    try:
        patch_player(root)
        patch_player_shell(root)
        verify_backup_player(root)
        patch_stream_media_layouts(root)
        patch_labels(root)
        patch_spotui_preparing_labels(root)
    except (OSError, RuntimeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    print("Verified HMOD v1.5 SpotUI launcher patch complete")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
