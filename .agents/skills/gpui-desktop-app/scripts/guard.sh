#!/bin/bash
# Runs wtype only when the Pipkin window is the active window; otherwise refuses.
c=$(hyprctl activewindow -j | python3 -c "import json,sys; print(json.load(sys.stdin).get('class',''))")
if [ "$c" != "pipkin" ]; then echo "GUARD: active window is '$c', not sending input" >&2; exit 1; fi
exec wtype "$@"
