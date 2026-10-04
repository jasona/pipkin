#!/bin/bash
S=${PI_TEST_DIR:-/tmp/pi-native-test}; mkdir -p "$S"
A=$(cat $S/addr)
G=$(hyprctl clients -j | python3 -c "import json,sys; c=[c for c in json.load(sys.stdin) if c['address']=='$A'][0]; print('%d,%d %dx%d'%(c['at'][0],c['at'][1],c['size'][0],c['size'][1]))")
grim -g "$G" $S/$1.png
