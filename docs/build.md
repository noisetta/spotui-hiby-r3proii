# Build and deploy SpotUI

This document records the tested local cross-build and incremental deployment
workflow for SpotUI on the HiBy R3 Pro II.

The current device-tested development checkpoint is
`0.1.0-beta.5-test.4`. The latest publicly packaged tester prerelease remains
`0.1.0-beta.2`.

The commands below assume:

- the public SpotUI repository is cloned at
  `$HOME/hiby-standalone-client-public`;
- a working Rust nightly toolchain is installed;
- a working `mipsel-unknown-linux-musl` cross-build environment is already
  configured;
- the local librespot source tree is available at
  `$HOME/mips-toolchain/librespot`;
- ADB can reach the HiBy R3 Pro II;
- the device already has a compatible SpotUI firmware/runtime installation.

The repository does not include proprietary firmware, device credentials,
Spotify credentials, or a complete MIPS toolchain.

> [!WARNING]
> These instructions are specific to the HiBy R3 Pro II development setup used
> for SpotUI. Do not deploy the binaries to another model unless that device has
> been independently tested.

## Source locations

The canonical SpotUI interface source is:

```text
engine/ui/src/main.rs
```

The canonical daemon source is:

```text
apps/spotify/daemon/spotui_daemon.rs
```

The daemon is compiled inside a local librespot source tree because it uses
librespot as an example binary.

## Version and checkpoint boundary

The source checkpoint name, runtime version, and compiled Rust package version
are related but are not automatically the same thing.

Before building the UI, inspect its package version:

```fish
rg '^version = ' \
    ~/hiby-standalone-client-public/engine/ui/Cargo.toml
```

The on-device Diagnostics version label is compiled from that Cargo package
version.

Do not assume that a binary belongs to a development checkpoint solely because
of the Diagnostics label. For development validation, also record:

- the source commit or tag;
- the UI SHA-256;
- the daemon SHA-256;
- the runtime or firmware checkpoint being tested.

The `0.1.0-beta.5-test.4` checkpoint identifies an exact validated development
state. Source-level package-version changes should be reviewed separately from
documentation-only synchronization.

## Rust toolchain

The build uses Rust nightly and Cargo's unstable `build-std` support.

Install or update the required nightly components:

```fish
rustup toolchain install nightly --component rust-src
rustup component add rust-src --toolchain nightly
```

Confirm the toolchains:

```fish
rustup toolchain list
rustc +nightly --version
cargo +nightly --version
```

The custom MIPS environment must also provide the linker, archiver, C runtime,
and target configuration required by `mipsel-unknown-linux-musl`. Those details
are currently external to this repository.

## Build the interface

From the repository:

```fish
cd ~/hiby-standalone-client-public/engine/ui

cargo +nightly build \
    --release \
    -Z build-std=std,panic_abort \
    --target mipsel-unknown-linux-musl
```

The resulting interface binary is:

```text
engine/ui/target/mipsel-unknown-linux-musl/release/spotui-ui-poc
```

Verify it:

```fish
file \
    target/mipsel-unknown-linux-musl/release/spotui-ui-poc

sha256sum \
    target/mipsel-unknown-linux-musl/release/spotui-ui-poc
```

The release profile is configured for a small binary with link-time
optimization, symbol stripping, and abort-on-panic behavior.

## Prepare the daemon build dependency

The SpotUI daemon parses Spotify profile responses for playlist browsing and
therefore requires `serde_json` as a direct development dependency in the
local librespot checkout.

Ensure:

```text
$HOME/mips-toolchain/librespot/Cargo.toml
```

contains:

```toml
[dev-dependencies]
serde_json = "1.0"
```

This changes only the separate local librespot checkout.

## Prepare the daemon source

The daemon is built against the local librespot tree. The current source
targets the librespot 0.8.0 API.

Copy the canonical daemon source into the librespot examples directory:

```fish
cp \
    ~/hiby-standalone-client-public/apps/spotify/daemon/spotui_daemon.rs \
    ~/mips-toolchain/librespot/examples/spotui_daemon.rs
```

Confirm that the canonical and build-tree copies match:

```fish
sha256sum \
    ~/hiby-standalone-client-public/apps/spotify/daemon/spotui_daemon.rs \
    ~/mips-toolchain/librespot/examples/spotui_daemon.rs
```

Both hashes must be identical before building.

## Build the daemon

```fish
cd ~/mips-toolchain/librespot

env RUSTFLAGS='-C strip=symbols' \
    cargo +nightly build \
    --release \
    --example spotui_daemon \
    -Z build-std=std,panic_abort \
    --target mipsel-unknown-linux-musl \
    --no-default-features \
    --features 'rustls-tls-webpki-roots,with-libmdns'
```

The resulting daemon binary is:

```text
$HOME/mips-toolchain/librespot/target/mipsel-unknown-linux-musl/release/examples/spotui_daemon
```

Verify it:

```fish
file \
    target/mipsel-unknown-linux-musl/release/examples/spotui_daemon

sha256sum \
    target/mipsel-unknown-linux-musl/release/examples/spotui_daemon
```

## Device paths

The current managed runtime uses these principal paths:

```text
/usr/data/spotui-ui-poc
/usr/data/spotui_daemon
/usr/data/ld-musl-mipsel-sf.so.1
/usr/data/start_spotui.sh
/usr/data/start_spotui.real.sh
/usr/data/return_to_hiby.sh
```

The launcher expects the UI, daemon, and applicable scripts to have their
tested permissions.

Runtime state and credentials also live under `/usr/data`; do not replace or
delete unrelated persistent data during an incremental binary deployment.

## Before replacing runtime files

Exit SpotUI normally so that the stock HiBy interface has returned.

Pause playback and confirm that SpotUI's playback process is no longer active
before pulling, copying, or hashing large device binaries.

Large reads and hashes can compete with real-time audio on the R3 Pro II and
produce underruns that are artifacts of maintenance rather than normal
playback.

Confirm the current process state:

```fish
adb shell '
ps | grep -E "spotui|aplay|hiby_player" | grep -v grep
'
```

The stock `hiby_player` should be running normally when SpotUI is not active.

## Back up the installed runtime files

The `/usr/data` partition is constrained. Keep full archives on the build host
rather than accumulating many rollback binaries on the device.

Create a dated host directory:

```fish
mkdir -p ~/spotui-device-backups/YYYY-MM-DD-description
```

Pull the active matched files:

```fish
adb pull /usr/data/spotui-ui-poc \
    ~/spotui-device-backups/YYYY-MM-DD-description/spotui-ui-poc

adb pull /usr/data/spotui_daemon \
    ~/spotui-device-backups/YYYY-MM-DD-description/spotui_daemon

adb pull /usr/data/ld-musl-mipsel-sf.so.1 \
    ~/spotui-device-backups/YYYY-MM-DD-description/ld-musl-mipsel-sf.so.1

adb pull /usr/data/start_spotui.sh \
    ~/spotui-device-backups/YYYY-MM-DD-description/start_spotui.sh

adb pull /usr/data/start_spotui.real.sh \
    ~/spotui-device-backups/YYYY-MM-DD-description/start_spotui.real.sh

adb pull /usr/data/return_to_hiby.sh \
    ~/spotui-device-backups/YYYY-MM-DD-description/return_to_hiby.sh
```

Record the backup hashes:

```fish
sha256sum ~/spotui-device-backups/YYYY-MM-DD-description/*
```

Keep only the device-side rollback copies required for the current test.

If staging a new binary requires space, remove an obsolete device rollback only
after its host archive and hashes have been verified.

## Deploy the interface

Upload to a temporary path first:

```fish
adb push \
    ~/hiby-standalone-client-public/engine/ui/target/mipsel-unknown-linux-musl/release/spotui-ui-poc \
    /usr/data/spotui-ui-poc.new
```

Compare the local and device hashes:

```fish
sha256sum \
    ~/hiby-standalone-client-public/engine/ui/target/mipsel-unknown-linux-musl/release/spotui-ui-poc

adb shell \
    sha256sum /usr/data/spotui-ui-poc.new
```

After the hashes match, rotate the active binary instead of overwriting it:

```fish
adb shell '
set -e

chmod 755 /usr/data/spotui-ui-poc.new

if [ -f /usr/data/spotui-ui-poc ]; then
    rm -f /usr/data/spotui-ui-poc.previous
    mv \
        /usr/data/spotui-ui-poc \
        /usr/data/spotui-ui-poc.previous
fi

mv \
    /usr/data/spotui-ui-poc.new \
    /usr/data/spotui-ui-poc

sync
ls -lh \
    /usr/data/spotui-ui-poc \
    /usr/data/spotui-ui-poc.previous \
    2>/dev/null
'
```

## Deploy the daemon

Upload to a temporary path:

```fish
adb push \
    ~/mips-toolchain/librespot/target/mipsel-unknown-linux-musl/release/examples/spotui_daemon \
    /usr/data/spotui_daemon.new
```

Compare the hashes:

```fish
sha256sum \
    ~/mips-toolchain/librespot/target/mipsel-unknown-linux-musl/release/examples/spotui_daemon

adb shell \
    sha256sum /usr/data/spotui_daemon.new
```

After the hashes match:

```fish
adb shell '
set -e

chmod 755 /usr/data/spotui_daemon.new

if [ -f /usr/data/spotui_daemon ]; then
    rm -f /usr/data/spotui_daemon.previous
    mv \
        /usr/data/spotui_daemon \
        /usr/data/spotui_daemon.previous
fi

mv \
    /usr/data/spotui_daemon.new \
    /usr/data/spotui_daemon

sync
ls -lh \
    /usr/data/spotui_daemon \
    /usr/data/spotui_daemon.previous \
    2>/dev/null
'
```

For a UI/daemon protocol change, stage and hash-check both `.new` binaries
before rotating either active file. Activate the compatible pair together and
retain a compatible rollback pair.

## Test after incremental deployment

A reboot is useful when the test specifically requires a clean cold-start
state:

```fish
adb reboot
```

After the device finishes booting:

1. confirm that the stock HiBy interface becomes responsive;
2. open the Stream media screen;
3. confirm that the dedicated SpotUI tile and stock Qobuz tile both appear;
4. tap SpotUI;
5. if launch readiness is still pending, confirm the native
   **Preparing SpotUI...** notice appears;
6. confirm SpotUI starts successfully;
7. open Diagnostics and record the displayed version and status;
8. load Liked Songs and at least one playlist;
9. start playback;
10. confirm audio through the expected output;
11. test Previous, Next, pause, resume, seeking, and automatic advancement;
12. test Search;
13. confirm screen sleep/wake and headphone behavior;
14. exit SpotUI normally;
15. confirm the existing stock HiBy interface returns without rebooting;
16. confirm no stock tile is selected unexpectedly;
17. confirm SpotUI does not immediately relaunch;
18. launch SpotUI again and complete a second launch/exit cycle.

Do not treat the Diagnostics label alone as proof of the source checkpoint.
Record the source commit and binary hashes with the test result.

A separate reboot after this sequence is useful when validating cold-boot
persistence or startup behavior, but reboot is no longer the normal SpotUI exit
path.

## Current handoff expectations

The current development handoff is:

1. the dedicated tile sends a request to the prestarted broker;
2. the guarded launcher waits for the safe readiness state;
3. the existing `hiby_player` process is suspended;
4. SpotUI takes control of the framebuffer, audio path, and touchscreen;
5. the UI grabs the touchscreen exclusively;
6. normal Exit shuts down SpotUI's UI, daemon, and playback process;
7. `/usr/data/return_to_hiby.sh` resumes the existing stock player;
8. the broker is re-armed after the short input-drain delay.

Do not manually launch a second `hiby_player` process as part of normal
deployment or recovery.

## Runtime files and logs

The primary current runtime state includes:

```text
/tmp/spotui.sock
/tmp/spotui-ui.log
/tmp/daemon.log
/tmp/start_spotui.wrapper.log
/tmp/return_to_hiby.log
/tmp/spotui-provision.log
```

Inspect the logs with:

```fish
adb shell '
echo "=== Wrapper log ==="
cat /tmp/start_spotui.wrapper.log 2>/dev/null || true

echo
echo "=== UI log ==="
cat /tmp/spotui-ui.log 2>/dev/null || true

echo
echo "=== Daemon log ==="
cat /tmp/daemon.log 2>/dev/null || true

echo
echo "=== Return log ==="
cat /tmp/return_to_hiby.log 2>/dev/null || true

echo
echo "=== Provision log ==="
cat /tmp/spotui-provision.log 2>/dev/null || true
'
```

Check the relevant processes and socket:

```fish
adb shell '
ps | grep -E "spotui|aplay|librespot|hiby_player" | grep -v grep
ls -l /tmp/spotui.sock 2>/dev/null
'
```

Reading a short log tail during playback is normally lightweight.

Perform full device-binary hashes, large pulls, and storage audits while
playback is paused.

## Touchscreen validation

The current UI exclusively grabs the touchscreen while SpotUI is active.

A validated build should log successful exclusive touchscreen ownership.

After Exit, verify that:

- a touch made inside SpotUI is not replayed into the stock interface;
- no stock tile is selected unexpectedly;
- SpotUI does not immediately relaunch;
- a new deliberate tap on the SpotUI tile still works.

If those conditions fail, preserve the UI, wrapper, and return logs before
changing the handoff timing.

## Framebuffer return diagnostics

One intermittent stale-framebuffer return was observed during earlier
development testing but was not reproduced during final
`0.1.0-beta.5-test.4` validation.

No speculative framebuffer-page workaround is part of the current deployment
workflow.

If it reappears, follow the evidence-capture procedure in
[Recovery and restore notes](recovery.md) before modifying framebuffer behavior.

## Development archive

After a complete device regression passes, archive the exact active development
pair on the build host.

Record:

- UI SHA-256;
- daemon SHA-256;
- relevant launcher/return-script hashes;
- source commit or tag;
- firmware/runtime checkpoint;
- device model;
- test date.

A successful local regression does not make that archive a public release
candidate.

Before public distribution, follow
[Tester release bundle workflow](release-bundle.md).

Keep credentials, cache files, proprietary firmware, device backups, and
user-specific logs out of public artifacts.

## Rollback

To restore the immediately previous UI/daemon pair:

```fish
adb shell '
set -e

if [ -f /usr/data/spotui-ui-poc.previous ]; then
    cp -p \
        /usr/data/spotui-ui-poc.previous \
        /usr/data/spotui-ui-poc
    chmod 755 /usr/data/spotui-ui-poc
fi

if [ -f /usr/data/spotui_daemon.previous ]; then
    cp -p \
        /usr/data/spotui_daemon.previous \
        /usr/data/spotui_daemon
    chmod 755 /usr/data/spotui_daemon
fi

sync
reboot
'
```

The reboot above establishes a clean state after restoring the binary pair. It
is not the normal SpotUI Exit behavior.

For launcher, return-script, firmware, managed-runtime, or framebuffer
recovery, see [Recovery and restore notes](recovery.md).

## Repository hygiene

Do not commit:

- compiled MIPS binaries;
- librespot cache contents;
- Spotify credentials or tokens;
- WiFi credentials;
- proprietary firmware files;
- extracted proprietary rootfs trees;
- device backups;
- unsanitized device logs containing private information.

Commit the canonical Rust sources, scripts, documentation, and
non-proprietary project assets only.
