# Hopp Omarchy implementation and lessons learned

Updated 7 October 2026 (interface rework: see “Omarchy-native interface”). Originally written 6 October 2026. The project now has a native Rust + GPUI Hopp client
under `linux-client/`, built in the style of Tobi’s DiskTree with Omarchy theme
integration. Two clients authenticated against a local Hopp backend, connected
a call, and displayed a real monitor share on the unlocked desktop. This is a
development build; the remaining acceptance checks are in [tasks/todo.md](tasks/todo.md).

## Architecture and scope

The client uses `gpui-kit` and `gpui-omarchy` for the interface, Hopp’s existing
HTTP and presence APIs for accounts and calls, and native LiveKit/WebRTC for
media and participant messages. Screen capture uses the ScreenCast portal and
its PipeWire file descriptor. Camera capture uses V4L2; audio uses CPAL and
WebRTC audio processing. Hyprland virtual keyboard/pointer protocols provide
monitor-scoped input, and layer-shell provides cursor and drawing overlays.

WebKitGTK is absent from this client’s rendering and media path. This avoids
the webview dependency, while Linux capture, input, audio and lifecycle support
remain explicit responsibilities of the native client.

All implementation additions are in `linux-client/`. The existing backend,
core and Tauri applications remain unchanged. Compatibility work references
Hopp commit `0350cedab2d69eeff572d7aca2ac161f1e3aef53`; the LiveKit SDK revision
and Rust dependencies are pinned. Managed Hopp remains the default endpoint,
with a configurable self-hosted origin. Railway or other production deployment
has not been selected or implemented.

## Work completed

- Built the native interface: browser sign-in, teammates, rooms and presence,
  direct calls, invitations, participant controls, settings and device choices.
- Added Omarchy colors and font integration, compact layouts, separate screen
  and camera previews, aspect-preserving video and letterbox-aware input.
- Implemented native H.264 screen/video transport, Opus audio, camera tracks,
  device selection and frame cleanup when media stops.
- Added native acoustic echo cancellation with the actual mixed speaker signal
  as its reference, persistent resampling and bounded audio queues.
- Implemented portal monitor/window selection, PipeWire capture, participant
  control grants, keyboard/pointer forwarding, drawing and clipboard commands.
- Added lock-state guards, release on revoke/disconnect, stale-operation
  isolation, cancellable joins and rejection of late sharing consent.
- Added private settings and sign-in IPC, an IPC-only `--revoke-control` command,
  and user-local binary, icon, launcher and browser-link installation support.
- Built a local backend fixture and repeatable native UI, input, overlay and
  media checks. See [dev/README.md](dev/README.md) and [tasks/evidence](tasks/evidence/README.md).

## Verification status

Development ran on ARM64 Omarchy, installed Hyprland 0.56.2, and a
5120×2160 display at scale 1.6.

| Area | Evidence and limits |
| --- | --- |
| Build and default suite | Release build, formatting and strict Clippy passed; 43 default tests passed and 7 integration tests are explicitly ignored by default. |
| Hopp backend | Real authentication, room tokens, direct calls, join cancellation, rejoining and presence cleanup were exercised against local Hopp. |
| Native transport | H.264 encode/decode, Opus sample delivery, separate camera tracks and reliable participant messages passed with Hopp-issued tokens. The 5K encoding regression produces 4096×1728; 3840×2160 stays native. |
| Camera and audio processing | Physical camera capture/release passed. Echo cancellation passed synthetic delayed-echo and teardown checks; real speaker/microphone acoustics need acceptance testing. |
| Interface | Real GPUI rendering passed wide and compact visual checks. Saved UI screenshots use synthetic users and media. |
| Input and overlays | Scoped pointer movement, clicks, typing, release on revoke, focus-loss guards and overlay cleanup passed in an isolated installed-Hyprland session. |
| Real desktop demonstration | Publisher Local and Viewer Local opened against the local backend; the connected call and monitor feed arriving in the other window were observed. This was a manual demonstration, separate from the isolated acceptance harness. |
| Portal automation | A native linking crash was fixed. Subsequent isolated attempts encountered missing frames or chooser timeouts; the automated capture/stop check remains incomplete. Window capture and clipboard round trips remain to verify. |

The demo launched the latest release directly from the build target. The
previous user-local installation should be refreshed before comparing it with
the latest source. Personal desktop verification captures were deleted and
were not added to the project’s evidence.

## Omarchy-native interface

The single 1280×820 window was replaced on 7 October 2026 by surfaces shaped for
what they show, following an HTML prototype (`prototype/hopp-omarchy.html`):
a bar plugin, a floating fixed-size call window, the shared screen as a tiled
viewport, a peek in `special:hopp`, a layer-shell call card and sharing pill,
and a panel-sized Hopp window for sign-in and settings. The README has the
table. Media, capture, input and backend code are unchanged except for the
sharer publishing its pointer and the IPC additions.

`src/ui/`:

- `session.rs` owns all call state and drains engine events and frames on an
  app-level timer, so windows come and go without affecting the call. Frames
  are decoded once; `painted.rs` releases textures per window.
- `mod.rs` (`Shell`) opens and closes windows from session state, remembers
  windows the user closed during a call or share, and publishes JSON snapshots
  for the bar plugin.
- `geometry.rs` is pure, tested viewport math; `hypr.rs` asks Hyprland to resize
  the call window, anchored to its nearest corner.
- `faces.rs`, `viewer.rs`, `toast.rs`, `sharing.rs`, `home.rs` are the surfaces;
  `style.rs` derives everything from the live Omarchy theme.

`omarchy-plugin/local.hopp/` is the bar plugin (QML + a stdlib Python bridge),
`packaging/hypr/hopp.lua` the rules and bindings. `--panel`, `--peek`,
`--settings`, `--background` are new launch options.

## Capture and the `hopp` command (14 October 2026)

The client was renamed from `hopp-omarchy` to `hopp`: binary, app ids (`hopp.faces`,
`hopp.viewer`, …), config `~/.config/hopp` (moved on first start, `paths.rs`), runtime
socket `$XDG_RUNTIME_DIR/hopp`, journal identifier `hopp`, scope `app-hopp-<pid>`. The
command line is `clap` (`cli.rs`); old flags are hidden aliases. The crate is now a
workspace with `whisper/`, the transcription helper.

Capture (`src/capture/`) transcribes calls locally, after Tuple's Capture (see
`docs/tuple-comparison.md` and `prototype/hopp-capture.html`):

- **Audio tap.** `capture::Tap` is shared by every audio path of a call: each remote
  track's 48 kHz task in `media.rs` and the microphone after APM. Idle it costs one
  atomic load per 10 ms frame; capturing, frames go to the recorder with `try_send`.
- **Recorder** (`recorder.rs`, its own thread): per-speaker utterance splitting
  (`vad.rs`, level above the quietest moment of the last two seconds), batching
  (`batch.rs`: utterances in one request, each person's together a second apart and
  3 s between people, sent when 24 s are waiting or the oldest has waited 12 s, split
  back by DTW timings a whole word at a time), clean-up (`text.rs`), lines into SQLite
  (`store.rs`, WAL, FTS5, schema 2: each speech line has `end_ms`). Lines are written
  as transcription finishes, so out of order; `Store::transcript` reads them by time,
  and `store::messages` joins one person's consecutive lines (pauses up to 5 s, nothing
  between) into messages for Markdown, `show`, MCP and the Captures window. `follow`
  keeps write order so a cursor never misses a line. Drops and skipped backlog
  become "not transcribed" lines. A stopping capture waits half its backlog (30 s to
  5 min). The helper is unloaded after 90 quiet seconds and reloaded on speech.
- **Helper** (`whisper/`, `hopp-whisper`): whisper.cpp via whisper-rs, at full
  precision. On the GPU through Vulkan by default (first non-software device; no
  flash attention, which disables DTW); on the CPU when the Capture setting says so
  (all but four cores, at most eight threads, idle time only).
  Length-prefixed JSON + f32 PCM in; JSON lines out, one timed token per segment.
  Runs in its own systemd scope (`scope::isolated`: `MemoryMax` 1.5× the model +
  512 MB, `CPUWeight=idle`, no swap). Replies carry a generation so a replaced
  helper's last words are ignored; after a crash it restarts once, then falls back
  from the GPU to the CPU.
- **Measurements** are in `tasks/capture-bench/` (scripts and results).
- **Engine** (`start_capture` / `stop_capture`): sets the LiveKit attribute
  `hopp.capture` on the audio room (best effort; the user chose no consent step),
  writes joins, shares and control changes, finishes in `spawn_blocking` after the
  call so leaving never waits, then writes the Markdown copy and runs hooks.
- **Reading:** `hopp capture …` and `hopp mcp` read the database directly; `follow`
  polls it every 300 ms. The Captures window does the same at 600 ms.

## Lessons learned

### A tile has the compositor’s shape; a screen has its own

Fitting a fixed-aspect screen into an arbitrary tile always leaves bars, and
floating it opts out of tiling. Treating the tile as a viewport (readable scale,
cropped, following the sharer) makes the tile’s shape irrelevant; the whole
screen is one keystroke away in a special workspace. Click mapping must then go
through the crop, not just the letterbox.

### Hyprland owns the size of a mapped floating window

A client resizing itself after mapping is ignored for floating windows. The call
window asks Hyprland directly (`hl.dsp.window.resize/move` with an
`address:` selector) and keeps its nearest corner fixed. `hyprctl dispatch`
takes Lua on 0.56; window rules and layer rules use the same `o.window` /
`hl.layer_rule` forms as Omarchy’s own files, and `hyprctl configerrors` shows
whether they were accepted.

### Omarchy shell plugins

QML has no Unix sockets here, so plugins talk through a long-lived Process with
JSON lines, as `local.otuple` does. `omarchy-shell shell toggle <id>` returns
success even for unknown ids, and `listPlugins`’ `active` means “panel open”,
not “loaded”; check `enabled`. `omarchy plugin enable` can race a rescan, so the
installer retries it.


### Native rendering and native media solve different problems

GPUI removes the WebKitGTK dependency and fits the Omarchy interface well.
Capture and control still need platform-specific implementations. Keep those
modules separate from the UI so permissions, failures and cleanup can be
tested independently.

### Inspect native symbol resolution when capture crashes

The first accepted portal capture crashed at `pipewire::init`. WebRTC’s static
archive supplied weak `pw_*` forwarding stubs whose private function pointers
were uninitialized, and the linker selected them over the system library.
Loading the correct PipeWire shared library alone did not prevent the collision.

The fix vendors the pinned `webrtc-sys` build and renames definitions and
internal references in a private copy of the WebRTC archive. It leaves runtime
`dlsym` names, installed libraries and the shared SDK cache intact. Renaming
only `pw_init` would leave the other collisions. Linux builds require
`llvm-nm` and `llvm-objcopy`; a default test now initializes system PipeWire
and creates/destroys its main loop. Details: [vendor patch](vendor/webrtc-sys/HOPP-PATCH.md)
and [crash diagnosis](tasks/evidence/capture-linker-regression.md).

### Canceling a future does not necessarily dismiss a portal chooser

Hyprland portal 1.4.1 runs its picker synchronously on its D-Bus dispatch
thread. It cannot handle close requests while that picker is waiting. Aborting
the client future left orphan choosers in the test desktop.

The client now retains and drains a canceled selection, blocks duplicate
requests, gives immediate cancellation feedback and rejects late consent
before starting capture. Session generations and private frame queues keep
old results out of a newer call. The system chooser still needs dismissal;
shutdown cannot guarantee dismissal with this backend version.

### Separate harness failures from product failures

The private desktop needed Weston → labwc/wlroots → installed Hyprland because
of compositor protocol-version differences. Aquamarine’s initial configure
ordering also required a narrow patch in the disposable parent compositor.
That patch must never ship or be installed on the user’s desktop.

Chooser timing, orphan dialogs and PipeWire session management complicated
automation. The later real-desktop monitor demonstration succeeded, so a
failed nested check alone does not establish a product capture failure. Keep
manual observations, synthetic rendering, transport tests and full acceptance
checks separately labeled. See the [isolated harness](tasks/nested-hyprland-check/README.md).

### Consent and geometry must stay attached to the selected source

Input targets an identified monitor, accounts for its scale/rotation, and
rejects clicks in video letterboxing. Window sharing is view-only because
stable window geometry is unavailable through the selected portal path.
Locks, disconnects and revocation must release held keys/buttons and remove
grants. Clipboard work and overlays need the same session and consent guards.

### Echo cancellation needs the real playback reference

Noise suppression alone does not establish echo cancellation. Feed WebRTC
audio processing the final mixed speaker signal and the capture/playback
delay. Process fixed audio blocks with bounded queues and avoid blocking or
allocating in device callbacks. Synthetic acoustic tests are useful regressions;
real devices and long calls still determine practical audio quality.

### Account for codec limits before publishing large displays

The native H.264 path needs bounded dimensions and pixel counts. Scaling
5120×2160 to 4096×1728 preserves aspect ratio and keeps the encoder within its
limits. The displayed frame geometry must also drive remote input mapping.

### Never run the client inside the compositor's service

On 9 October 2026 the client (old 7 October build) reached 24.5 GB and the kernel
OOM killer killed it. It had been started by the Omarchy bar (Quickshell), so it lived in
`wayland-wm@hyprland.desktop.service`, whose `OOMPolicy=stop` made systemd stop the whole
desktop session. `scope.rs` now re-executes the long-running client into
`app-hopp-<pid>.scope` (`app-hopp-omarchy-<pid>` before the rename) under `app.slice` (MemoryHigh 3G, MemoryMax 4G) whatever
launched it; a runaway client is then the only thing killed, and the bar restarts it.
The leak itself did not reproduce headlessly: receiving 5K + camera + pointer/drawing for
3 minutes stayed flat (~800 MB) on both builds, as did the sharing encoder path (~270 MB).
It is on the live sharing/controlled path (portal capture, overlays, virtual input) and
needs a monitored live share to pin down.

### Measure latency end to end before tuning

A synthetic 30 fps share through the local LiveKit, with a frame counter drawn into
each frame, showed where the time went. The receiver's jitter buffer, not the
encoder, held frames back (fixed by zero playout delay, which upstream core already
enabled). Fixed-rate ticks added up to 25 ms to remote input and up to 40 ms to
received video, so the engine and UI now wake on arrival. A fresh large share is
limited first by the bandwidth estimate (start small and step up) and then by the
single-threaded H.264 encoder: on an M2 Max it sustains 30 fps at 2560×1080 but not
3840×1620, so the share settles at the largest size that encodes in about 28 ms.
WebRTC never reports a bandwidth limit for screen shares, so use the candidate pair's
available bitrate and the outbound encode time instead.

### Use the installed Hyprland Lua API and give demos enough space

Legacy dispatcher syntax failed on Hyprland 0.56.2. Runtime commands use Lua,
for example `hyprctl dispatch 'hl.dsp.focus({ workspace = "7" })'`.
The two demo windows initially tiled to roughly 384 logical pixels wide among
existing windows; moving only those windows to an empty workspace gave each
about 1581×1291 logical pixels. Test both cramped tiling and generous layouts.

### Tap audio where it is still per person

The playback mix loses who said what, and diarising it again would be slow and
wrong. Every remote track already has its own decode task, so the capture copies
frames there, before mixing; your own voice is taken after echo cancellation, so
speaker bleed is never attributed to you. Speaker names come for free.

### GPU memory is charged to whoever allocates it

On Apple silicon the GPU uses system RAM, and Asahi charges it to the allocating
process's cgroup as shared memory. A loaded large-v3 shows about 100 MB of RSS but
4.6 GB in its scope's `memory.current`. Inside the client's scope (3 GB high, 4 GB
max) it would have throttled and then killed the call. Size limits from the scope's
`memory.peak`, not from RSS, and give memory-hungry helpers a scope of their own.
The same applies to the client: its own windows and video decoding are GPU memory
too, so its limits should be checked against `memory.peak` after a real call.

### On Apple silicon, GPU inference makes the desktop stutter

Hyprland draws every frame, the cursor included, on the same GPU, and the Asahi
driver can't interrupt a running compute job. With large-v3 on the GPU a frame probe
(`tasks/capture-bench/frame_probe.py`) saw 12–17% of frames dropped and stalls of
150–230 ms: the cursor and typing lag. Low queue priority (`VK_KHR_global_priority`)
and ten times smaller submissions, both tried in a patched ggml, changed nothing.
The cause is in the kernel: the Asahi driver pins compute queues to the highest
priority, so the firmware never switches them out for the compositor. Aurora fixed
this for G14X (M2 Pro/Max/Ultra) in `omacom/linux-aurora` PR #68, first released in
2026.10.11.14; older kernels can opt in with `asahi.debug_flags=281474976710656`.
On the CPU with `CPUWeight=idle` the desktop is untouched (dropped frames as when
idle) at 2–3× real time with large-v3-turbo. With the fix (release 57 kernel, built
from Aurora 2026.10.13.18) the same probe with large-v3 on the GPU dropped no frames
while transcribing (worst 19.7 ms), at 3.9× real time, so the GPU is the default and
the CPU a setting for kernels without the fix.

### Batched words can move to the wrong person

When whisper writes nothing for an utterance (a short "mm", a word cut off), DTW
times the next utterance's first words early, into it. With people interleaved in a
request that gave the next person's first word to someone else ("Sam: It" /
"Alex: 's better than…" on a real call; "He" moving 1.3 s back into another
speaker's slot in a test). Assigning tokens one at a time also split words.
`batch.rs` now places whole words and lays each person's utterances out together,
3 s from the next person's. Test with two tracks on one timeline:
`hopp capture transcribe Alice=a.wav Bob=b.wav` (use a separate `XDG_DATA_HOME`;
newer builds migrate the capture database).

### whisper.cpp drops DTW timings when flash attention is on

With both enabled every token's `t_dtw` is -1, with no error. Word timings and flash
attention are mutually exclusive; the helper picks DTW whenever it batches.

### Benchmark the engineering, not the model

The question is what Hopp's choices cost: GPU or CPU, batching, window sizes,
priorities, limits. Accuracy is only measured to make sure a saving doesn't cost
quality (a shorter encoder window and a style prompt both did, and were dropped).

### Check Omarchy's bindings before choosing one

`Super+Shift+C` is Omarchy's calendar. `hyprctl binds -j` lists what is taken;
`Super+Alt+C` sits next to Hopp's `Super+Alt+H`.

### Protocol compatibility requires backend support for correlation

The inspected legacy `call_reject` message does not identify the rejecting
peer or validate the pending call. A delayed decline while switching outgoing
calls cannot be reliably correlated by the client. Accepted sessions ignore
generic declines; complete correlation needs an upstream protocol change.

## Continue development

Use [README.md](README.md) for build/install commands and surfaces, and
[dev/README.md](dev/README.md) for the local stack and two-client launcher.
Credentials belong in private fixture/configuration files, never in command
arguments, screenshots or committed documentation. The local fixture binds
the API to `127.0.0.1:18826`, LiveKit signalling to `18880`, and RTC TCP/UDP
to `18881`/`18882`.

Run the standalone client checks without invoking the upstream interactive
core/Tauri workflows:

```sh
cd linux-client
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --release --locked --workspace
```

Next acceptance priorities:

0. Share a screen between the two fixture clients with the new interface:
   viewer follow, peek (Super+H), sharing pill, control through the crop. The
   clients now log the call lifecycle to stderr (`/tmp/hopp-live-*.log` when
   launched with `dev/run-client.py` as in dev/README.md).
1. Make monitor/window capture and clean stopping reproducible; exercise
   chooser cancel, late acceptance and leave/rejoin with real portal sessions.
2. Verify clipboard copy/cut/paste through two controlled clients and configure
   and test the desktop-wide revoke shortcut. `make install` now binds
   Super+Ctrl+Escape to `--revoke-control` in `~/.config/hypr/hopp.lua`.
3. Check live scaling, rotations, hotplug, tiled/floating layouts, focus changes
   and lock/reconnect behavior with ongoing capture and control.
4. Exercise real microphone/speaker acoustics, device removal, long calls,
   network disruption and existing Mac/Windows clients.

IME composition remains unsupported. Review it alongside the window-control
and backend-correlation limits before describing the client as feature complete.
