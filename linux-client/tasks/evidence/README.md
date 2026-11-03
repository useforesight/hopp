# Synthetic native UI evidence

`ui/` holds real native GPUI rendering of every Hopp surface, captured by
`../visual-check/run.sh` in an isolated headless Weston compositor, one window per
scene at that window’s real size, in the Tokyo Night theme. Users, presence,
cameras (4:3 frames, cropped by the tiles) and the shared screen (a synthetic
2560×1600 editor) are fixture data; these images do not demonstrate a live call.
Layer-shell surfaces (call card, sharing pill) fall back to plain windows there.

- `home-signin`, `home`, `home-call`, `settings`: the Hopp window at 420×680.
- `faces-2`, `faces-3`, `faces-large`, `faces-row`, `faces-bubbles`: the call
  window at exactly the size `faces::window_size` computes.
- `viewer-wide` (1400×820) and `viewer-narrow` (640×860): the shared screen as a
  tiled viewport, opened onto the sharer’s cursor, with a drawing, another
  participant’s cursor and the minimap.
- `peek`: the whole screen fitted on an ultrawide-shaped output.
- `sharing`, `call`: the sharing pill and the incoming call card.

`legacy-ui/` keeps the screenshots of the earlier single-window interface.

`bridge-live-call.txt` is the output of `../../dev/bridge-call-check.py`: a real
call between the two local fixture accounts, placed, accepted and left entirely
through the bar plugin’s bridge against the local Hopp backend and LiveKit. No
microphone, camera or screen share was opened.

No user desktop was captured or sent input by these checks.

## Isolated installed-Hyprland checks

`native-input-pass.txt` records the installed release binary's scoped pointer, click, typing, release-on-revoke and blocked-after-revoke checks in nested Hyprland 0.56.2. `native-input-focus-guard.txt` records a stopped attempt when the private compositor's own update notice took focus. The successful retry had no other clients or layers.

`native-overlay-pass.txt` and `native-overlays.png` demonstrate actual output-bound cursor/drawing rendering, cursor hide/remap, drawing clear/remap and removal on Drop. The background is an update notice inside the private test compositor. These are separate checks; concurrent input with overlays was not established.

The installed Hyprland/Aquamarine and Hopp binaries were unmodified. A narrow compatibility patch was applied only to the extracted wlroots parent inside the isolated test environment. Details and exact patch: `../nested-hyprland-check/README.md`. The real desktop stayed locked throughout.

## Local backend desktop demonstration

Later on 2026-10-06, after the user unlocked the desktop and requested a live
demo, the latest native release opened Publisher Local and Viewer Local
against the local Hopp backend on a separate workspace. Both accounts
authenticated; a connected call and real monitor feed arriving in the other
client were observed. The clients used the active Omarchy theme.

This is a manual desktop observation, separate from the synthetic screenshots
and isolated checks above. Desktop verification captures were deleted and
are not stored as evidence. Automated portal capture/stop, window capture,
clipboard round trips and broader interoperability remain acceptance work.
See [the project handover](../../HANDOVER.md).
