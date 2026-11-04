#!/usr/bin/env bash
# Isolated native GPUI screenshots. Never connects to the user's Wayland socket.
set -euo pipefail
visual_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
visual_tmp="$(mktemp -d /tmp/hopp-visual.XXXXXX)"
visual_target="${CARGO_TARGET_DIR:-$visual_root/target}"
visual_evidence="$visual_root/../evidence/ui"
visual_compositor_pid=''
visual_fixture_pid=''
cleanup() {
  if [[ -n "$visual_fixture_pid" ]]; then kill "$visual_fixture_pid" 2>/dev/null || true; wait "$visual_fixture_pid" 2>/dev/null || true; fi
  if [[ -n "$visual_compositor_pid" ]]; then kill "$visual_compositor_pid" 2>/dev/null || true; wait "$visual_compositor_pid" 2>/dev/null || true; fi
  rm -rf -- "$visual_tmp"
}
trap cleanup EXIT INT TERM
mkdir -m 700 "$visual_tmp/runtime"
mkdir -p "$visual_tmp/root" "$visual_evidence"
visual_url="$(pacman -Sp --print-format '%n %l' weston | awk '$1 == "weston" { print $2; exit }')"
[[ -n "$visual_url" ]] || { echo 'Cannot resolve the Arch Weston package' >&2; exit 1; }
curl --fail --location --silent --show-error "$visual_url" -o "$visual_tmp/weston.pkg"
tar -xf "$visual_tmp/weston.pkg" -C "$visual_tmp/root"
visual_backend=("$visual_tmp"/root/usr/lib/libweston-*/headless-backend.so)
visual_shell="$visual_tmp/root/usr/lib/weston/kiosk-shell.so"
export XDG_RUNTIME_DIR="$visual_tmp/runtime"
export LD_LIBRARY_PATH="$visual_tmp/root/usr/lib:$visual_tmp/root/usr/lib/weston${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
cargo build --manifest-path "$visual_root/Cargo.toml" --target-dir "$visual_target"
start_compositor() {
  local width="$1" height="$2"
  export WAYLAND_DISPLAY=hopp-fixture
  WESTON_MODULE_MAP="headless-backend.so=${visual_backend[0]};kiosk-shell.so=$visual_shell" \
    "$visual_tmp/root/usr/bin/weston" --backend=headless --renderer=pixman \
    --width="$width" --height="$height" --socket="$WAYLAND_DISPLAY" --shell=kiosk \
    --no-config --idle-time=0 --fake-seat --debug --log="$visual_tmp/weston.log" \
    >"$visual_tmp/weston.stdout" 2>&1 &
  visual_compositor_pid=$!
  for _ in {1..30}; do [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]] && return; sleep 0.1; done
  cat "$visual_tmp/weston.log" >&2
  return 1
}
capture() {
  local name="$1"; shift
  "$visual_target/debug/hopp-visual-fixture" "$@" >"$visual_tmp/fixture.log" 2>&1 &
  visual_fixture_pid=$!
  sleep 2
  kill -0 "$visual_fixture_pid" || { cat "$visual_tmp/fixture.log" >&2; return 1; }
  (cd "$visual_tmp"; timeout 20 "$visual_tmp/root/usr/bin/weston-screenshooter")
  local images=("$visual_tmp"/wayland-screenshot-*.png)
  cp -- "${images[-1]}" "$visual_evidence/$name.png"
  cp -- "$visual_tmp/fixture.log" "$visual_evidence/$name.log" 2>/dev/null || true
  rm -- "${images[@]}"
  kill "$visual_fixture_pid"
  wait "$visual_fixture_pid" 2>/dev/null || true
  visual_fixture_pid=''
}
shot() {
  local name="$1" width="$2" height="$3"; shift 3
  # ONLY=regex limits the run to matching screenshots.
  if [[ -n "${ONLY:-}" && ! "$name" =~ $ONLY ]]; then return; fi
  start_compositor "$width" "$height"
  capture "$name" "$@"
  kill "$visual_compositor_pid"
  wait "$visual_compositor_pid" 2>/dev/null || true
  visual_compositor_pid=''
}
# Hopp window (panel-sized), signed out, signed in, in a call, settings.
shot home-signin   420 680 --scene signin
shot home          420 680 --scene home
shot home-call     420 680 --scene home-call --people 3
shot settings      860 620 --scene settings
shot settings-capture 860 900 --scene settings-capture
# Call window: sizes are exactly what faces::window_size computes.
shot faces-2       212 452 --scene faces --people 2
shot faces-3       212 658 --scene faces --people 3
shot faces-large   332 692 --scene faces --people 2 --size large
shot faces-row     426 124 --scene faces --people 3 --layout row --size small
shot faces-bubbles 76  314 --scene faces --people 3 --layout bubbles
# Shared screen as a tiled viewport (wide and narrow tiles) and as a fitted peek.
shot viewer-wide   1400 820 --scene viewer --people 3
shot viewer-narrow 640  860 --scene viewer --people 3
shot peek          1600 675 --scene peek --people 3
shot peek-tall     620  860 --scene peek --people 3
# A real 16:10 share in a 1582×1312 tile on a 5K ultrawide: docks top and bottom.
shot peek-tile     1582 1312 --scene peek --people 2
# You're sharing: others' marks over your monitor.
shot marks         1728 1080 --scene marks --people 3
# You're sharing: pill under the bar.
shot sharing       600  40  --scene sharing --people 2
# Drawing on your own screen: the same pill as the drawing bar.
shot sharing-draw  600  40  --scene sharing-draw --people 2
# Incoming call card.
shot call          360  92  --scene call
# Drawing on your own shared screen: the pad itself, which shows nothing.
shot drawpad       1280 720 --scene drawpad
# Agent cards: a question on top, two notices, one waiting behind "+1 more".
shot cards         360  454 --scene cards
printf 'Synthetic native UI screenshots saved in %s\n' "$visual_evidence"
