#!/bin/sh
# Runs the controller, restarting it if it crashes. A clean exit (Exit from the menu, or
# SIGTERM from stop.sh) ends the loop.
#
# On a clean exit the controller hands the screen back to the Kindle UI itself. After a
# crash it doesn't, and neither does this script: the controller is started again 10s
# later, and handing back in between would start X and the whole Kindle UI. If this script
# is stopped while no controller is running, it hands back with --restore-framework.
#
# Started by the aircon-controller upstart job at boot, or by start.sh.
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
    if [ -n "$CHILD" ]; then
        # The controller hands back itself when it exits cleanly.
        kill -TERM "$CHILD" 2>/dev/null
        wait "$CHILD" || restore
    else
        restore
    fi
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
    [ "$STATUS" -eq 0 ] && break
    echo "$(date) controller exited with status $STATUS; restarting in 10s" >>"$LOG"
    sleep 10
done

rm -f "$PIDFILE"
