# Native test scripts (Hyprland 0.56 / Wayland)
All are optional helpers; read references/native-testing.md first and obey its safety rules.
- guard.sh: runs `wtype ARGS` only if the active window class is `pi-desktop` (edit the class for your app_id).
- run.sh [app args]: launches target/release/desktop-app with a data dir, records its address in $PI_TEST_DIR/addr (does not move or focus anything).
- shot.sh NAME: crops a `grim` capture to the recorded window into $PI_TEST_DIR/NAME.png.
- size.sh W H: relative resize of the ACTIVE window to W×H (Lua dispatch). Only use when your app is the active window.
- mouse.py CMD ...: virtual absolute pointer via /dev/uinput (edit W,H to your output size); keep coordinates inside your own window's exclusive area.
