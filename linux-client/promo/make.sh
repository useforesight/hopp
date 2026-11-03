#!/usr/bin/env bash
# Builds hopp-omarchy-promo.mp4: assets → soundtrack → frames → mux.
set -euo pipefail
cd "$(dirname "$0")"
T=/usr/share/omarchy/themes
mkdir -p build
[ -f build/wp-tokyo.jpg ] || ffmpeg -loglevel error -y -i $T/tokyo-night/backgrounds/0-winding-road.jpg -vf scale=2880:-2 -q:v 3 build/wp-tokyo.jpg
for pair in catppuccin:catppuccin/backgrounds/2-waves.png kanagawa:kanagawa/backgrounds/1-kanagawa.jpg \
            gruvbox:gruvbox/backgrounds/1-the-backwater.jpg nord:nord/backgrounds/0-black-moon.jpg; do
  name=${pair%%:*}; src=$T/${pair#*:}
  [ -f build/wp-$name.jpg ] || ffmpeg -loglevel error -y -i "$src" \
    -vf "scale=2880:1620:force_original_aspect_ratio=increase,crop=2880:1620" -q:v 3 build/wp-$name.jpg
done
[ -f build/screen.png ] || chromium --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=3 \
  --window-size=1500,1000 --screenshot=build/screen.png "file://$PWD/screen.html" 2>/dev/null
node music.mjs
node render.mjs --fps "${FPS:-30}" --workers "${WORKERS:-6}"
ffmpeg -loglevel error -y -i build/video.mp4 -i build/music.wav -map 0:v -map 1:a -c:v copy \
  -af "loudnorm=I=-14:TP=-1.0:LRA=11" -ar 48000 -c:a aac -b:a 256k -shortest -movflags +faststart hopp-omarchy-promo.mp4
echo "hopp-omarchy-promo.mp4"
