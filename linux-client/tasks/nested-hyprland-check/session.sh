#!/usr/bin/env bash
set -euo pipefail
# The caller already replaced /dev and /run and cleared the environment.
[[ ! -e /dev/input && ! -e /dev/dri/card1 && ! -e /run/user && ! -e /run/dbus ]] || exit 70
[[ "$LIBSEAT_BACKEND" == hopp-disabled ]] || exit 71
printf 'Isolation verified: /dev has only render node; host /run and bus unavailable\n'
export LD_LIBRARY_PATH=/tmp/hopp-weston/root/usr/lib:/tmp/hopp-weston/root/usr/lib/weston
export WESTON_MODULE_MAP='headless-backend.so=/tmp/hopp-weston/root/usr/lib/libweston-15/headless-backend.so;gl-renderer.so=/tmp/hopp-weston/root/usr/lib/libweston-15/gl-renderer.so;kiosk-shell.so=/tmp/hopp-weston/root/usr/lib/weston/kiosk-shell.so'
/tmp/hopp-weston/root/usr/bin/weston --backend=headless --renderer=gl --width=1280 --height=820 --socket=hopp-parent --shell=kiosk --no-config --idle-time=0 --fake-seat --debug --log=/tmp/hopp-nested/weston.log >/tmp/hopp-nested/weston.stdout 2>&1 &
parent_pid=$!
trap 'kill "$parent_pid" 2>/dev/null || true' EXIT
for _ in {1..30}; do [[ -S "$XDG_RUNTIME_DIR/hopp-parent" ]] && break; sleep .1; done
[[ -S "$XDG_RUNTIME_DIR/hopp-parent" ]] || { cat /tmp/hopp-nested/weston.log; exit 72; }
export WAYLAND_DISPLAY=hopp-parent
export LD_LIBRARY_PATH=/tmp/hopp-nested/wlroots-build:/tmp/hopp-nested/labwc-root/usr/lib:$LD_LIBRARY_PATH
WLR_BACKENDS=wayland WLR_RENDER_DRM_DEVICE=/dev/dri/renderD128 /tmp/hopp-nested/labwc-root/usr/bin/labwc -C /tmp/hopp-nested/labwc-config -s /tmp/hopp-nested/middle-start.sh >/tmp/hopp-nested/labwc.stdout 2>&1 &
middle_pid=$!
trap 'kill "$middle_pid" "$parent_pid" 2>/dev/null || true' EXIT
for _ in {1..100}; do [[ -f /tmp/hopp-nested/middle.socket ]] && break; kill -0 "$middle_pid" || { cat /tmp/hopp-nested/labwc.stdout;exit 75; }; sleep .1; done
[[ -f /tmp/hopp-nested/middle.socket ]] || { cat /tmp/hopp-nested/labwc.stdout;exit 76; }
export WAYLAND_DISPLAY="$(cat /tmp/hopp-nested/middle.socket)"
Hyprland --config /tmp/hopp-nested/minimal.lua >/tmp/hopp-nested/hyprland.stdout 2>&1 &
hypr_pid=$!
trap 'kill "$hypr_pid" "$middle_pid" "$parent_pid" 2>/dev/null || true' EXIT
for _ in {1..100}; do
  locks=("$XDG_RUNTIME_DIR"/hypr/*/hyprland.lock)
  [[ -f "${locks[0]}" ]] && break
  kill -0 "$hypr_pid" || { cat /tmp/hopp-nested/hyprland.stdout; exit 73; }
  sleep .1
done
[[ -f "${locks[0]}" ]] || { cat /tmp/hopp-nested/hyprland.stdout; exit 74; }
export HYPRLAND_INSTANCE_SIGNATURE="$(basename "$(dirname "${locks[0]}")")"
export WAYLAND_DISPLAY="$(sed -n '2p' "${locks[0]}")"
hyprctl -j monitors > /tmp/hopp-nested/monitors.json
hyprctl -j locked > /tmp/hopp-nested/locked.json
printf 'Nested ready: %s %s\n' "$WAYLAND_DISPLAY" "$HYPRLAND_INSTANCE_SIGNATURE"
while kill -0 "$hypr_pid"; do
  if [[ -f /tmp/hopp-nested/action.sh ]]; then
    mv /tmp/hopp-nested/action.sh /tmp/hopp-nested/current-action.sh
    bash /tmp/hopp-nested/current-action.sh || true
    python3 -c 'from pathlib import Path;Path("/tmp/hopp-nested/current-action.sh").unlink(missing_ok=True)'
  fi
  sleep .2
done
