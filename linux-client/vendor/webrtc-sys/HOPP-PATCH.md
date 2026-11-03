# Hopp Linux PipeWire symbol isolation

Upstream: LiveKit rust-sdks commit `2dd762da4fdc8b73504983aff078d7824ea5d4ee`, crate `webrtc-sys` 0.3.47. Original source and Apache 2.0 license are retained. The build utility remains pinned to that same Git revision.

Changes: workspace-inherited manifest fields are explicit for this vendored crate; on Linux only, `build.rs` copies the downloaded WebRTC archive to this crate's private `OUT_DIR`, derives all defined global `pw_*` symbols with `llvm-nm`, and renames their definitions and archive-internal references to `hopp_webrtc_pw_*` with `llvm-objcopy`. Only the private archive is linked. Original `dlsym("pw_...")` strings and C++ loader initialization remain unchanged. Other platforms use the upstream archive directly.

Reason: WebRTC's optional PipeWire loader supplies weak `pw_*` stubs. The static linker otherwise binds native Rust PipeWire calls to these uninitialized stubs, causing screen capture to crash at `pw_init`. Namespacing keeps WebRTC's internal loader separate and lets native capture use the system PipeWire DSO. Hiding symbols or wrapping only `pw_init` does not fix the rest of the API collisions.

Linux builds require LLVM's `llvm-nm` and `llvm-objcopy`; optional `HOPP_LLVM_NM` and `HOPP_LLVM_OBJCOPY` select their executable paths. The shared SDK cache is never changed. The map is ordered and both absence of original definitions and presence of renamed definitions are checked after rewriting.
