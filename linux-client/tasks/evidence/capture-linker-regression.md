# PipeWire / native WebRTC symbol collision

The first actual monitor capture test on 2026-10-06 crashed after the real
Hyprland portal chooser accepted the source. The extracted core was diagnosed
with GDB and deleted immediately afterward. No user-desktop frames or devices
were involved.

The symbolized stack was `capture::stream_frames` → `pipewire::init` → address
zero. `libpipewire-0.3.so.0` was correctly loaded. `readelf` nevertheless showed
the executable defining its own local `pw_init`; the native WebRTC archive
provided weak `pw_*` forwarding functions backed by private, uninitialized
function pointers. Static symbol resolution chose those wrappers instead of
the actual system library. This also affects functions beyond initialization.

The build correction must rename both the definitions and internal references
of WebRTC's `pw_*` symbols in a private copy of its static archive. The real
`pw_*` symbols then resolve to the system PipeWire library. Runtime `dlsym`
names remain unchanged. The shared SDK cache and installed desktop libraries
must never be modified.

The default regression test
`system_pipewire_initialization_and_loop_do_not_call_webrtc_lazy_stubs`
initializes the actual library and creates/destroys a main loop. It opens no
portal, display, daemon connection, camera or microphone. Actual portal-frame
delivery and stop are tested separately after the corrected link.
