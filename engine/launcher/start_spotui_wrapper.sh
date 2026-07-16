#!/bin/sh
# Non-blocking entry point invoked by the repurposed Qobuz tile.
#
# The foreground request returns to hiby_player immediately. A detached,
# locked worker runs the real launcher and preserves its uptime safety gate.

SELF=/usr/data/start_spotui.sh
REAL=/usr/data/start_spotui.real.sh
LOCK_DIR=/tmp/spotui_start.lock
LOG=/tmp/start_spotui.wrapper.log

cleanup_lock() {
    rmdir "$LOCK_DIR" 2>/dev/null
}

close_inherited_device_fds() {
    for descriptor in /proc/$$/fd/*; do
        [ -e "$descriptor" ] || continue

        fd=${descriptor##*/}

        case "$fd" in
            0|1|2)
                continue
                ;;
        esac

        target=$(readlink "$descriptor" 2>/dev/null)

        case "$target" in
            /dev/fb*|/dev/input/*|/dev/snd/*|/dev/graphics/*)
                echo "[wrapper] closing inherited fd $fd -> $target" >> "$LOG"
                eval "exec ${fd}>&-"
                ;;
        esac
    done
}

if [ "$1" = "--worker" ]; then
    trap cleanup_lock EXIT INT TERM

    close_inherited_device_fds

    echo "[wrapper] worker started pid=$$ uptime=$(cut -d. -f1 /proc/uptime)s" >> "$LOG"

    mkdir -p /usr/data/tmp
    rm -f /usr/data/tmp/.tmp*
    echo "[wrapper] cleaned stale tmp files" >> "$LOG"

    sh "$REAL" >> "$LOG" 2>&1
    rc=$?

    echo "[wrapper] real launcher exited with $rc" >> "$LOG"
    exit "$rc"
fi

if ! mkdir "$LOCK_DIR" 2>/dev/null; then
    echo "[wrapper] SpotUI already launching or running; request ignored" >> "$LOG"
    exit 0
fi

: > "$LOG"
echo "[wrapper] launch request accepted uptime=$(cut -d. -f1 /proc/uptime)s" >> "$LOG"

nohup setsid sh "$SELF" --worker </dev/null >>"$LOG" 2>&1 &
worker_pid=$!

echo "[wrapper] detached worker pid=$worker_pid" >> "$LOG"

exit 0
