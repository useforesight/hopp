# Native UI visual fixture

This standalone harness renders the real GPUI UI (`src/ui/`, included directly)
with synthetic users, rooms, participants, cameras and screen frames. It never
authenticates, opens a portal, transmits media or injects input. Its engine is a
contract stub (`src/lib.rs`), so the main client build remains the integration
check; the request type and frame code are shared with the client, so they
cannot drift.

Each scene opens one kind of window (`ui::Options::only`) in a headless Weston
whose output is exactly that window’s size:

```sh
CARGO_TARGET_DIR=/tmp/hopp-visual-target ./linux-client/tasks/visual-check/run.sh
```

The script extracts the Arch Weston package into a temporary directory without
installing it. Weston uses a private runtime directory, a headless pixman output
and a fake seat; screenshots use only that private Wayland socket. Scoped child
processes and temporary files are cleaned up on exit. Output goes to
`tasks/evidence/ui/`.

Run one scene by hand, for example:

```sh
cargo run --manifest-path linux-client/tasks/visual-check/Cargo.toml -- --scene viewer --people 3
cargo run --manifest-path linux-client/tasks/visual-check/Cargo.toml -- --scene faces --people 4 --layout row --size small
```

Scenes: `signin`, `home`, `home-call`, `settings`, `faces`, `faces-speaking`, `faces-silent`, `viewer`, `peek`, `captures` (reads this computer's real captures, so `run.sh` doesn't save it),
`sharing`, `call`.

```sh
cargo clippy --manifest-path linux-client/tasks/visual-check/Cargo.toml --all-targets -- -D warnings
```
