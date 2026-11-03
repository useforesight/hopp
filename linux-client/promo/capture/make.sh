#!/usr/bin/env bash
# Builds hopp-capture-promo.mp4: soundtrack → frames → mux. Shares ../render.mjs.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p build
[ -f build/screen.png ] || cp ../build/screen.png build/
[ -f build/wp-tokyo.jpg ] || cp ../build/wp-tokyo.jpg build/
node music.mjs
node ../render.mjs --film film.html --build build --fps "${FPS:-30}" --workers "${WORKERS:-6}"
ffmpeg -loglevel error -y -i build/video.mp4 -i build/music.wav -map 0:v -map 1:a -c:v copy \
  -af "loudnorm=I=-14:TP=-1.0:LRA=11" -ar 48000 -c:a aac -b:a 256k -shortest -movflags +faststart hopp-capture-promo.mp4
echo "hopp-capture-promo.mp4"
