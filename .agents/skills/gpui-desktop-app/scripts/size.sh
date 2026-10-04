#!/bin/bash
# usage: size.sh W H  (resizes the focused app window)
S=${PI_TEST_DIR:-/tmp/pi-native-test}; mkdir -p "$S"
A=$(cat $S/addr)
read CW CH < <(hyprctl clients -j | python3 -c "import json,sys; c=[c for c in json.load(sys.stdin) if c['address']=='$A'][0]; print(c['size'][0],c['size'][1])")
hyprctl dispatch "hl.dsp.window.resize({ x = $(( $1 - CW )), y = $(( $2 - CH )), relative = true })" >/dev/null
sleep 1
hyprctl clients -j | python3 -c "import json,sys; c=[c for c in json.load(sys.stdin) if c['address']=='$A'][0]; print(c['size'])"
