#!/usr/bin/env python3
"""Drive a real call between the two fixture clients through the bar plugin bridge.

Start both clients first (see README.md), then run this. It calls the viewer
from the publisher, accepts, checks both snapshots, and leaves. It never
toggles microphones, cameras or screen sharing."""
import json
from pathlib import Path
import queue
import subprocess
import sys
import threading
import time

BACKEND = str(Path(__file__).resolve().parent.parent / "omarchy-plugin/local.hopp/backend.py")


class Peer:
    def __init__(self, instance):
        self.name = instance
        self.process = subprocess.Popen(
            [sys.executable, BACKEND, "--instance", instance, "--no-autostart"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
        self.messages = queue.Queue()
        self.state = None
        threading.Thread(target=self.read, daemon=True).start()

    def read(self):
        for line in self.process.stdout:
            self.messages.put(json.loads(line))

    def send(self, action, value=""):
        self.process.stdin.write(json.dumps({"action": action, "value": value}) + "\n")
        self.process.stdin.flush()

    def wait(self, predicate, what, timeout=25):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                message = self.messages.get(timeout=0.5)
            except queue.Empty:
                continue
            if message["type"] == "state":
                self.state = message["state"]
                if predicate(self.state):
                    return self.state
            elif message["type"] != "offline":
                print(f"  {self.name}: {message}")
        raise SystemExit(f"FAIL {self.name}: timed out waiting for {what}")


def summary(state):
    call = state.get("call")
    if not call:
        return f"no call · ringing={state.get('ringing')} dialing={state.get('dialing')}"
    return (f"call in {call['room']!r} connected={call['connected']} "
            f"participants={[p['name'] for p in call['participants']]} mic={call['microphone']}")


publisher, viewer = Peer("local-publisher"), Peer("local-viewer")
p = publisher.wait(lambda s: s["signed_in"] and s["teammates"], "sign-in + teammates")
v = viewer.wait(lambda s: s["signed_in"], "sign-in")
print(f"publisher signed in as {p['user']['name']}; sees {len(p['teammates'])} teammate(s)")
target = next(t for t in p["teammates"] if t["id"] == v["user"]["id"])
print(f"publisher → call {target['name']} (online={target['online']})")
publisher.send("call", target["id"])
publisher.wait(lambda s: s.get("dialing"), "dialing")
print("publisher: dialing")
ringing = viewer.wait(lambda s: s.get("ringing"), "incoming call")["ringing"]
print(f"viewer: ringing from {ringing['name']} ({ringing['kind']})")
viewer.send("accept")
print("viewer → accept")
v = viewer.wait(lambda s: s.get("call") and s["call"]["connected"] and s["call"]["participants"], "connected call")
p = publisher.wait(lambda s: s.get("call") and s["call"]["connected"] and s["call"]["participants"], "connected call")
print("viewer:   ", summary(v))
print("publisher:", summary(p))
if "--stay" not in sys.argv:
    publisher.send("leave")
    viewer.send("leave")
    publisher.wait(lambda s: not s.get("call"), "left")
    viewer.wait(lambda s: not s.get("call"), "left")
    print("both left; call cleared")
print("PASS")
