#!/bin/sh
# Starts the controller's supervisor in the background (KUAL waits for its actions to exit).
# With the boot job installed (upstart/autostart.sh), it's started through upstart instead,
# so `status aircon-controller` stays accurate.
DIR="$(cd "$(dirname "$0")/.." && pwd)"
PIDFILE=/tmp/aircon-wall-controller.pid

if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
    echo "already running"
    exit 0
fi

if [ -f /etc/upstart/aircon-controller.conf ]; then
    start aircon-controller
    exit 0
fi

nohup /bin/sh "$DIR/bin/supervise.sh" >/dev/null 2>&1 &
