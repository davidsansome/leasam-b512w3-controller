#!/bin/sh
# Stops the controller; the supervisor then gives the screen back to the Kindle UI.
# Waits (up to 20s) until it has actually exited, so the binary can be replaced afterwards.
PIDFILE=/tmp/aircon-wall-controller.pid

if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
    kill -TERM "$(cat "$PIDFILE")"
else
    # No supervisor: stop a stray controller directly (it restores the UI itself).
    killall -TERM aircon-wall-controller 2>/dev/null
fi

i=0
while [ $i -lt 20 ] && { [ -f "$PIDFILE" ] || pidof aircon-wall-controller >/dev/null; }; do
    sleep 1
    i=$((i + 1))
done
