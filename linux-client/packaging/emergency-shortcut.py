#!/usr/bin/env python3
"""Print an Omarchy binding; desktop configuration is never changed here."""

import argparse
import json
import re
import shlex
from pathlib import Path


def binding(prefix, instance):
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,32}", instance):
        raise ValueError("Instance name needs 1–32 letters, digits, hyphens or underscores")
    binary = str(prefix.expanduser().resolve() / "bin/hopp")
    if any(ord(character) < 32 or ord(character) == 127 for character in binary):
        raise ValueError("Install prefix cannot contain control characters")
    command = [binary, "revoke"]
    if instance != "main":
        command.extend(["--instance", instance])
    # The Omarchy helper receives a shell command inside a Lua string. Quote
    # argv for the shell first, then the whole command for Lua independently.
    return 'o.bind("SUPER + CTRL + Escape", "Revoke Hopp remote control", ' + json.dumps(shlex.join(command), ensure_ascii=False) + ')'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prefix", type=Path, default=Path.home() / ".local")
    parser.add_argument("--instance", default="main")
    args = parser.parse_args()
    try:
        print(binding(args.prefix, args.instance))
    except ValueError as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
