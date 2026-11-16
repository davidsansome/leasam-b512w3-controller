#!/bin/sh
# Starts the aircon wall controller at boot instead of the Kindle UI, or puts things back.
#
#   sh autostart.sh install    install the boot job, and stop X starting at boot
#   sh autostart.sh uninstall  remove the boot job, and put x_prelaunch.conf back
#   sh autostart.sh disable    keep everything installed, but boot into the Kindle UI
#   sh autostart.sh enable     undo disable
#
# X is kept off by a check in /etc/upstart/x_prelaunch.conf: while aircon-controller.conf
# is in /etc/upstart, it doesn't emit x_should_start. Nothing after X (the Kindle UI)
# starts either. The controller starts X itself when you choose Exit. A copy of the
# original x_prelaunch.conf is kept next to this script.
#
# Changes take effect at the next boot.
set -e

HERE="$(cd "$(dirname "$0")" && pwd)"
UPSTART=/etc/upstart
JOB="$UPSTART/aircon-controller.conf"
PRELAUNCH="$UPSTART/x_prelaunch.conf"
BACKUP="$HERE/x_prelaunch.conf.orig"
# The stock line the check goes in front of.
STOCK_IF='    if \[ "\$SHOULD_SKIP_X_PRELAUNCH" == "true" \]; then'

patched() {
    grep -q "aircon-controller" "$PRELAUNCH"
}

rw() {
    mntroot rw >/dev/null
    trap 'mntroot ro >/dev/null' EXIT
}

patch_prelaunch() {
    if patched; then
        return
    fi
    cp "$PRELAUNCH" "$BACKUP"
    sed -i "s|^$STOCK_IF\$|    if [ -f $JOB ]; then\n        f_log I \"x_prelaunch\" \"aircon-controller installed: not starting X\"\n    elif [ \"\$SHOULD_SKIP_X_PRELAUNCH\" == \"true\" ]; then|" "$PRELAUNCH"
    if ! patched; then
        echo "x_prelaunch.conf isn't the version this script knows; not installing" >&2
        exit 1
    fi
}

unpatch_prelaunch() {
    if ! patched; then
        return
    fi
    if [ -f "$BACKUP" ] && ! grep -q "aircon-controller" "$BACKUP"; then
        cp "$BACKUP" "$PRELAUNCH"
    else
        # No backup: take the two added lines out and turn the elif back into an if.
        sed -i -e '/aircon-controller/d' -e 's|^    elif \[ "\$SHOULD_SKIP_X_PRELAUNCH"|    if [ "$SHOULD_SKIP_X_PRELAUNCH"|' "$PRELAUNCH"
    fi
}

case "$1" in
    install)
        rw
        patch_prelaunch
        rm -f "$JOB.disabled"
        cp "$HERE/aircon-controller.conf" "$JOB"
        echo "Installed. The controller starts instead of the Kindle UI from the next boot."
        ;;
    uninstall)
        rw
        rm -f "$JOB" "$JOB.disabled"
        unpatch_prelaunch
        echo "Removed. The Kindle boots into its own UI from the next boot."
        ;;
    disable)
        [ -f "$JOB" ] || { echo "not enabled" >&2; exit 1; }
        rw
        mv "$JOB" "$JOB.disabled"
        echo "Disabled. The Kindle boots into its own UI from the next boot."
        ;;
    enable)
        [ -f "$JOB.disabled" ] || { echo "not disabled (run install first?)" >&2; exit 1; }
        patched || { echo "x_prelaunch.conf isn't patched: run install instead" >&2; exit 1; }
        rw
        mv "$JOB.disabled" "$JOB"
        echo "Enabled. The controller starts instead of the Kindle UI from the next boot."
        ;;
    *)
        echo "usage: sh $0 install|uninstall|enable|disable" >&2
        exit 1
        ;;
esac
