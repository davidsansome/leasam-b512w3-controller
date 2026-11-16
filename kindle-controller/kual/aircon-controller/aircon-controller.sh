#!/bin/sh
# Name: Aircon Wall Controller
# Author: David Sansome
# Icon: /mnt/us/extensions/aircon-controller/icon.png

# Library launcher ("scriptlet"): copy this file to /mnt/us/documents/ and it shows up in
# the Library, using icon.png as its cover. Tapping it starts the controller.
#
# Start in the background and return straight away, so the Library's script launcher finishes.
# The delay lets it get back to the home screen first, so it doesn't draw over the controller.
START=/mnt/us/extensions/aircon-controller/bin/start.sh
setsid sh -c "sleep 2; exec sh $START" >/dev/null 2>&1 </dev/null &
