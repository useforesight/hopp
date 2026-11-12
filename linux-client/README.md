# Hopp for Omarchy

A native Rust + GPUI pairing client for Hopp on Omarchy and Hyprland. The
interface uses `gpui-kit` and `gpui-omarchy`, as in Tobi’s DiskTree, plus an
Omarchy shell plugin for the bar. Calls, screen video, camera video and audio
use the native LiveKit SDK. WebKitGTK is not involved in rendering or media.

The client lives independently of Hopp’s existing Mac/Windows application.
It uses the same backend authentication, room tokens, presence socket and
input-message formats. The default server is `https://pair.gethopp.app`;
settings also accept your self-hosted Hopp origin.

See [implementation and lessons learned](HANDOVER.md) for the architecture,
completed work, verification status and remaining acceptance priorities.

## How it fits a tiling desktop

Hopp has no big main window. Each part of a call is a surface shaped for what
it shows:

| Surface | What it is | How Hyprland treats it |
| --- | --- | --- |
| **Bar app** (`local.hopp` plugin) | Teammates, rooms, presence, call controls; a live pill (faces, timer, mic, sharing dot, capture waveform) during calls. `Super+Alt+H`. | Omarchy shell panel |
| **Call window** (`hopp.faces`) | Centre-cropped camera tiles (S/M/L): square in a column, row or bubbles, wide in a grid or a speaker view (whoever is talking big, everyone else below). The sharer has a green ring and a "Sharing" badge that brings their screen to you. Call controls (mic, camera, share, capture, reactions, leave) sit on a frosted, see-through background. Grows by one tile per person; never stretches. Steps aside while the shared screen holds the faces and comes back on other workspaces. | Floating, pinned to every workspace, no initial focus, hidden from your own screen share, anchored to its nearest corner as it grows |
| **Shared screen** (`hopp.viewer`) | A *viewport* onto the remote screen: readable scale, cropped to whatever shape the tile has, following the sharer’s cursor, with minimap, Follow / 1:1 / Fit and a fading control pill. In **Fit**, the whole window behind the screen is the screen itself stretched to the window's shape and heavily blurred (TV-style, see-through onto the frosted desktop). Whose screen and the view controls sit in one band; the call card (faces sized to fill the band, plus the call controls) in the other. Left and right on wide tiles, top and bottom on tall ones. Otherwise the faces have a rail down the side of the screen (or along its bottom, or bubbles over its corner, as Settings says); a button by the view controls sends them to the call window instead, and back. | An ordinary tile |
| **Peek** (`hopp.peek`) | The whole remote screen fitted, with the same blurred bands and dock. `Super+H` in and out. | `special:hopp` workspace |
| **Call card** (`hopp-call`) | Incoming/outgoing call with Accept/Decline/Cancel. | Layer-shell overlay, top right; never takes keyboard focus unless clicked |
| **Sharing pill** (`hopp-sharing`) | “Sharing eDP-1 · Mira has control · Revoke · Stop”. | Layer-shell overlay under the bar, excluded from screen sharing |
| **Reactions** (`hopp-reactions`) | The tray from the call window's reactions button (emoji, effects, a soundboard on keys 1–8, a switch for their sounds), and the layer they play on for everyone in the call. | Layer-shell overlays: the tray beside the call window, the layer over the whole monitor with the call window, click-through and open only while something plays; both excluded from screen sharing |
| **Hopp window** (`hopp`) | Sign-in and the same teammates/rooms/call view without the plugin. | Floating, centred, panel-sized |
| **Settings** (`hopp.settings`) | Account and server, audio and video devices, call window, Capture (models, storage, agents, hooks), shortcuts. | Floating, centred |
| **Captures** (`hopp.captures`) | The live transcript while you capture, and every captured call, searchable. Narrow, just the transcript. Right-click the capture button, or `hopp capture window`. | An ordinary tile |

A tile has whatever shape the layout gives it while a screen has a fixed one,
so the default viewer never letterboxes into a tile; for the whole picture there
is Fit and peek, where the leftover space becomes a blurred copy of the screen
holding the call. The installed rules turn on Hyprland’s `decoration:blur` so the
see-through part is frosted (Omarchy ships with it off; it only affects see-through
windows); the blurred screen copy is drawn by the client either way. Window rules live in `packaging/hypr/hopp.lua`.

## Build, install and launch

Requires Rust 1.97+, a C/C++ toolchain, CMake, pkg-config, and the development
libraries for ALSA, PipeWire, Wayland, XKB, font rendering and Vulkan. The
native SDK downloads its prebuilt WebRTC dependency during the first build.
Arch/Omarchy packages used by the build include `clang`, `llvm`, `cmake`, `pkgconf`,
`alsa-lib`, `pipewire`, `wayland`, `libxkbcommon`, `fontconfig`, `freetype2`,
`vulkan-headers` and your GPU’s Vulkan driver. Clipboard operations use
`wl-clipboard`.

```sh
make install                              # binaries, launcher, links, Hyprland rules, bar plugin
make install INSTALL_FLAGS=--enable-plugin # …and place Hopp in the bar
make uninstall                            # removes all of the above
```

Installation is user-local. It writes `~/.local/bin/hopp` and its transcription
helper `~/.local/bin/hopp-whisper`, the desktop entry and icon, `~/.config/hypr/hopp.lua` plus one marked
`require("hypr.hopp")` line in `~/.config/hypr/hyprland.lua`, and the plugin in
`~/.config/omarchy/plugins/local.hopp`. `--no-hyprland` and `--no-plugin`
(via `INSTALL_FLAGS`) skip those parts. The installer remembers the previous
Hopp URL handler and restores it on uninstall. Installing over the old
`hopp-omarchy` build removes its files; the first start moves
`~/.config/hopp-omarchy` to `~/.config/hopp`, keeping you signed in. Upstream
Hopp's Linux package also ships a `hopp`; `~/.local/bin` normally comes first
in `PATH`, and the bar plugin always prefers `~/.local/bin/hopp`.

With the plugin in the bar, Hopp starts itself in the background when the shell
starts. Sign in through your normal browser; the installed app handles Hopp’s
return link. An access-token field is available for local development. Tokens
are saved in the user’s private Hopp configuration with mode 0600.

```sh
hopp                  # start, or bring the Hopp window forward
hopp --background     # start without a window (what the bar plugin does)
hopp status           # who's online and what's happening in your call (--json)
hopp call sam       # call a teammate by name; join, accept, decline, leave
hopp set-status dnd   # available, away or dnd (Do not disturb)
hopp mute | camera | share
hopp panel            # Super+Alt+H: bar panel, or the Hopp window without it
hopp peek             # Super+H
hopp revoke           # Super+Ctrl+Esc, from anywhere
hopp settings | quit
hopp --doctor         # desktop capabilities as JSON
```

The old flags (`--peek`, `--panel`, `--settings`, `--quit`, `--revoke-control`)
still work.

## Capture

Capture transcribes a call on this computer and keeps it there; nothing is
uploaded. Start it with the capture button in the call window or the bar's call
card, `Super+Alt+C`, or `hopp capture start`. Others in the call see a badge on
your tile if they use this client; Hopp for Mac and Windows shows nothing.

- Each person's audio is transcribed separately (yours from the microphone after
  echo cancellation), so every line has the right speaker, in a colour from your
  Omarchy theme. Joins, shares, control changes and bookmarks are noted in
  between.
- whisper.cpp runs in `hopp-whisper`, a separate process in a systemd scope of its
  own (`app-hopp-whisper-*`) that only gets idle CPU and has its own memory limit.
  By default it runs on the GPU through Vulkan. On Apple silicon that needs a
  kernel whose GPU driver lets the firmware preempt compute work: Aurora
  2026.10.11.14 or later, or the boot option `asahi.debug_flags=281474976710656`.
  Without that the compositor waits behind each pass and the cursor and typing lag
  (measured: 15% of frames dropped, stalls of 200 ms; with the fix, none). On such a
  kernel choose the CPU in Settings → Capture → Runs on: up to 8 threads on idle
  time only, so the desktop stays smooth, at two to three times faster than speech.
  If the GPU fails, the helper carries on with the CPU. After 90 quiet seconds the helper exits and gives its
  memory back, and reloads when someone speaks.
- Utterances are sent several at a time and split back by word timings: about
  2.5 times less GPU time than one at a time, at the same or better accuracy. Each
  person's utterances sit together in a request, so a word never moves to someone
  else. Text therefore arrives in batches, up to about half a minute after it was
  said, and lands where it was said: transcripts are in time order, with each
  person's consecutive lines joined into one message.
- The default model, `large-v3-turbo` (1.6 GB, 2 GB while transcribing), downloads
  on first use and is checked against a SHA-256 built into Hopp. `large-v3` is the
  most accurate: about four times faster than speech on the GPU, but only about as
  fast on the CPU, so there it can fall behind. Choose in Settings → Capture or with
  `hopp capture model use large-v3`. If the GPU fails, the helper carries on with the
  CPU.
- If transcription falls more than ten minutes behind, the oldest audio is
  skipped and marked as not transcribed; the call is never slowed down.
- Everything is in `~/.local/share/hopp/capture`: an SQLite database (0600) and a
  Markdown copy of each call, written when the capture ends. Settings can delete
  captures after 30 or 90 days.
- Measurements and the scripts behind them: [tasks/capture-bench](tasks/capture-bench/README.md).

```sh
hopp capture start | stop | toggle    # toggle is Super+Alt+C
hopp capture bookmark the fix         # mark a moment
hopp capture window                   # live transcript and past calls
hopp capture list                     # newest first (--json)
hopp capture show latest              # Markdown (--json); an id, latest or current
hopp capture search xrun
hopp capture follow                   # new lines as they're said
hopp capture follow --on-wake         # wait until someone says "Jeepers…", print it, exit
hopp capture annotate ID --title T --summary S   # or --json on stdin
hopp capture delete ID
hopp capture model list | download NAME | use NAME
```

**Agents.** `hopp mcp` is an MCP server on stdin/stdout, tested with Claude Code
and Codex:

```sh
claude mcp add hopp -- hopp mcp
codex mcp add hopp -- hopp mcp
```

Its tools are `call_status`, `capture_list`, `capture_search`,
`capture_transcript`, `capture_follow` (waits for new lines, or for a wake phrase;
45 seconds by default, since agents often give up on a tool after 60) and
`capture_annotate`. The reading tools are marked read-only, so agents run them
without asking. Agents can read and annotate captures but cannot start one.

**Asking an agent in a call.** Say "Jeepers" anywhere in what you ask:
"Jeepers, why is that failing?" or "…what do you reckon, Jeepers?". Hopp tells
agents to listen for it; `--on-wake` or `wake_phrase` can name another. It wakes once you've finished talking: after 2 seconds of quiet, or when
someone else speaks. A short reaction like "yeah" doesn't count. The reply holds
your words as `asked`, the lines before them, and `next`. `next` tells the agent
how much of the call it hasn't seen and to read back and look at the frames before
answering.

While you capture, Hopp shows at once that it heard "Jeepers", a second or two after
it's said and well before any agent answers: a light runs round the speaker's tile,
and a bubble beside the call window, pointing at their tile, echoes their words with
the word lit up (under the bar when the faces are docked round a shared screen). Both go a few
seconds after they stop talking, or at once if the final text turns out not to have
the word. Hopp can't tell when an agent answers, and only "Jeepers" is shown, not
another phrase given to `--on-wake`.

**Off the record.** Say "tinfoil hat" or "off the record" and every capture in the
call stops keeping anything for two minutes, on every client at once: no words, no
frames of the shared screen, nothing for an agent following the call. The sentence
with the phrase in it is dropped too, and the transcript gets one line when it starts
and one when it ends. A pill under the bar counts down, turns amber and beeps through
the last five seconds, then the capture carries on. `+2 min` on the pill adds two
minutes (up to ten at once), `Resume` ends it now, and saying the phrase again starts
the two minutes afresh. Whoever does any of these, everyone's Hopp follows. A capture
still listens while off the record, in memory, so that it can hear the phrase again.
It takes a second or two to hear it, as with "Jeepers", so say it before the thing
you don't want kept. Everyone needs a version of Hopp that knows about it: an older
one keeps capturing.

**Reactions.** The call window's party button opens a tray of reactions that play on
everyone's screen, with sound: eight quick emoji; confetti (tap again quickly for
more), a gong, a flipped table, a slow clap that everyone can join, fireworks (click
for three rockets, hold for up to ten), an achievement for someone, a disco and a
"ship it" launch; and a soundboard, also on keys 1–8 while the tray has the
keyboard. The tray stays open until Escape or the button again. They play on a
click-through layer over the monitor with the call window, kept out of your screen
share, and only over a monitor you share when the `hopp-share-clean` plugin is
loaded (otherwise you only hear them). The tray's switch turns their sounds off on
this computer. The Mac app sends and plays the same reactions.

**Hooks.** Executables in `~/.config/hopp/hooks/capture-started` and
`capture-complete` (or in `capture-complete.d/`) run in the background with
the event as JSON on stdin: `id`, `room`, `title`, `participants`, `started_ms`,
`ended_ms`, `db` and, when complete, `markdown`. They are stopped after ten
minutes. For example, to have Claude write a title and summary:

```sh
#!/bin/sh
id=$(jq -r .id)
hopp capture show "$id" |
  claude -p 'Reply with JSON: {"title": "...", "summary": "..."} for this call' |
  hopp capture annotate "$id" --json
```

## Desktop behavior

- Active Omarchy colours and font everywhere, following theme changes live;
  people get stable colours derived from the theme accent.
- Monitor and window selection through the desktop’s ScreenCast portal, with
  direct PipeWire capture from the portal-provided file descriptor.
- Sharing a monitor publishes your pointer position (normalized, like upstream
  Hopp hosts), which viewers follow.
- Explicit participant control grants through Hyprland’s virtual keyboard and
  virtual pointer protocols; no privileged input daemon is required.
- Remote clicks map through the viewer’s crop and pan; clicks outside the
  visible image never reach the peer. Without control, dragging pans the view
  and Ctrl+scroll zooms.
- Output-bound cursor and drawing layers with empty input regions.
- Sharing and remote input stop when Hyprland’s lock notifier reports a lock.
- Statuses, as on the Mac: pick Available, Away or Do not disturb in the bar
  panel, the Hopp window or Settings, and teammates see theirs beside their name.
  Do not disturb turns calls and invitations away, tells the caller, and lists
  them as missed, with the card held back until you turn it off. Locking the
  screen sets you Away until you unlock, except during a call (Settings → Away
  while locked); a status you chose yourself stays. Needs a server with statuses
  (Foresight's backend from 6 November 2026); on others all of this stays hidden.
- Screen shares sent as H.265 (5K from a Mac) are decoded through the system
  FFmpeg, on the hardware through VA-API when there is a decoder (on Apple
  silicon, the AVD with `avd-fw` and `libva-v4l2-request`), else in software.
  `HOPP_H265_DECODER=0` turns it off; `HOPP_H265_HWDEC=0` keeps it in software.
- Private single-instance IPC for launchers, browser return links and the bar
  plugin (state snapshots and call commands over a 0600 socket).

Remote control requires an identifiable shared **monitor**. Window sharing is
view-only: the portal does not supply enough stable window geometry to safely
inject input into a moving window. IME composition and arbitrary non-Latin text
injection are not yet implemented; ordinary layout-aware keys and shortcuts use
the compositor’s XKB keymap.

## Verification

Development is verified on ARM64 Omarchy and Hyprland 0.56.2. The local fixture
runs real Hopp, PostgreSQL, Redis and LiveKit in isolated rootless containers;
see [dev/README.md](dev/README.md).

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace                   # 108 tests
hopp capture transcribe speech.wav                # the capture pipeline on a 16-bit WAV
./tasks/visual-check/run.sh                       # real GPUI screenshots, isolated Weston
python dev/bridge-call-check.py                   # live call through the bar plugin bridge
HOPP_API_TEST_FIXTURE=/path/to/private/client-test.json \
  cargo test real_engine_join_cancel_and_presence_cleanup -- --ignored --nocapture
```

The geometry behind the viewer (fit, cover, crop-aware click mapping, follow
dead zone, pan clamping) and the call window’s sizing and corner anchoring are
unit-tested. Real native rendering of every surface is captured with synthetic
users and media in [tasks/evidence/ui](tasks/evidence/README.md). Against the
local backend, the engine lifecycle test passes and a full call placed and
accepted through the bar plugin’s bridge connects both clients and clears on
leave. Hyprland accepts the installed rules and bindings without errors.

Earlier native checks (still applicable, unchanged media/input code): H.264
screen publication and decoding, Opus transport, separate camera tracks,
5120×2160 → 4096×1728 encoder bounds, USB camera capture/release, scoped
virtual input and overlays in an isolated installed Hyprland, and a WebRTC /
PipeWire symbol-collision regression test.

Not yet verified live with the new interface: screen sharing between two clients
(the chooser appeared in a first attempt but no share was observed), following
the sharer’s cursor, peek in `special:hopp`, and the sharing pill. Clipboard
round trips, cross-platform sessions with the Mac/Windows clients, long calls
and network disruption need separate acceptance testing. This is not yet a
verified feature-complete release.

Reference implementation: Hopp commit
`0350cedab2d69eeff572d7aca2ac161f1e3aef53`. Native dependencies are pinned in
`Cargo.lock`; the LiveKit SDK revision is pinned in `Cargo.toml`.

The inspected Hopp backend’s legacy `call_reject` message does not identify
the rejecting peer or validate that its pending call still exists. A delayed
decline while switching between outgoing calls cannot be correlated reliably
by a compatible client. Accepted sessions ignore such generic declines;
complete ring correlation requires an upstream backend protocol change.
