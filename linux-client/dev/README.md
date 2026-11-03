# Local Hopp integration fixture

`local-stack.py` starts the published Hopp backend, PostgreSQL, Redis and LiveKit
inside a separate rootless Docker daemon. It downloads the rootless helper tools
into the fixture directory; it does not install packages or change the system
Docker service. Docker/dockerd, Docker Compose, newuidmap/newgidmap and enabled
user namespaces with `/etc/subuid` and `/etc/subgid` mappings are prerequisites.
Native ARM64 and x86_64 hosts are supported.

From `linux-client`:

```sh
python dev/local-stack.py start
python dev/local-stack.py seed
python dev/local-stack.py status
python dev/local-stack.py stop
```

The default private directory is `/tmp/hopp-local-<uid>`. Use
`--state-dir /tmp/hopp-local-your-test` on each command to choose another fixture.
`start` can be rerun; `stop` retains its PostgreSQL volume and private files.

The Hopp API listens on `http://127.0.0.1:18826`; LiveKit signalling uses
`ws://127.0.0.1:18880`, with RTC TCP `18881` and UDP `18882`. RootlessKit publishes
these ports on localhost only. PostgreSQL and Redis remain inside the namespace.
The backend and LiveKit share that namespace so backend presence queries can reach
the same LiveKit address advertised to native clients.

`seed` creates two local `.invalid` accounts on one team and the
“Omarchy integration” room through Hopp's real signup, invitation and room APIs.
No email, telemetry, billing or messaging credentials are configured. It writes:

- `client-test.json`: API base, room ID, and publisher/viewer API JWTs and user IDs.
- `room-tokens.json`: LiveKit URL, room ID, and both users' `audioToken`,
  `videoToken`, `cameraToken`, and `participant` fields.
- `secrets.json`: credentials for this local backend, PostgreSQL and LiveKit.

The directory is mode `0700`; credential files are mode `0600`. The helper prints
file paths rather than tokens. Native integration tests can read those files
directly, keeping credentials out of CLI arguments. Calling `seed` again refreshes
the 24-hour media tokens. Fetching room tokens marks the test users in the room;
tests should close their LiveKit connections and call Hopp's room-leave endpoint.

This fixture uses published container images, not a build of the local backend
checkout. The desktop implementation itself uses Hopp's existing HTTP and media
protocols without backend modifications.

## Open both native clients

Build the binary, then start publisher and viewer in separate terminals. Each
uses a fresh private configuration directory and a separate instance socket;
the normal app’s saved account is unchanged. Tokens stay in private files.

```sh
python dev/run-client.py --fixture /path/to/client-test.json --peer publisher --binary /path/to/hopp
python dev/run-client.py --fixture /path/to/client-test.json --peer viewer --binary /path/to/hopp
```

Join “Omarchy integration” in both windows. Share a monitor or a window through
the real desktop chooser, then grant the other participant control if sharing
a monitor. Cameras and microphones start off.

Or start both in the background at once (logs in `/tmp/hopp-live-*.log`):

```sh
dev/run-both.sh /path/to/client-test.json
```

## Live call through the bar plugin

With both clients running, this places a call from the publisher to the viewer
through the Omarchy plugin’s `backend.py`, accepts it, checks both state
snapshots and leaves. It never opens a microphone, camera or screen share:

```sh
python dev/bridge-call-check.py
```

## Complete worker lifecycle check

```sh
mkdir -m 700 /tmp/hopp-engine-check
XDG_CONFIG_HOME=/tmp/hopp-engine-check \
HOPP_API_TEST_FIXTURE=/path/to/client-test.json \
  cargo test real_engine_join_cancel_and_presence_cleanup -- --ignored --nocapture
```

This authenticates two real engine workers, joins/leaves the shared room,
cancels a join, reconnects, checks old events cannot end the new session,
checks locked sharing is rejected, and verifies backend room presence clears.
It opens no microphone, camera, portal chooser or virtual input devices.
