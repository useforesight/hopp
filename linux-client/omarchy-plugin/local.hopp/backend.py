#!/usr/bin/env python3
"""Bridge between the Omarchy bar plugin and the Hopp client.

Speaks the client's private instance socket (length-prefixed JSON, one status
byte per request) and turns it into JSON lines on stdout for QML:

  {"type": "state", "state": {...}}   whenever the client's state changes
  {"type": "offline", "stopped": bool} while no client is reachable; stopped
                                       after the user quit, until they start it
  {"type": "error", "message": "..."}  when a command could not be delivered

Commands arrive on stdin as {"action": "...", "value": "..."} lines.
"""
import argparse
import json
import os
import shutil
import socket
import stat
import struct
import subprocess
import sys
import threading
import time

# Actions the panel may send, mapped to the client's request names. Anything
# else is rejected here, before it reaches the socket.
SIMPLE = {
    "open": "open",
    "settings": "settings",
    "peek": "peek",
    "faces": "show_faces",
    "viewer": "show_viewer",
    "accept": "accept",
    "decline": "decline",
    "clear_missed": "clear_missed",
    "leave": "leave",
    "microphone": "microphone",
    "camera": "camera",
    "share": "share",
    "capture": "capture",
    "captures": "captures",
    "draw": "draw",
    "quit": "quit",
}
WITH_ID = {
    "call": "call",
    "join": "join_room",
    "dismiss_missed": "dismiss_missed",
    "status": "set_status",
}

output_lock = threading.Lock()


def emit(message):
    with output_lock:
        sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
        sys.stdout.flush()


def socket_path(instance):
    runtime = os.environ.get("XDG_RUNTIME_DIR")
    directory = os.path.join(runtime, "hopp") if runtime else f"/tmp/hopp-{os.getuid()}"
    return os.path.join(directory, f"{instance}.sock")


def connect(path):
    """Connect only to a private socket owned by this user."""
    info = os.lstat(path)
    if not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise PermissionError("Hopp socket is not private to this user")
    client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    client.settimeout(3)
    client.connect(path)
    return client


def read_exact(client, size):
    data = b""
    while len(data) < size:
        chunk = client.recv(size - len(data))
        if not chunk:
            raise ConnectionError("Hopp closed the connection")
        data += chunk
    return data


def request(client, action):
    body = json.dumps({"link": None, "action": action}).encode()
    client.sendall(struct.pack(">I", len(body)) + body)
    if read_exact(client, 1) != b"\x00":
        raise RuntimeError("Hopp rejected the request")


class Bridge:
    def __init__(self, instance, binary, autostart):
        self.path = socket_path(instance)
        self.instance = instance
        self.binary = binary
        self.autostart = autostart
        self.last_start = 0.0
        # Set when the user quits Hopp, so we don't start it straight back up.
        # Cleared by "open" or by the client coming back some other way.
        self.stopped = False
        self.client = None

    def start_client(self):
        # Reap a client we started that has since exited, so it doesn't linger as a zombie.
        if self.client is not None and self.client.poll() is not None:
            self.client = None
        # Launch at most once every 20 s, detached so it outlives the shell.
        if self.stopped or not self.autostart or not self.binary or time.monotonic() - self.last_start < 20:
            return
        self.last_start = time.monotonic()
        arguments = [self.binary, "--background"]
        if self.instance != "main":
            arguments += ["--instance", self.instance]
        self.client = subprocess.Popen(arguments, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                       stderr=subprocess.DEVNULL, start_new_session=True)

    def subscribe_forever(self):
        delay = 0.5
        while True:
            try:
                client = connect(self.path)
                request(client, "subscribe")
                client.settimeout(None)
                self.stopped = False
                delay = 0.5
                while True:
                    (length,) = struct.unpack(">I", read_exact(client, 4))
                    if length > 4 * 1024 * 1024:
                        raise ValueError("Oversized Hopp state")
                    emit({"type": "state", "state": json.loads(read_exact(client, length))})
            except (OSError, ConnectionError, RuntimeError, ValueError):
                emit({"type": "offline", "stopped": self.stopped})
                self.start_client()
                time.sleep(delay)
                delay = min(delay * 2, 5)

    def command(self, action, value):
        if action == "captures":
            # Carries an optional call id: {"captures": null} opens the window as it was.
            payload = {"ui": {"captures": None}}
        elif action in SIMPLE:
            payload = {"ui": SIMPLE[action]}
        elif action in WITH_ID and isinstance(value, str) and 0 < len(value) <= 128:
            payload = {"ui": {WITH_ID[action]: value}}
        else:
            emit({"type": "error", "message": f"Unknown Hopp action: {action}"})
            return
        if action == "open":
            self.stopped = False
        elif action == "quit":
            # Before sending: the client may drop the state stream before it acks.
            self.stopped = True
        try:
            client = connect(self.path)
            with client:
                request(client, payload)
        except (OSError, ConnectionError, RuntimeError) as error:
            if action == "open":
                self.last_start = 0.0
                self.start_client()
            elif action == "quit":
                emit({"type": "offline", "stopped": True})
            else:
                emit({"type": "error", "message": f"Hopp isn't running ({error})"})


def find_binary(explicit):
    if explicit:
        return explicit
    # Prefer the user's own install: upstream Hopp's Linux package also ships a `hopp`.
    local = os.path.expanduser("~/.local/bin/hopp")
    if os.access(local, os.X_OK):
        return local
    return shutil.which("hopp")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--instance", default=os.environ.get("HOPP_INSTANCE", "main"))
    parser.add_argument("--binary")
    parser.add_argument("--no-autostart", action="store_true")
    arguments = parser.parse_args()
    bridge = Bridge(arguments.instance, find_binary(arguments.binary), not arguments.no_autostart)
    threading.Thread(target=bridge.subscribe_forever, daemon=True).start()
    for line in sys.stdin:
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(message, dict):
            bridge.command(message.get("action"), message.get("value"))


if __name__ == "__main__":
    main()
