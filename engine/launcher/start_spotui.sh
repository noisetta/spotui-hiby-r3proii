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

# --- 0. Wait for HiBy hardware readiness ------------------------------------
# Stopping hiby_player during codec initialization can leave the DAC unusable
# until reboot. Require a minimum uptime plus stable HiBy, framebuffer, ALSA,
# mixer-control, and mixer-state signals before taking over.
#
# The 90-second limit remains only as a fallback if readiness cannot be
# established. The detached entry-point keeps HiBy responsive while waiting.
MIN_UPTIME=60
FALLBACK_UPTIME=90
READY_STREAK_REQUIRED=3

ready_streak=0
last_mixer_hash=

echo "[start] waiting for HiBy hardware readiness"
echo "[start] minimum uptime ${MIN_UPTIME}s; fallback ${FALLBACK_UPTIME}s"

while true; do
    up=$(cut -d. -f1 /proc/uptime)

    if [ "$up" -ge "$FALLBACK_UPTIME" ]; then
        echo "[start] readiness fallback reached (${up}s)"
        break
    fi

    ready=1

    hiby_pid=$(pidof hiby_player 2>/dev/null | awk "{ print \$1 }")
    threads=0
    framebuffer_owned=0
    controls=0
    mixer_hash=

    [ "$up" -ge "$MIN_UPTIME" ] || ready=0
    [ -n "$hiby_pid" ] || ready=0

    if [ -n "$hiby_pid" ] && [ -d "/proc/$hiby_pid" ]; then
        threads=$(awk "/^Threads:/ { print \$2 }" "/proc/$hiby_pid/status" 2>/dev/null)

        for descriptor in "/proc/$hiby_pid"/fd/*; do
            [ "$(readlink "$descriptor" 2>/dev/null)" = "/dev/fb0" ] || continue
            framebuffer_owned=1
            break
        done
    fi

    [ "${threads:-0}" -ge 20 ] 2>/dev/null || ready=0
    [ "$framebuffer_owned" -eq 1 ] || ready=0

    [ -e /dev/snd/controlC0 ] || ready=0
    [ -e /dev/snd/pcmC0D0p ] || ready=0

    controls=$(amixer -c 0 controls 2>/dev/null | wc -l | tr -d " ")
    [ "${controls:-0}" -ge 10 ] 2>/dev/null || ready=0

    amixer -c 0 cget numid=9 >/dev/null 2>&1 || ready=0

    mixer_hash=$(amixer -c 0 contents 2>/dev/null |
        sha256sum |
        awk "{ print \$1 }")

    [ -n "$mixer_hash" ] || ready=0

    if [ "$ready" -eq 1 ]; then
        if [ "$mixer_hash" = "$last_mixer_hash" ]; then
            ready_streak=$((ready_streak + 1))
        else
            ready_streak=1
        fi

        last_mixer_hash=$mixer_hash

        echo "[start] readiness ${ready_streak}/${READY_STREAK_REQUIRED} at ${up}s pid=${hiby_pid} threads=${threads} controls=${controls}"
    else
        if [ "$ready_streak" -ne 0 ]; then
            echo "[start] readiness reset at ${up}s"
        fi

        ready_streak=0
        last_mixer_hash=
    fi

    [ "$ready_streak" -ge "$READY_STREAK_REQUIRED" ] && break

    sleep 1
done

echo "[start] readiness gate passed (${up}s)"

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
# already running), then request a DHCP lease. A cold boot can leave udhcpc
# associated but sending unanswered discovery packets indefinitely, so keep a
# single client and recycle both association and DHCP at bounded intervals.
if ! ps | grep -q "[w]pa_supplicant"; then
    echo "[start] starting wpa_supplicant"
    wpa_supplicant -B -i wlan0 -c /usr/data/wpa_supplicant.conf >/dev/null 2>&1
    sleep 3
fi

DHCP_LOG=/tmp/spotui-dhcp.log

stop_dhcp_clients() {
    for pid in $(ps | grep "[u]dhcpc.*wlan0" | awk '{print $1}'); do
        kill "$pid" 2>/dev/null
    done
}

start_dhcp_client() {
    echo "[start] requesting DHCP lease"
    udhcpc -i wlan0 -b -n -t 5 -T 2 \
        -x hostname:HiBy-R3PROII >>"$DHCP_LOG" 2>&1 &
    DHCP_PID=$!
    echo "[start] DHCP client launched (pid $DHCP_PID)"
}

stop_dhcp_clients
: >"$DHCP_LOG"
start_dhcp_client

echo "[start] waiting for WiFi..."
i=0
while [ $i -lt 180 ]; do
    if ifconfig wlan0 2>/dev/null | grep -q "inet addr:"; then
        echo "[start] WiFi is up"
        break
    fi

    # At 30s and 60s, replace a stalled client rather than stacking another
    # udhcpc process. Reassociation cleared the observed cold-boot failure
    # where WPA was complete but the access point returned no DHCP offer.
    if [ "$i" -eq 59 ] || [ "$i" -eq 119 ]; then
        echo "[start] DHCP stalled; renewing WiFi association"
        stop_dhcp_clients
        wpa_cli -i wlan0 reassociate >/dev/null 2>&1
        sleep 2
        start_dhcp_client
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
