# Build and deploy SpotUI

This document records the tested local cross-build workflow for the SpotUI interface and its librespot-based daemon on the HiBy R3 Pro II.

The commands below assume:

- the public SpotUI repository is cloned at `$HOME/hiby-standalone-client-public`;
- a working Rust nightly toolchain is installed;
- a working `mipsel-unknown-linux-musl` cross-build environment is already configured;
- the local librespot source tree is available at `$HOME/mips-toolchain/librespot`;
- ADB can reach the HiBy R3 Pro II.

The repository does not include proprietary firmware, device credentials, Spotify credentials, or a complete MIPS toolchain.

> [!WARNING]
> These instructions are specific to the HiBy R3 Pro II development setup used for SpotUI. Do not deploy the binaries to another model unless that device has been independently tested.

## Source locations

The canonical SpotUI interface source is:

```text
engine/ui/src/main.rs
```

The canonical daemon source is:

```text
apps/spotify/daemon/spotui_daemon.rs
```

The daemon is compiled inside a local librespot source tree because it uses librespot as an example binary.

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

The custom MIPS environment must also provide the linker, archiver, C runtime, and target configuration required by `mipsel-unknown-linux-musl`. Those details are currently external to this repository.

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

The release profile is configured for a small binary with link-time optimization, symbol stripping, and abort-on-panic behavior.

## Prepare the daemon source

The daemon is built against the local librespot tree. The current source targets the librespot 0.8.0 API.

Copy the canonical daemon source into the librespot examples directory:

```fish
cp \
    ~/hiby-standalone-client-public/apps/spotify/daemon/spotui_daemon.rs \
    ~/mips-toolchain/librespot/examples/spotui_daemon.rs
```

Confirm that the two copies match before building:

```fish
sha256sum \
    ~/hiby-standalone-client-public/apps/spotify/daemon/spotui_daemon.rs \
    ~/mips-toolchain/librespot/examples/spotui_daemon.rs
```

Both hashes must be identical.

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

The tested deployment uses:

```text
/usr/data/spotui-ui-poc
/usr/data/spotui_daemon
/usr/data/ld-musl-mipsel-sf.so.1
/usr/data/start_spotui.real.sh
/usr/data/start_spotui.sh
```

The launcher expects the UI and daemon binaries to be executable.

## Back up the installed binaries

Before replacing either binary:

```fish
adb shell '
set -e

if [ -f /usr/data/spotui-ui-poc ]; then
    cp -p \
        /usr/data/spotui-ui-poc \
        /usr/data/spotui-ui-poc.previous
fi

if [ -f /usr/data/spotui_daemon ]; then
    cp -p \
        /usr/data/spotui_daemon \
        /usr/data/spotui_daemon.previous
fi

sync
'
```

This keeps one immediate rollback copy on the device. Preserve additional known-good copies outside the device as well.

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

After the hashes match:

```fish
adb shell '
set -e

chmod 755 /usr/data/spotui-ui-poc.new
mv \
    /usr/data/spotui-ui-poc.new \
    /usr/data/spotui-ui-poc

sync
ls -lh /usr/data/spotui-ui-poc
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
mv \
    /usr/data/spotui_daemon.new \
    /usr/data/spotui_daemon

sync
ls -lh /usr/data/spotui_daemon
'
```

## Reboot and test

Do not try to restart the stock `hiby_player` process manually after SpotUI has taken over the framebuffer. The tested workflow is to reboot the player after deployment:

```fish
adb reboot
```

After the device finishes booting:

1. Open the Stream media screen.
2. Confirm that the SpotUI launcher tile renders correctly.
3. Launch SpotUI.
4. Confirm that the interface starts.
5. Load the track list.
6. Start playback.
7. Confirm that audio is produced through the expected output.
8. Exit and relaunch SpotUI.
9. Reboot once more and repeat the startup test.

## Runtime files and logs

The tested runtime paths include:

```text
/tmp/spotui.sock
/tmp/spotui-ui.log
/tmp/daemon.log
/tmp/start_spotui.wrapper.log
```

Inspect them with:

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
'
```

Check the relevant processes:

```fish
adb shell '
ps | grep -E "spotui|aplay|librespot" | grep -v grep
'
```

## Rollback

To restore the immediately previous binaries:

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

For firmware-level recovery, see [Recovery notes](recovery.md).

## Repository hygiene

Do not commit:

- compiled MIPS binaries;
- librespot cache contents;
- Spotify credentials or tokens;
- WiFi credentials;
- proprietary firmware files;
- extracted proprietary rootfs trees;
- device logs containing private information.

Commit the canonical Rust sources, scripts, documentation, and non-proprietary project assets only.
