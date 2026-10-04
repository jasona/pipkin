#!/bin/bash
# usage: run.sh [extra args]   launches app with fresh data dir unless DATA set; records the window address (does not move, focus or restyle anything)
S=${PI_TEST_DIR:-/tmp/pi-native-test}; mkdir -p "$S"
D=${DATA:-$S/data}
cd "${PI_REPO:-$PWD}"
(RUST_LOG=warn ./target/release/desktop-app --data-dir $D "$@" > $S/app.log 2>&1 &)
sleep 4
A=$(hyprctl clients -j | python3 -c "import json,sys; print([c['address'] for c in json.load(sys.stdin) if c['class']=='pi-desktop'][0])")


echo $A > $S/addr
