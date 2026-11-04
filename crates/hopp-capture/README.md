# hopp-capture

A transcript of a call, one track per person, saved only on this computer. It started in
Hopp for Linux (`linux-client/src/capture/`) and this crate is what the Mac app uses. Nothing
here knows about LiveKit or either client's windows: a client feeds `Tap` one track per person.

`crates/hopp-transcribe` builds the helper that does the transcribing, in a process of its
own, from the Linux client's source: both clients run one program.

## What differs per client

- **Sample rate.** 16 kHz by default, which is what the Mac streams. Hopp for Linux streams
  48 kHz: turn on the `rate-48k` feature.
- **Keeping the helper apart.** `helper::set_scope` lets a client wrap it, as Linux does with
  a systemd scope. Without one it runs at low priority (`taskpolicy -b` on the Mac, `nice` elsewhere).
- **Where things go.** `paths`: `~/Library/Application Support/com.hopp.app` on the Mac, the XDG
  directories elsewhere, or `HOPP_CAPTURE_HOME` for tests.
- **Capturing every call.** `Config::auto` is on by default on Linux and off on the Mac.

## Moving Hopp for Linux onto this crate

This is a copy for now: Linux can't be compiled on a Mac and CI doesn't build it, so nobody
could check a switch made from there. To switch, on Linux:

1. Depend on it with `hopp-capture = { path = "../crates/hopp-capture", features = ["rate-48k"] }`.
2. Call `helper::set_scope` with the systemd scope, and send `hopp_core::capture` logs to the
   Linux log.
3. Re-export it as `capture`, and delete the modules it now has. Modules not moved yet stay
   where they are until the Mac needs them.

Until then a fix in one copy has to be made in the other.

## Testing

```sh
cargo test                     # from core/: cargo test -p hopp-capture
cargo test --features rate-48k # from this directory
```

`hopp capture transcribe Alex=a.wav Sam=b.wav` runs two files through the real recorder and helper.
Always give it a throwaway `HOPP_CAPTURE_HOME`, and `HOPP_TRANSCRIBE_HELPER` if the helper
isn't beside it. `a_call_goes_from_its_tracks_to_a_transcript` in `core/src/livekit/call_capture.rs`
does the same through the Mac's taps, in real time.

`bench/live_text.py` times the live transcript. On an Apple M5 at background priority, with two
synthetic voices over 95 s: first words after a median of 1.86 s (Linux 2.1 s), final text after
0.78 s (Linux 0.56 s), transcriber busy for 13% of the audio (Linux 26%), and no word taken back.

`moments` keeps numbered stills of the shared screen, with everyone's marks drawn on them, in
`capture/frames/<id>/`: `frame-NNN.webp` (at most 1440 px), `frame-NNN.full.webp` and
`frame-NNN.json`. `a_scripted_call_keeps_every_mark_on_a_frame` in `core/src/livekit/call_capture.rs`
runs that through the real recorder and helper, and `what_a_still_costs` and `what_a_frame` measure
it. On an Apple M5: 1.46 µs per video frame at worst when not capturing; a 5120x2880 frame costs
the video thread a median of 680 µs (worst 3.8 ms, debug build) when a still is wanted; a 2560x1440
still takes 182 ms on its own thread (release).
