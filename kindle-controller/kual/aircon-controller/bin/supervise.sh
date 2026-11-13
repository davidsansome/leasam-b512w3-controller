#!/bin/sh
# Runs the controller, restarting it if it crashes. A clean exit (the long-press exit
# gesture, or SIGTERM from stop.sh) ends the loop.
#
# Whatever happens, the Kindle UI is restored after each run (--restore-framework), so a
# crash or kill -9 can't leave the Kindle frozen behind a stale screen.
DIR="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$DIR/bin/aircon-wall-controller"
CONFIG="$DIR/config.toml"
LOG="$DIR/controller.log"
PIDFILE=/tmp/aircon-wall-controller.pid

echo $$ > "$PIDFILE"
CHILD=""

restore() {
    "$BIN" --config "$CONFIG" --restore-framework >>"$LOG" 2>&1
}

on_term() {
    [ -n "$CHILD" ] && kill -TERM "$CHILD" 2>/dev/null && wait "$CHILD"
    restore
    rm -f "$PIDFILE"
    exit 0
}
trap on_term TERM INT HUP

while :; do
    # Keep the log from filling the user partition.
    if [ -f "$LOG" ] && [ "$(wc -c < "$LOG")" -gt 1048576 ]; then
        mv -f "$LOG" "$LOG.old"
    fi
    "$BIN" --config "$CONFIG" >>"$LOG" 2>&1 &
    CHILD=$!
    wait "$CHILD"
    STATUS=$?
    CHILD=""
    restore
    [ "$STATUS" -eq 0 ] && break
    echo "$(date) controller exited with status $STATUS; restarting in 10s" >>"$LOG"
    sleep 10
done

rm -f "$PIDFILE"
