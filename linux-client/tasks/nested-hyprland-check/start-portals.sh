#!/usr/bin/env bash
set -euo pipefail
[[ "$XDG_RUNTIME_DIR" == /tmp/hopp-nested/runtime && ! -e /run/user && ! -e /dev/input ]] || exit 70
export XDG_CURRENT_DESKTOP=Hyprland
# Updates only the sandbox's dbus-run-session bus, never systemd.
dbus-update-activation-environment WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE XDG_CURRENT_DESKTOP XDG_RUNTIME_DIR XDG_CONFIG_HOME XDG_CACHE_HOME XDG_DATA_HOME LIBSEAT_BACKEND HYPRLAND_NO_SD_VARS HYPRLAND_NO_SD_NOTIFY
python3 - <<'PY'
from pathlib import Path
import os,signal
for entry in Path('/proc').iterdir():
 if entry.name.isdigit():
  try:
   args=(entry/'cmdline').read_bytes().split(b'\0')
   if args[0] == b'/usr/lib/xdg-desktop-portal':os.kill(int(entry.name),signal.SIGTERM)
  except (FileNotFoundError,ProcessLookupError,PermissionError):pass
PY
pipewire >/tmp/hopp-nested/pipewire.log 2>&1 &
/usr/lib/xdg-desktop-portal-hyprland >/tmp/hopp-nested/xdph.log 2>&1 &
/usr/lib/xdg-desktop-portal >/tmp/hopp-nested/portal.log 2>&1 &
sleep 1
busctl --user list > /tmp/hopp-nested/bus-services.txt
pw-cli info 0 > /tmp/hopp-nested/pipewire-info.txt
printf '%s\n' 'Private portal/PipeWire ready' > /tmp/hopp-nested/portal.ready
