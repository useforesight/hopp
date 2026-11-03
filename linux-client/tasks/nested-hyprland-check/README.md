# Isolated installed-Hyprland test environment

These are the exact configuration, launcher, session worker, portal launcher and overlay probe used on the Omarchy Apple Silicon machine on 2026-10-06. They are test infrastructure; no compositor library was installed or changed on the user's system. The current prepared environment lives at `/tmp/hopp-nested` and the extracted Weston package at `/tmp/hopp-weston/root`.

The chain is Weston 15.0.1 headless GL → extracted labwc 0.20.2 / wlroots 0.20.2 → installed Hyprland 0.56.2. Weston only advertises compositor v5, while Aquamarine 0.15.0 requests v6, so labwc supplies the compatible parent protocol. Native rendering and grim capture happen on the innermost `WAYLAND-1` output, 1280×720 at scale 1.

Bubblewrap supplies a separate user/PID/IPC/UTS/network namespace, private `/run`, private `/dev` and a private runtime/config/cache/data directory. The only real GPU device exposed is `/dev/dri/renderD128`, used for rendering; display-card nodes and physical input/audio devices are absent. Host D-Bus/logind/seat/systemd sockets are hidden. `dbus-run-session` creates the test bus. `LIBSEAT_BACKEND=hopp-disabled` fails explicitly without backend fallback, and Hyprland systemd environment updates/notifications are disabled. `WLR_BACKENDS=wayland` selects the labwc parent backend. The session verifies device/socket isolation before starting any compositor.

## Test-parent compatibility patch

Unmodified Aquamarine submits its first non-null framebuffer before acknowledging its parent's initial `xdg_surface.configure`. The protocol trace proves the labwc parent rejected it with `xdg_surface has never been configured`; the innermost compositor then had working input protocols but no frame presentation. A destroyed cursor could remain as a frozen fade snapshot with pid -1. That snapshot did not establish a live surface leak.

The approved `wlroots-test-parent.patch` replaces only that initial unconfigured-buffer rejection with `surface->configured = true`. It intentionally relaxes an XDG-shell protocol requirement in the disposable test parent. **Never install or ship this library.** It does not validate Aquamarine's nested configure ordering. Hopp, installed Hyprland and installed Aquamarine remain unchanged. After the patch, native frames, grim capture and overlay destruction all completed.

Build inputs:

- Official wlroots archive: `https://gitlab.freedesktop.org/wlroots/wlroots/-/archive/0.20.2/wlroots-0.20.2.tar.gz`
- Extracted Arch ARM `weston-15.0.1-3`, `labwc-0.20.2-1`, `wlroots0.20-0.20.2-1`, `libsfdo-0.1.4-1`, `libliftoff-0.5.0-1` packages, resolved with `pacman -Sp --print-format '%n %l' weston labwc`.
- Meson 1.9.2 release archive from `https://github.com/mesonbuild/meson/releases/download/1.9.2/meson-1.9.2.tar.gz` and extracted Arch Ninja 1.13.2.

Apply the patch to the extracted source, then configure/build in `/tmp/hopp-nested/wlroots-build`:

```sh
python3 /tmp/hopp-nested/build-tools/meson-1.9.2/meson.py setup \
  /tmp/hopp-nested/wlroots-build /tmp/hopp-nested/wlroots-source \
  -Dbackends=auto -Drenderers=gles2 -Dallocators=gbm \
  -Dxwayland=enabled -Dexamples=false -Dsession=enabled \
  -Dcolor-management=disabled -Dlibliftoff=disabled
PATH=/tmp/hopp-nested/build-tools/usr/bin:$PATH \
  /tmp/hopp-nested/build-tools/usr/bin/ninja -C /tmp/hopp-nested/wlroots-build -j 8
```

The full set of backend symbols is needed for ABI compatibility with the extracted labwc executable; runtime still selects Wayland only and physical devices/session sockets remain absent. The compatibility library is loaded only via the private session's `LD_LIBRARY_PATH`.

## Using the prepared sandbox

Copy these scripts/configuration into `/tmp/hopp-nested` and run `launch.sh` after preparing its extracted packages/build and private runtime directories. The launcher expects the listed machine's render node; adjust that explicit node only when preparing a different test machine. Never substitute display-card or input nodes.

`session.sh` waits for its own compositor lock and exports its own Wayland socket and Hyprland instance signature. It executes an action file inside that existing namespace and bus. Write an approved action to a temporary file and rename it atomically to `/tmp/hopp-nested/action.sh`. Each action inherits the private environment. Root tests used the installed release binary copied to `/tmp/hopp-nested/hopp-omarchy` with `--input-test`. `overlay_probe.rs` compiles against the real `native_input.rs` / `cursor_overlay.rs` modules and accepts only the exact test runtime.

`start-portals.sh` launches private PipeWire, the Hyprland portal implementation and desktop portal core. Its explicit runtime/device guards prevent running it against the user's ordinary bus. It updates only the private activation environment, without `--systemd`. Private portal configuration selects Hyprland ScreenCast/Screenshot and `/usr/bin/hyprland-share-picker`; all selections and input automation must target only owned windows inside this sandbox.

The default network namespace has no host backend connection. For authorized localhost backend tests, run `HOPP_TEST_HOST_NETWORK=1 /tmp/hopp-nested/launch.sh`. This removes only `--unshare-net`; all other namespace/device/session guards remain. The installed user compositor and host session bus remain inaccessible. Root requested this optional host network mode for two-client engine tests after the initial input/overlay proofs.

Evidence is under `../evidence/`. Native input, focus-loss guards and overlay lifecycle/rendering passed. Portal and live-call checks are separate and should record their own outcome. Shutdown only the owned bubblewrap launcher; its PID namespace contains all test children.
