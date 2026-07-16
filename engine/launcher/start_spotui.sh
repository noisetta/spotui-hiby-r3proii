#!/bin/sh
# start_spotui.sh - launch the standalone SpotUI stack on the HiBy R3 Pro II.
#
# Startup ORDER matters. The panel blanks after ~20s of no framebuffer writes,
# and once blanked this (SPI ST7701) panel can't be woken from software. The
# UI's continuous refresh keeps it lit -- but only once the UI is running. So
# we must launch the UI IMMEDIATELY after killing hiby_player, before the long
# WiFi/daemon startup, or the panel blanks during that gap and stays black.
#
# Order:
#   1. kill hiby_player       (frees DAC + framebuffer)
#   2. launch UI in background (panel stays lit via its refresh; UI shows
#                              "Connecting..." and retries LIKED until daemon up)
#   3. DNS + wait for WiFi
#   4. set default output port
#   5. start daemon | aplay   (DEFAULT aplay buffering; big buffer flags starve
#                              the live pipe -> silence)
#   The UI (already running) auto-loads liked songs once the daemon answers.
#
# Run on-device:  sh /usr/data/start_spotui.sh
# Logs: /tmp/daemon.log, /tmp/spotui-ui.log
# busybox ash (no bashisms).

LOADER=/usr/data/ld-musl-mipsel-sf.so.1
DAEMON=/usr/data/spotui_daemon
UI=/usr/data/spotui-ui-poc
SOCK=/tmp/spotui.sock
DAEMON_LOG=/tmp/daemon.log
UI_LOG=/tmp/spotui-ui.log
KEEPALIVE_MS=200

echo "[start] SpotUI launcher"

# --- 0. Wait for hiby_player's codec init (gate on UPTIME, not sleep) --------
# S99spotui's delay counts from init, which is long before hiby_player has
# actually started; killing hiby_player mid-codec-init leaves the DAC in a
# broken state for the entire boot (even raw static is silent afterwards).
# Gating on system uptime guarantees hiby_player has had enough real runtime
# to finish initialising the codec, regardless of when init launched us.
# Panel stays lit throughout thanks to the "Stay on" backlight setting.
UPTIME_GATE=60
echo "[start] waiting for uptime >= ${UPTIME_GATE}s"
while true; do
    up=$(cut -d. -f1 /proc/uptime)
    [ "$up" -ge "$UPTIME_GATE" ] && break
    sleep 2
done
echo "[start] uptime gate passed (${up}s)"

# --- 1. Kill hiby_player -----------------------------------------------------
for pid in $(ps | grep hiby_player | grep -v grep | awk '{print $1}'); do
    kill "$pid" 2>/dev/null
done
echo "[start] hiby_player stopped"

# --- 2. Launch the UI IMMEDIATELY (keeps the panel lit) ----------------------
# It comes up showing "Connecting..." and retries the liked-songs fetch every
# ~2s until the daemon is up. Launching it now (before the WiFi/daemon wait)
# means its refresh takes over the panel within the ~20s blank window.
"$LOADER" "$UI" "$KEEPALIVE_MS" 2>"$UI_LOG" &
UI_PID=$!
echo "[start] UI launched (pid $UI_PID)"

# --- 3. DNS + bring up WiFi + wait -------------------------------------------
printf 'nameserver 1.1.1.1\nnameserver 8.8.8.8\n' > /tmp/resolv.conf

# hiby_player normally associates WiFi; since we've killed it, we must bring
# WiFi up ourselves. Start wpa_supplicant against the saved config (unless it's
# already running), then request a DHCP lease. Guarded so re-runs don't stack
# duplicate daemons.
if ! ps | grep -q "[w]pa_supplicant"; then
    echo "[start] starting wpa_supplicant"
    wpa_supplicant -B -i wlan0 -c /usr/data/wpa_supplicant.conf >/dev/null 2>&1
    sleep 3
fi
echo "[start] requesting DHCP lease"
udhcpc -i wlan0 -n -q >/dev/null 2>&1 &

echo "[start] waiting for WiFi..."
i=0
while [ $i -lt 180 ]; do
    if ifconfig wlan0 2>/dev/null | grep -q "inet addr:"; then
        echo "[start] WiFi is up"
        break
    fi
    # Re-request a lease periodically in case the first attempt was too early.
    if [ $((i % 20)) -eq 19 ]; then
        udhcpc -i wlan0 -n -q >/dev/null 2>&1 &
    fi
    i=$((i + 1))
    sleep 0.5
done
[ $i -ge 180 ] && echo "[start] WARNING: WiFi not up within timeout; continuing"

# --- 4. Output routing: pick the port matching the plugged jack -------------
# Belt-and-suspenders: set the correct port up front based on the jack switches
# (2 = 3.5mm headset, 3 = 4.4mm balance). The UI also does this and keeps it
# refreshed every ~1s, but setting it here too avoids any silent-at-boot window.
if [ "$(cat /sys/class/switch/balance/state 2>/dev/null)" = "1" ]; then
    amixer -c 0 cset numid=9 3 >/dev/null 2>&1
    echo "[start] 4.4mm balanced detected -> port 3"
else
    amixer -c 0 cset numid=9 2 >/dev/null 2>&1
    echo "[start] defaulting to 3.5mm -> port 2"
fi

# --- 5. Start the daemon piped to aplay (DEFAULT buffering) ------------------
# Supervise it: if the daemon exits (e.g. Spotify unreachable because WiFi
# wasn't fully ready yet at cold boot), wait and relaunch. This makes the stack
# self-healing -- the UI stays up showing "Connecting..." and picks up the
# liked-songs list on its next retry once a daemon attempt finally succeeds.
rm -f "$SOCK"
SPOTUI_TMP=/tmp/spotui
rm -rf "$SPOTUI_TMP"
mkdir -p "$SPOTUI_TMP"
(
    while true; do
        echo "[start] (re)starting daemon+aplay"
        TMPDIR="$SPOTUI_TMP" "$LOADER" "$DAEMON" 2>>"$DAEMON_LOG" \
            | aplay -D hw:0,0 -f S16_LE -r 44100 -c 2
        echo "[start] daemon exited; retrying in 5s"
        sleep 5
    done
) &
DAEMON_SUP_PID=$!
echo "[start] daemon supervisor launched (pid $DAEMON_SUP_PID)"

# The UI is already up and will pick up the liked-songs list on its next retry
# once the daemon reports ready. Wait on the UI so this script (and the process
# tree) stays alive as long as the UI runs.
wait "$UI_PID"
echo "[start] UI exited"
