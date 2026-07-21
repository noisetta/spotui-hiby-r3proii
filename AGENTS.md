# SpotUI Repository Instructions

## Scope

These instructions apply to the entire repository.

SpotUI is a standalone Spotify client for the HiBy R3 Pro II. It consists of
a Rust framebuffer/touchscreen UI and a Rust control daemon built against
librespot 0.8.0. The UI communicates with the daemon through
`/tmp/spotui.sock`, and the daemon starts a dedicated `aplay` subprocess for
each active playback sink.

## Canonical Sources and Artifacts

- Canonical daemon source:
  `apps/spotify/daemon/spotui_daemon.rs`
- UI source:
  `engine/ui/src/main.rs`
- Librespot build-tree daemon copy:
  `~/mips-toolchain/librespot/examples/spotui_daemon.rs`
- Built UI:
  `engine/ui/target/mipsel-unknown-linux-musl/release/spotui-ui-poc`
- Built daemon:
  `~/mips-toolchain/librespot/target/mipsel-unknown-linux-musl/release/examples/spotui_daemon`
- Device UI:
  `/usr/data/spotui-ui-poc`
- Device daemon:
  `/usr/data/spotui_daemon`
- Immediate rollback files:
  `/usr/data/spotui-ui-poc.previous` and
  `/usr/data/spotui_daemon.previous`

Always edit the canonical daemon source in this repository. Copy it into the
librespot build tree immediately before building, then verify that the two
source hashes match.

## Source and Protocol Constraints

- Never run `cargo fmt`. These large existing Rust files intentionally retain
  their current formatting, and repository-wide formatting creates unrelated
  diffs.
- Keep playback-control sends fire-and-forget. Commands that require replies
  should continue using the request path.
- Treat the UI and daemon wire protocol as a matched interface. When changing
  load commands, queue status, or request IDs, update and deploy both sides as
  a compatible pair.
- Preserve latest-tap-wins behavior: row highlighting is immediate, rapid taps
  coalesce to the newest request, stale daemon requests do not reach
  `player.load()`, and queue acknowledgement matches the newest request ID.
- Timestamp-seed UI load request IDs so a UI-only restart cannot generate IDs
  older than those remembered by a still-running daemon.
- Automatic queue advancement retains the active queue request ID; it does not
  create a new user-load request.
- Preserve source-aware highlighting when switching between Liked Songs and
  playlists.
- Plain `LOAD` and `STOP` clear the active playback queue.
- Preserve unavailable-track recovery: clear the queue, stop the player, and
  report `STOPPED`.
- Avoid adding a background command worker unless device evidence shows the
  simpler connection-task, debounce, request-ID, and stale-check design is
  insufficient.

## Build Commands

Before building the daemon:

```text
cp ~/hiby-standalone-client-public/apps/spotify/daemon/spotui_daemon.rs \
   ~/mips-toolchain/librespot/examples/spotui_daemon.rs

sha256sum \
  ~/hiby-standalone-client-public/apps/spotify/daemon/spotui_daemon.rs \
  ~/mips-toolchain/librespot/examples/spotui_daemon.rs
```

Build the daemon from `~/mips-toolchain/librespot`:

```text
env RUSTFLAGS='-C strip=symbols' \
  cargo +nightly build \
  --release \
  --example spotui_daemon \
  -Z build-std=std,panic_abort \
  --target mipsel-unknown-linux-musl \
  --no-default-features \
  --features 'rustls-tls-webpki-roots,with-libmdns'
```

Build the UI from `engine/ui`:

```text
cargo +nightly build \
  --release \
  -Z build-std=std,panic_abort \
  --target mipsel-unknown-linux-musl
```

After each build, verify the output with `file` and `sha256sum`.

## Controlled Change Workflow

Use this sequence for source changes:

1. Inspect the exact current source and repository state.
2. Make one controlled change.
3. Run `git diff --check`.
4. Inspect the complete relevant diff.
5. Build every affected binary.
6. Verify binary type and hash.
7. Archive the current device binaries on the laptop.
8. Upload new binaries to `/usr/data` with `.new` suffixes.
9. Verify device hashes before activation.
10. Rotate active binaries with `mv`, retaining a compatible rollback pair.
11. Reboot when the launcher or matched pair requires it.
12. Test manually and inspect `/tmp/spotui-ui.log` and `/tmp/daemon.log`.
13. Commit only after device testing passes.

Do not deploy or commit generated Rust that has not been inspected. Do not
push a milestone until the user approves or requests the push.

## Device Storage and Deployment Safety

- `/usr/data` is only about 35.8 MB, while the daemon is about 12 MB. The
  device cannot safely retain three daemon binaries.
- Before uploading a new daemon, archive the active and rollback binaries on
  the laptop, verify their hashes, and remove only the obsolete device
  rollback files needed to create staging space.
- Stage large binaries directly in `/usr/data` as `.new` files.
- Never stage the daemon in `/tmp`; it is RAM-backed and has previously caused
  memory pressure and ADB instability.
- Verify every staged hash before rotating files.
- Keep UI and daemon rollback files protocol-compatible.
- After activation, verify the active and rollback hashes, reboot if needed,
  and confirm `spotui-ui-poc`, `spotui_daemon`, and `/tmp/spotui.sock` are
  healthy. Confirm `aplay` appears while audio is playing and exits while
  playback is paused or stopped.
- Report what was removed from the device and where its verified laptop backup
  is stored.

## Device Test Expectations

For playback or queue changes, test at least:

- Normal and rapid taps in Liked Songs and playlists
- Automatic advancement and final queue completion
- Pause, resume, and seeking
- Switching between Liked Songs and playlist sources
- Queue highlighting and acknowledgement
- Unavailable-track recovery when reproducible
- UI-only restart while the daemon remains alive
- Full device reboot

Correlate UI queued/dispatched request logs with daemon request logs and
librespot `Loading` events. A latest-tap burst passes only when intermediate
rapid taps do not reach `player.load()`.

## Shell Notes

The user's interactive host shell is Fish. Commands intended for the user to
paste into that shell must be Fish-compatible; avoid Bash heredocs and
Bash-only syntax. Commands inside `adb shell '...'` may use POSIX shell syntax.
