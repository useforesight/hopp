#!/usr/bin/env python3
"""How quickly the live transcript keeps up, in real time, with no call or window.

    HOPP_CAPTURE_HOME=/tmp/throwaway live_text.py path/to/hopp Alex=a.wav Sam=b.wav

Plays the tracks through `hopp capture transcribe --realtime` and times
`hopp capture follow --drafts --json`. Start the tracks with a few seconds of silence, so the
follower is attached before anyone speaks.

- First words: from a line's start until any of its words show.
- Final text: from the end of its speech until its final text shows.
- Busy: the transcriber's time as a share of the audio.

A line starts up to 0.2 s before the speech (the recorder keeps what came just before it),
and `follow` polls every 0.3 s, so both latencies read a little high.
"""

import json
import os
import statistics
import subprocess
import sys
import threading
import time


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    if not os.path.isabs(os.environ.get("HOPP_CAPTURE_HOME", "")):
        sys.exit("Set HOPP_CAPTURE_HOME to a throwaway directory: this writes captures.")
    hopp, tracks = sys.argv[1], sys.argv[2:]

    follow = subprocess.Popen(
        [hopp, "capture", "follow", "current", "--drafts", "--json"],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    changes = []

    def read():
        for line in follow.stdout:
            changes.append((time.time(), json.loads(line)))

    reader = threading.Thread(target=read)
    reader.start()

    played = subprocess.run(
        [hopp, "capture", "transcribe", "--realtime", *tracks],
        capture_output=True,
        text=True,
    )
    if played.returncode != 0:
        follow.kill()
        sys.exit(played.stderr)
    # The follower sees the capture end on its own, within a poll.
    reader.join(timeout=10)
    follow.kill()

    calls = subprocess.run(
        [hopp, "capture", "list", "--json", "--limit", "1"],
        capture_output=True,
        text=True,
        check=True,
    )
    started_ms = json.loads(calls.stdout)[0]["started_ms"]

    first_words, final_text, taken_back, shown, versions = [], [], 0, {}, 0
    finals = set()
    for arrived, line in changes:
        id, kind = line["id"], line["kind"]
        if kind not in ("draft", "speech"):
            continue
        # A line appears as soon as someone starts speaking, before it has any words.
        if not shown.get(id) and line["text"].strip():
            first_words.append(arrived * 1000 - (started_ms + line["t_ms"]))
        if kind == "draft":
            versions += 1
            if not line["text"].startswith(shown.get(id, "")):
                taken_back += 1
            shown[id] = line["text"]
        else:
            shown[id] = shown.get(id) or line["text"]
            finals.add(id)
            if line.get("end_ms") is not None:
                final_text.append(arrived * 1000 - (started_ms + line["end_ms"]))

    print(played.stdout)
    print(played.stderr.strip())
    print()
    print(f"{len(finals)} lines, {versions} draft versions, {taken_back} taking words back")
    for name, values in (("First words", first_words), ("Final text", final_text)):
        if values:
            seconds = sorted(value / 1000 for value in values)
            print(
                f"{name}: median {statistics.median(seconds):.2f} s, "
                f"worst {seconds[-1]:.2f} s, best {seconds[0]:.2f} s"
            )


if __name__ == "__main__":
    main()
