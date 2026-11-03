# Native Omarchy pairing client

Build the authorized Rust + GPUI client, keeping upstream Mac/Windows code
intact. The implementation uses Hopp’s existing backend and native LiveKit
messages. Feature-complete acceptance requires the remaining live checks.

## Implemented and verified

- [x] Native GPUI application and active Omarchy theme/font integration.
- [x] Bounded BGRA frame model, aspect-preserving rendering and atlas cleanup.
- [x] Hopp API sign-in, private token storage, browser-link IPC, rooms/presence.
- [x] Direct calls/invitations and early-cancel notification against real Hopp.
- [x] Native H.264/Opus/camera/data transport against actual API-issued tokens.
- [x] 5K source encoding within native codec bounds; 4K remains native.
- [x] Physical V4L2 camera capture/release and device preferences.
- [x] Actual native AEC with speaker reference, resampling and bounded callbacks;
      synthetic acoustic regression and repeated teardown pass.
- [x] Private session/capture queues discard events/frames from older operations.
- [x] Cancellable joins, tracked portal selection and late-consent rejection;
      ordered backend presence rollback. System chooser dismissal is manual.
- [x] Participant grants and bounded clipboard/annotation protocol handling.
- [x] Keymap/letterbox tests; cursor show/move/hide/cleanup on actual Hyprland.
- [x] Compositor lock notifier and atomic lock guard, reconnect grant removal.
- [x] Real native visual checks: wide, compact, calls, settings, camera previews.
- [x] Repeatable private Weston visual harness; labeled synthetic screenshots.
- [x] Passing default tests (58 after the interface rework); format check and strict Clippy.
- [x] Release build, user-local binary/icon/launcher and browser-link handler.

## Omarchy-native interface (7 October 2026)

- [x] Window-independent session model; engine pump at app level; per-window
      texture release. Closing any window never ends a call.
- [x] Call window: square cover-cropped tiles, S/M/L, column/row/bubbles, size
      derived from participants, corner-anchored resizing through Hyprland.
- [x] Shared screen as a tiled viewport: Follow / 1:1 / Fit, crop-aware input
      mapping, drag/scroll pan, zoom, minimap, fading pill; opens on the sharer.
- [x] Peek window in `special:hopp`; `--peek` / Super+H over the instance socket.
- [x] Linux sharers publish their pointer (Hyprland IPC socket, 20 Hz, only
      on the shared monitor) as upstream Hopp hosts do.
- [x] Layer-shell call card and sharing pill, excluded from screen sharing.
- [x] Hopp window as panel-sized sign-in / settings / fallback.
- [x] IPC v2: state snapshots (`subscribe`) and call/window commands.
- [x] `local.hopp` Omarchy bar plugin: panel, in-call pill, autostarts the client.
- [x] Installer: Hyprland rules + bindings, bar plugin; clean uninstall.
- [x] Fit/peek dock: screen stretched and blurred behind itself (TV-style, 75% opaque,
      rebuilt 4×/s from a 96-px copy), controls left / faces right (top / bottom on
      tall tiles), pill fallback for thin bands; transparent viewer window.
- [x] Call window frosted; hides while a docked viewer or peek is on a visible
      workspace (Hyprland event socket) and returns on other workspaces.
- [x] 63 tests, strict Clippy, 15 synthetic screenshots, live bridge call PASS (67 after the latency work).
- [ ] Live screen share between the two clients with the new viewer: follow,
      peek, Fit dock, call window hiding/returning across workspaces, sharing pill,
      control through the crop. First attempt: chooser
      shown, no share observed; lifecycle logging now on stderr.
- [ ] Bar plugin panel in a live call (only the signed-out panel was seen live).

## Rename and Capture (14 October 2026)

- [x] Renamed to `hopp`: binary, app ids, config (migrated), socket, journal, scope;
      `clap` command line with call commands; old flags kept.
- [x] Capture store (SQLite, FTS5), `hopp capture` read/search/follow/annotate/delete,
      `hopp mcp` (6 tools, no way to start a capture).
- [x] `hopp-whisper` helper; models with SHA-256; per-speaker utterances; resampler;
      `hopp capture transcribe` (JFK sample: 11 s of speech in 2.4 s, correct).
- [x] Live capture: tap on remote tracks and post-APM mic, `hopp.capture` attribute,
      events, bookmarks, hooks, Markdown copy; tried live in a real call.
- [x] Interface: capture control in the call window and viewer dock, badges, bar pill
      and call card, Captures window (live transcript + search), Settings section with
      model downloads, `Super+Alt+C`, `Ctrl+Shift+C` in Hopp windows.
- [x] 108 tests, strict Clippy, visual check updated (five controls).
- [x] GPU (Vulkan), batching with DTW word timings, helper in its own idle,
      memory-limited scope, idle unloading, measured limits (`tasks/capture-bench`).
- [x] Settings window (sidebar, rows), speaker colours from the Omarchy palette,
      capture retention, MCP tested with Claude Code and Codex (read-only hints).
- [ ] Captures window and Settings model download tried live.
- [ ] Read the client scope's `memory.peak` after a real call with a shared screen
      and set its limits from that (GPU memory counts).
- [x] Desktop frame pacing measured: GPU transcription stalls the compositor on
      Asahi (priority and smaller submissions don't help); CPU at idle is invisible.
      CPU (8 threads) and large-v3-turbo are the defaults; the GPU is a setting.
- [x] On a kernel with Aurora's G14X compute preemption, `frame_probe.py` with
      large-v3 on the GPU drops no frames; the GPU is now the default.
- [ ] Round 2: screen recording (VP9 per share), playback against the transcript,
      `capture_frame` for agents.
- [ ] Tune utterance thresholds on real crosstalk; consider participant names as a
      whisper prompt for better spelling.

## Latency, CPU and memory (9 October 2026)

Measured against the local fixture (real LiveKit, H.264, both ends in one process).

- [x] Zero playout delay, as upstream core: decoded frames shown at once
      (2560×1080 p50 127 → 19 ms; 720p 66 → 20 ms).
- [x] Engine and UI woken by what they consume (`wake.rs`): remote input, room data,
      frames and events handled on arrival instead of 25 ms / 16 ms ticks. Idle
      0.60 → 0.13% CPU, 552 → 113 context switches/s.
- [x] Capture converts PipeWire buffers straight to I420 on its thread (no 44 MB BGRA
      copy), capped at 30 fps; encoding runs off the engine loop; sharer pointer
      polled on its own thread.
- [x] 12 Mbps screen bitrate; shares start at 1920 and follow the bandwidth estimate
      up, settling at the largest size the single-threaded encoder sustains at 30 fps.
      5K on this M2 Max: first frame 0.35 s (was ~10 s); settles at 2560×1080,
      30 fps, ~30 ms (4096×1728 managed 15–20 fps at 90–140 ms).
- [x] Frames and pointer moves redraw only the viewer / call window; the bar plugin
      snapshot no longer rebuilds per frame. Camera tiles decoded at ≤360 px short
      side; the unused local screen preview is gone.
- [x] 67 tests, strict Clippy, 15 screenshots, live engine + media fixture tests PASS.
- [ ] Remote-control input latency and the encoder ceiling on a real network and on
      other machines (the ceiling adapts per machine; tune `SCREEN_ENCODE_BUDGET`).

## Memory incident (9 October 2026)

- [x] Desktop session loss explained: client in Hyprland's service + `OOMPolicy=stop`.
- [x] Client runs in its own capped scope (`scope.rs`); verified live (3G/4G, app.slice).
- [x] Receiving and encoding paths ruled out headlessly (flat memory over 3 minutes).
- [ ] Reproduce the 24.5 GB growth on a monitored live share (portal capture, being
      controlled); heaptrack would show the allocation site.

## Acceptance checks still required

- [x] Real monitor share observed between two native clients connected to
      local Hopp on the unlocked desktop.
- [ ] Repeatable portal monitor/window capture, frame delivery and clean stop;
      isolated capture automation and window capture remain to verify.
- [x] Actual scoped virtual pointer/keyboard and release-on-revoke probe.
- [x] Host drawing layer render/crop/clear/click-through and destruction in
      installed Hyprland inside a private desktop.
- [ ] Clipboard copy/cut/paste through two controlled clients.
- [ ] Display transforms, hotplug, scaling, tiled/floating/focus behavior live.
- [ ] Global emergency revoke outside the Hopp window.
- [ ] Mac/Windows client interoperability, long calls and impaired networks.

Runtime tests can proceed in a fully isolated nested Hyprland if physical
devices, actual session sockets, system/user buses and configuration are
excluded. Otherwise the remaining user-desktop checks require unlocking.
Do not unlock or wake the user’s desktop automatically.

## Known limits

Window sharing is view-only; control targets an identified monitor. IME
composition is not implemented. Escape is scoped to the focused Hopp window.
Legacy backend call rejection has no rejecting-peer identifier, so reliable
correlation of delayed outgoing-call declines needs an upstream API change.
Local self-hosted tests are authorized; managed Hopp remains the default
endpoint. Hosting provider/deployment preference is still open.
