#!/bin/bash
# Installs the latest Foresight build of Hopp on a Mac, replacing any existing copy.
#
# Hopp isn't notarized by Apple. Files fetched with curl aren't quarantined, so macOS doesn't
# block the app on first open. The signature is checked here instead.
set -euo pipefail

RELEASES="https://github.com/useforesight/hopp/releases/latest/download"
SIGNED_BY='certificate leaf = H"afe084bb990bb282dd1ab3ea5f88d32a504c5b3e"'
INSTALLED_APP="/Applications/hopp.app"

case "$(uname -m)" in
  arm64) arch="aarch64" ;;
  x86_64) arch="x64" ;;
  *)
    echo "Hopp doesn't support this Mac ($(uname -m))."
    exit 1
    ;;
esac

[[ -w /Applications ]] || {
  echo "You need to be an admin on this Mac to install to /Applications."
  exit 1
}

core_pid="$(pgrep -x hopp_core | head -1 || true)"
if [[ -n "$core_pid" ]] && lsof -nP -iUDP -a -p "$core_pid" 2>/dev/null | grep -q UDP; then
  echo "You look to be on a call, and installing would end it. Run this again afterwards."
  exit 1
fi

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

echo "Downloading Hopp..."
if ! curl -fsSL "$RELEASES/hopp_$arch.app.tar.gz" | tar -xz -C "$work_dir"; then
  echo "Couldn't download Hopp, and nothing was changed. Check your connection and try again."
  exit 1
fi
codesign --verify --deep --strict -R="$SIGNED_BY" "$work_dir/hopp.app" || {
  echo "The download isn't signed by Foresight, so it wasn't installed."
  exit 1
}

quit_hopp() {
  pgrep -x hopp >/dev/null || pgrep -x hopp_core >/dev/null || return 0
  echo "Quitting Hopp..."
  osascript -e 'tell application id "com.hopp.app" to quit' >/dev/null 2>&1 || true
  for _ in {1..20}; do
    pgrep -x hopp >/dev/null || pgrep -x hopp_core >/dev/null || return 0
    sleep 0.5
  done
  pkill -x hopp || true
  sleep 1
  pkill -x hopp_core || true
}

# macOS ties permissions to the app's signature, so a copy signed by someone else leaves entries
# that look enabled but don't work.
replaces_other_build=false
if [[ -d "$INSTALLED_APP" ]] && ! codesign --verify -R="$SIGNED_BY" "$INSTALLED_APP" 2>/dev/null; then
  replaces_other_build=true
fi

quit_hopp

ditto "$work_dir/hopp.app" "$INSTALLED_APP.new"
if [[ -d "$INSTALLED_APP" ]]; then
  # The old app goes last, so there's still a working Hopp if macOS refuses the swap.
  rm -rf "$INSTALLED_APP.old"
  if ! mv "$INSTALLED_APP" "$INSTALLED_APP.old"; then
    rm -rf "$INSTALLED_APP.new"
    echo "macOS stopped this replacing Hopp. Allow your terminal app under System Settings >"
    echo "Privacy & Security > App Management, then run this again."
    exit 1
  fi
fi
mv "$INSTALLED_APP.new" "$INSTALLED_APP"
rm -rf "$INSTALLED_APP.old" || echo "Couldn't delete $INSTALLED_APP.old. It's safe to move to the Bin."

if [[ "$replaces_other_build" == true ]]; then
  tccutil reset All com.hopp.app >/dev/null 2>&1 || true
fi

open "$INSTALLED_APP"

echo "Hopp is installed and opening. It updates itself from now on."
if [[ "$replaces_other_build" == true ]]; then
  echo "macOS will ask for Hopp's permissions again, so grant them when it does."
fi
