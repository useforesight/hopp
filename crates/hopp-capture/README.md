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
