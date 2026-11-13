#!/bin/sh
# Starts the controller's supervisor in the background (KUAL waits for its actions to exit).
DIR="$(cd "$(dirname "$0")/.." && pwd)"
PIDFILE=/tmp/aircon-wall-controller.pid

if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
    echo "already running"
    exit 0
fi

nohup /bin/sh "$DIR/bin/supervise.sh" >/dev/null 2>&1 &
