#!/usr/bin/env python3
"""Isolated rootless Hopp + LiveKit fixture; never changes the system Docker service."""

import argparse
import json
import os
import platform
import secrets
import signal
import subprocess
import tarfile
import time
import urllib.error
import urllib.request
from pathlib import Path

API = "http://127.0.0.1:18826"
LIVEKIT = "ws://127.0.0.1:18880"
PORTS = ["127.0.0.1:18826:18826/tcp", "127.0.0.1:18880:18880/tcp",
         "127.0.0.1:18881:18881/tcp", "127.0.0.1:18882:18882/udp"]


def private_write(path, value):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(descriptor, "w") as output:
        output.write(value)
    os.chmod(path, 0o600)


def docker(state, *args, check=True, capture=False):
    return subprocess.run(["docker", "--host", "unix://" + str(state / "run/docker.sock"),
                           *args], check=check, capture_output=capture, text=True)


def compose(state, *args, **kwargs):
    return docker(state, "compose", "--project-directory", str(state),
                  "-f", str(state / "compose.json"), *args, **kwargs)


def download_tools(state):
    machine = platform.machine()
    if machine not in ("aarch64", "x86_64"):
        raise RuntimeError("Local fixture currently supports ARM64 and x86_64")
    binary = state / "bin"
    binary.mkdir(exist_ok=True)
    if not (binary / "dockerd-rootless.sh").exists():
        archive = state / "rootless-extras.tgz"
        urllib.request.urlretrieve(
            f"https://download.docker.com/linux/static/stable/{machine}/docker-rootless-extras-29.0.0.tgz", archive)
        with tarfile.open(archive) as package:
            for entry in package.getmembers():
                if entry.isfile():
                    destination = binary / Path(entry.name).name
                    destination.write_bytes(package.extractfile(entry).read())
                    destination.chmod(0o755)
    if not (binary / "slirp4netns").exists():
        urllib.request.urlretrieve(
            f"https://github.com/rootless-containers/slirp4netns/releases/download/v1.3.6/slirp4netns-{machine}",
            binary / "slirp4netns")
        (binary / "slirp4netns").chmod(0o755)
    if not (binary / "rootlessctl").exists():
        archive = state / "rootlesskit.tgz"
        urllib.request.urlretrieve(
            f"https://github.com/rootless-containers/rootlesskit/releases/download/v3.2.0/rootlesskit-{machine}.tar.gz", archive)
        with tarfile.open(archive) as package:
            for entry in package.getmembers():
                if entry.isfile() and Path(entry.name).name == "rootlessctl":
                    (binary / "rootlessctl").write_bytes(package.extractfile(entry).read())
                    (binary / "rootlessctl").chmod(0o755)


def configure(state):
    if not (state / "secrets.json").exists():
        credentials = {"SESSION_SECRET": secrets.token_urlsafe(32),
                       "POSTGRES_PASSWORD": secrets.token_urlsafe(24),
                       "LIVEKIT_API_KEY": "hopp-native-local",
                       "LIVEKIT_API_SECRET": secrets.token_urlsafe(32)}
        private_write(state / "secrets.json", json.dumps(credentials))
    credentials = json.loads((state / "secrets.json").read_text())
    private_write(state / ".env", "\n".join(f"{key}={value}" for key, value in credentials.items()) + "\n")
    private_write(state / "livekit.yaml", """port: 18880
bind_addresses: [127.0.0.1]
log_level: info
rtc:
  tcp_port: 18881
  udp_port: 18882
  node_ip: 127.0.0.1
  use_external_ip: false
  enable_loopback_candidate: true
keys:
""" + f"  {credentials['LIVEKIT_API_KEY']}: {credentials['LIVEKIT_API_SECRET']}\n")
    common = {"network_mode": "host"}
    config = {"name": "hopp-native-local", "services": {
        "db": dict(common, image="postgres:16-alpine", environment={
            "POSTGRES_USER": "hopp", "POSTGRES_PASSWORD": "${POSTGRES_PASSWORD}", "POSTGRES_DB": "hopp"},
            command=["postgres", "-c", "listen_addresses=127.0.0.1", "-p", "15432"],
            volumes=["db_data:/var/lib/postgresql/data"]),
        "cache": dict(common, image="redis:7-alpine",
            command=["redis-server", "--bind", "127.0.0.1", "--port", "16379"]),
        "livekit": dict(common, image="livekit/livekit-server:v1.11",
            command=["--config", "/etc/livekit.yaml"],
            volumes=[str(state / "livekit.yaml") + ":/etc/livekit.yaml:ro"]),
        "backend": dict(common, image="iparaskev/hopp-backend:latest", environment={
            "SERVER_HOST": "127.0.0.1", "SERVER_PORT": "18826", "DEPLOY_DOMAIN": "127.0.0.1:18826",
            "USE_TLS": "false", "SESSION_SECRET": "${SESSION_SECRET}",
            "LIVEKIT_API_KEY": "${LIVEKIT_API_KEY}", "LIVEKIT_API_SECRET": "${LIVEKIT_API_SECRET}",
            "LIVEKIT_SERVER_URL": LIVEKIT,
            "DATABASE_DSN": "postgres://hopp:${POSTGRES_PASSWORD}@127.0.0.1:15432/hopp?sslmode=disable",
            "REDIS_URI": "redis://127.0.0.1:16379/0?protocol=3"})}, "volumes": {"db_data": {}}}
    private_write(state / "compose.json", json.dumps(config, indent=2))


def wait_until(check, description, seconds=45):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if check():
            return
        time.sleep(0.25)
    raise RuntimeError(f"Timed out waiting for {description}")


def api_ready():
    try:
        urllib.request.urlopen(API + "/api/auth/user", timeout=2)
    except urllib.error.HTTPError as error:
        return error.code == 401
    except (OSError, TimeoutError):
        return False
    return True


def start(state):
    state.mkdir(mode=0o700, parents=True, exist_ok=True)
    state.chmod(0o700)
    (state / "run").mkdir(mode=0o700, exist_ok=True)
    download_tools(state)
    configure(state)
    if docker(state, "info", check=False, capture=True).returncode:
        environment = os.environ.copy()
        environment.update(PATH=str(state / "bin") + ":" + environment["PATH"],
                           XDG_RUNTIME_DIR=str(state / "run"), DOCKERD_ROOTLESS_ROOTLESSKIT_NET="slirp4netns")
        log_descriptor = os.open(state / "daemon.log", os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(log_descriptor, "w") as output:
            process = subprocess.Popen([str(state / "bin/dockerd-rootless.sh"), "--storage-driver=vfs",
                "--data-root=" + str(state / "docker-data"), "--exec-root=" + str(state / "run/exec"),
                "--pidfile=" + str(state / "run/dockerd.pid"), "--host=unix://" + str(state / "run/docker.sock")],
                env=environment, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
        private_write(state / "daemon.pid", str(process.pid))
        wait_until(lambda: docker(state, "info", check=False, capture=True).returncode == 0, "rootless Docker")
    control = [str(state / "bin/rootlessctl"), "--socket", str(state / "run/dockerd-rootless/api.sock")]
    existing = subprocess.run(control + ["list-ports"], check=True, capture_output=True, text=True).stdout
    for port in PORTS:
        if port.split(":")[1] not in existing:
            subprocess.run(control + ["add-ports", port], check=True, stdout=subprocess.DEVNULL)
    compose(state, "up", "-d", "db", "cache", "livekit")
    wait_until(lambda: compose(state, "exec", "-T", "db", "pg_isready", "-p", "15432", "-U", "hopp",
                              check=False, capture=True).returncode == 0, "PostgreSQL")
    compose(state, "up", "-d", "backend")
    wait_until(api_ready, "Hopp API")
    print(f"Hopp API: {API}\nLiveKit: {LIVEKIT}\nPrivate state: {state}")


def request(method, path, token=None, body=None):
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = "Bearer " + token
    payload = None if body is None else json.dumps(body).encode()
    try:
        with urllib.request.urlopen(urllib.request.Request(API + path, payload, headers, method=method), timeout=15) as response:
            data = response.read()
            return json.loads(data) if data else None
    except urllib.error.HTTPError as error:
        raise RuntimeError(f"Local Hopp {method} {path} returned HTTP {error.code}") from None


def seed(state):
    fixture_path = state / "client-test.json"
    if fixture_path.exists():
        fixture = json.loads(fixture_path.read_text())
    else:
        publisher = request("POST", "/api/sign-up", body={"first_name": os.environ.get("HOPP_SEED_PUBLISHER", "Publisher"), "last_name": "Local",
            "email": "publisher@hopp-native.invalid", "password": secrets.token_urlsafe(24), "team_name": "Native client test"})
        invite = request("GET", "/api/auth/get-invite-uuid", publisher["token"])
        viewer = request("POST", "/api/sign-up", body={"first_name": os.environ.get("HOPP_SEED_VIEWER", "Viewer"), "last_name": "Local",
            "email": "viewer@hopp-native.invalid", "password": secrets.token_urlsafe(24), "team_invite_uuid": invite["invite_uuid"]})
        room = request("POST", "/api/auth/room", publisher["token"], {"name": "Omarchy integration"})
        fixture = {"api_base": API, "livekit_url": LIVEKIT, "room_id": room["id"],
                   "publisher": publisher, "viewer": viewer}
        private_write(fixture_path, json.dumps(fixture, indent=2))
    tokens = {"livekit_url": LIVEKIT, "room_id": fixture["room_id"]}
    for role in ("publisher", "viewer"):
        token = fixture[role]["token"]
        user = request("GET", "/api/auth/user", token)
        fixture[role]["user_id"] = user["id"]
        tokens[role] = request("GET", "/api/auth/room/" + fixture["room_id"], token)
    private_write(fixture_path, json.dumps(fixture, indent=2))
    private_write(state / "room-tokens.json", json.dumps(tokens, indent=2))
    print(f"Private API fixture: {fixture_path}\nPrivate media tokens: {state / 'room-tokens.json'}")


def stop(state):
    compose(state, "down", check=False)
    pidfile = state / "daemon.pid"
    if pidfile.exists():
        pid = int(pidfile.read_text())
        # Verify this process still belongs to our isolated daemon before signalling.
        try:
            command = Path(f"/proc/{pid}/cmdline").read_bytes()
            if str(state).encode() in command and b"rootlesskit" in command:
                os.killpg(pid, signal.SIGTERM)
        except (FileNotFoundError, ProcessLookupError):
            pass
    print(f"Stopped local stack; private data retained in {state}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("start", "seed", "status", "stop"))
    parser.add_argument("--state-dir", type=Path, default=Path(f"/tmp/hopp-local-{os.getuid()}"))
    arguments = parser.parse_args()
    state = arguments.state_dir.resolve()
    try:
        if arguments.command == "start":
            start(state)
        elif arguments.command == "seed":
            seed(state)
        elif arguments.command == "stop":
            stop(state)
        else:
            compose(state, "ps")
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        raise SystemExit(str(error)) from None


if __name__ == "__main__":
    main()
