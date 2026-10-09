# Spotify credential onboarding

SpotUI authenticates playback through librespot 0.8.0. The desktop onboarding
helper opens Spotify's OAuth authorization page, exchanges the authorization
for librespot's reusable credential format, and installs only that private
credential on the connected player. It never requests a Spotify password and
does not add credentials to firmware, an SD-card runtime archive, or this
repository.

## Requirements

- a Spotify Premium account;
- a SpotUI-prepared HiBy R3 Pro II at the stock HiBy interface;
- ADB enabled from the device's About screen;
- exactly one ready device attached over USB, or its ADB serial;
- Python 3 and ADB on a Linux computer;
- the matched `spotui-auth-helper` release binary;
- a browser on the same computer, with localhost port 5588 available.

The first tested helper build targets x86-64 Linux. A release bundle should put
the executable `spotui-auth-helper` beside `onboard_spotify.py` so the default
invocation can find it. Generated helper binaries are release artifacts and
must not be committed to the source repository.

## Build the desktop helper from source

The helper is a small host application, not a device binary. Its lockfile pins
the exact dependency graph around librespot 0.8.0. From the repository root:

```fish
cargo build --release --locked \
    --manifest-path tools/auth-helper/Cargo.toml
```

The result is:

```text
tools/auth-helper/target/release/spotui-auth-helper
```

Verify the binary before packaging it:

```fish
tools/auth-helper/target/release/spotui-auth-helper --version
file tools/auth-helper/target/release/spotui-auth-helper
sha256sum tools/auth-helper/target/release/spotui-auth-helper
```

Record the host platform and SHA-256 in the corresponding prerelease. Do not
represent a locally built binary as portable to systems on which it has not
been tested.

## First-time authorization

Keep SpotUI closed at the stock HiBy interface, then run:

```fish
tools/installer/onboard_spotify.py \
    --auth-helper tools/auth-helper/target/release/spotui-auth-helper
```

If multiple ADB devices are present, add `--serial DEVICE_SERIAL`. If port 5588
is occupied, add `--oauth-port PORT` using another available unprivileged
localhost port.

The helper performs this sequence:

1. confirms exactly one authorized ADB device and an installed SpotUI runtime;
2. refuses to continue while SpotUI, its launcher, daemon, or `aplay` is active;
3. creates a mode-0700 temporary directory on the computer;
4. opens Spotify's authorization page and also prints the same URL;
5. asks librespot to exchange the short-lived OAuth result for its reusable
   `credentials.json` format;
6. validates the credential structure without printing its account name or
   authentication data;
7. stages it as `/usr/data/librespot-cache/credentials.json.new`;
8. compares host and device hashes without displaying them;
9. preserves the current device credential as `credentials.json.previous`;
10. atomically activates the new mode-0600 credential in the mode-0700 cache;
11. removes the computer's temporary credential directory.

The authorization URL contains a short-lived state value and PKCE challenge,
not the resulting credential. Do not share the URL while authorization is in
progress. The access token and reusable authentication data are never printed.

After completion, launch SpotUI and test Liked Songs, a searched track, audio,
pause/resume, Next, and normal Exit back to the stock HiBy interface.
Authentication is persistent, so ADB and the computer are not required for
ordinary playback afterward.

Reboot once separately to confirm that the credential remains valid after a
cold start. Reboot is not the normal SpotUI exit path.

## Existing credential and rollback modes

Maintainers may install an already prepared reusable cache without rerunning
OAuth:

```fish
tools/installer/onboard_spotify.py \
    --credentials /private/path/credentials.json
```

The same validation, staging, hash, permission, backup, and activation rules
apply. The source file must already be private with mode 0600. A short-lived
access-token JSON, group/world-readable file, symlink, or malformed cache is
rejected.

If a newly activated credential fails, return to the stock HiBy interface and
restore the preserved device copy:

```fish
tools/installer/onboard_spotify.py --rollback
```

Rollback keeps the replaced credential as `credentials.json.failed` for
controlled diagnosis. All three files remain private to root. Do not copy any
of them into the repository, release assets, issue attachments, or public
logs.

## Authentication boundary

This authorization is for SpotUI's librespot playback session. It does not
enable adding or removing Liked Songs. Library writes remain a separate future
feature requiring an intentionally designed Spotify Web API authorization
flow.

The onboarding helper does not configure WiFi, enable ADB permanently, flash
firmware, launch SpotUI automatically, or upload account details anywhere
other than Spotify's own authorization/session endpoints and the user's
attached player.

## Maintainer validation record

The complete flow passed on one HiBy R3 Pro II on 2026-07-27 using the HMOD
v1.5 two-file tester installer. Browser authorization created a distinct
credential, the prior working cache was preserved, all credential paths were
mode 0600 inside a mode-0700 directory, and no host or device staging residue
remained. The new credential survived a reboot and passed Liked Songs, search,
playback, pause/resume, Next, normal tile launch, and the then-current
Exit/reboot behavior.

That validation record predates the later suspend/resume return-to-HiBy
handoff. It is retained as a historical authentication test record rather than
a description of the current exit path.
