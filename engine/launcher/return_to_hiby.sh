#!/bin/sh

LOG=/tmp/return_to_hiby.log
MARKER=/tmp/spotui.launch
BROKER_LOG=/tmp/spotui-broker.log

: > "$LOG"

echo "[return] suspend/resume return requested" >> "$LOG"
echo "[return] pid=$$ ppid=$PPID" >> "$LOG"

HIBY_PID=$(cat /tmp/spotui-hiby-suspended.pid 2>/dev/null)

if [ -z "$HIBY_PID" ] || [ ! -d "/proc/$HIBY_PID" ]; then
    echo "[return] ERROR: suspended HiBy PID unavailable" >> "$LOG"
    exit 1
fi

echo "[return] saved HiBy pid=$HIBY_PID" >> "$LOG"

# Rust UI normally exits immediately after spawning this script.
i=0
while ps | grep "[s]potui-ui-poc" >/dev/null 2>&1 && [ "$i" -lt 30 ]; do
    sleep 0.1
    i=$((i + 1))
done

echo "[return] UI wait iterations=$i" >> "$LOG"

# Stop launcher/supervisor first so it cannot respawn the daemon.
echo "[return] stopping SpotUI launcher/supervisor" >> "$LOG"

for pid in $(ps | grep -E "[s]tart_spotui\.sh|[s]tart_spotui\.real\.sh" | awk "{print \$1}"); do
    echo "[return] killing launcher pid=$pid" >> "$LOG"
    kill "$pid" 2>/dev/null
done

sleep 0.2

echo "[return] stopping SpotUI audio/runtime" >> "$LOG"

for pid in $(ps | grep -E "[s]potui_daemon|[a]play" | awk "{print \$1}"); do
    echo "[return] killing runtime pid=$pid" >> "$LOG"
    kill "$pid" 2>/dev/null
done

sleep 0.2

rm -f /tmp/spotui.sock
rm -rf /tmp/spotui_start.lock

echo "[return] resuming HiBy pid=$HIBY_PID" >> "$LOG"

kill -CONT "$HIBY_PID"

i=0
while [ "$i" -lt 20 ]; do
    state=$(awk "/^State:/ { print \$2 }" "/proc/$HIBY_PID/status" 2>/dev/null)
    [ "$state" != "T" ] && break
    sleep 0.05
    i=$((i + 1))
done

state=$(awk "/^State:/ { print \$2 }" "/proc/$HIBY_PID/status" 2>/dev/null)
echo "[return] HiBy state=$state" >> "$LOG"

rm -f /tmp/spotui-hiby-suspended.pid

# Let any touch events queued while HiBy was suspended drain before
# the fourth-tile broker becomes actionable again.
echo "[return] waiting for pending input to drain" >> "$LOG"
sleep 2

# Re-arm the fourth-tile launch broker.
echo "[return] re-arming SpotUI launch broker" >> "$LOG"

: > "$MARKER"

nohup setsid sh -c "
    while [ -e \"$MARKER\" ]; do
        sleep 0.25
    done

    while [ ! -x /usr/data/start_spotui.sh ]; do
        sleep 0.25
    done

    exec sh /usr/data/start_spotui.sh
" </dev/null >>"$BROKER_LOG" 2>&1 &

BROKER_PID=$!

echo "$BROKER_PID" > /tmp/spotui-broker.pid
echo "[return] broker pid=$BROKER_PID" >> "$LOG"
echo "[return] complete" >> "$LOG"
