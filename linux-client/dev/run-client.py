#!/usr/bin/env python3
"""Run either local fixture account without putting credentials in arguments."""
import argparse
import json
import os
from pathlib import Path
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--peer", choices=["publisher", "viewer"], required=True)
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    fixture = json.loads(args.fixture.read_text())
    binary = args.binary.resolve(strict=True)
    config = Path(tempfile.mkdtemp(prefix=f"hopp-{args.peer}-config-"))
    directory = config / "hopp"
    directory.mkdir(mode=0o700)
    settings = directory / "settings.json"
    fd = os.open(settings, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as file:
        json.dump({"server": fixture["api_base"], "token": fixture[args.peer]["token"]}, file)
    environment = os.environ.copy()
    environment["XDG_CONFIG_HOME"] = str(config)
    os.execve(binary, [str(binary), "--instance", f"local-{args.peer}"], environment)


if __name__ == "__main__":
    main()
