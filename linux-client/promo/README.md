# Promo video

A 47-second, 128 BPM promo for the Omarchy client, built entirely from code: no screen
recording, stock footage or music samples.

```sh
./make.sh          # → hopp-omarchy-promo.mp4 (about 1.5 min on 12 cores)
FPS=60 ./make.sh   # smoother, twice the render time
```

| File | What it is |
| --- | --- |
| `film.html` | The whole film. `render(t)` sets every element for time `t`; open `film.html?t=27.5` in a browser to preview any moment. Cuts, captions, camera moves and key presses all sit on the beat map `T(bar, beat)`. |
| `screen.html` | Mira's shared screen (3:2 laptop, Catppuccin, real `geometry.rs` code), rendered once at 3× to `build/screen.png`. |
| `render.mjs` | Drives headless Chromium over the DevTools protocol (no npm packages) and pipes frames to ffmpeg, with parallel workers. `--shots 8.2,27.5` writes stills to `build/shots/`. |
| `music.mjs` | Synthesises the soundtrack (kick, clap, hats, bass, supersaw pads, arp, risers, impacts, whooshes, key clicks) from the same beat map. |
| `make.sh` | Wallpapers → screen → music → frames → mux with loudness normalisation (−14 LUFS). |

Wallpapers come from the local Omarchy themes in `/usr/share/omarchy/themes`; the people
in the call window are SVG illustrations. `build/` is a scratch directory.
