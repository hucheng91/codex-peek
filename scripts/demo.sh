#!/usr/bin/env bash
# Records docs/screenshot.png and docs/demo.gif on a Linux X display (Xvfb works).
# Needs: ImageMagick (convert/import), xdotool, a release build.
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=./target/release/codex-peek
T=$(mktemp -d); F=$(mktemp -d)
snap() { import -window root -crop "${2:-450x160}+0+0" "$F/$1.png"; }
img() { convert -size 400x300 xc:"$1" -gravity center -pointsize 44 -fill white -annotate 0 "$2" "$T/codex-clipboard-$3.png"; }
echo '{"session_id":"demo"}' | $BIN --hook-start --dir "$T"   # what the Codex SessionStart hook does
sleep 2; snap 00
img "#e4572e" "Login bug" a1; sleep 1; snap 01
img "#17bebb" "Chart" b2;     sleep 1; snap 02
img "#ffc914" "Mockup" c3;    sleep 1; snap 03
img "#76b041" "Diagram" d4;   sleep 1; snap 04
cp "$F/04.png" docs/screenshot.png
xdotool mousemove 200 75 click 1; sleep 1.2; snap 05 450x400
xdotool mousemove 200 200 click 1; sleep 1; snap 06
xdotool mousemove 362 14 click 1; sleep 1; snap 07
img "#8e44ad" "Next prompt" e5; sleep 1; snap 08
echo '{"session_id":"demo"}' | $BIN --hook-stop               # SessionEnd hook closes it
for f in "$F"/*.png; do convert "$f" -background "#3b4a5c" -gravity northwest -extent 450x400 "$f"; done
convert -delay 110 -loop 0 "$F"/0[0-8].png -layers Optimize docs/demo.gif
rm -rf "$T" "$F"
echo "wrote docs/screenshot.png docs/demo.gif"
