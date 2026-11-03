# Native desktop acceptance helpers

These standalone helpers exercise real GPUI text editing and the production
virtual-input implementation. They run only inside the device-isolated desktop
described in [../nested-hyprland-check](../nested-hyprland-check/README.md): a
private runtime under `/tmp`, hidden `/dev/input` and hidden `/run/user` are
mandatory. They cannot run on the normal user desktop.

```sh
cargo build --locked --manifest-path tasks/desktop-check/Cargo.toml
```

`hopp-native-editor` opens a real native input field, starts with synthetic
text, reports its current value to `$XDG_RUNTIME_DIR/hopp-editor.txt` and exits
after five minutes. This makes actual remote paste, copy and cut observable.

`hopp-private-input` accepts input only while the isolated editor or the real
Hyprland sharing chooser has focus and the test compositor is unlocked. It
uses the production virtual keyboard/pointer module and releases held keys
and buttons on exit. Coordinates are relative to the focused test window.

```sh
hopp-private-input hyprland-share-picker click 0.5 0.4
hopp-private-input hopp-native-editor key a ctrl
```

The ignored `real_engine_portal_share_control_drawing_and_cleanup` test uses
two real engine workers and Hopp-issued media tokens. Set
`HOPP_DESKTOP_TEST=monitor` or `window`, a private fixture and a private
configuration directory. `HOPP_CLIPBOARD_TEST=1` enables the editor stages;
its focus and physical-device isolation guards run before each injected
operation. The normal test suite never opens a chooser or sends desktop input.
