"""Measure desktop frame pacing while hopp-whisper loads and transcribes.

A small GTK window animates continuously and records the interval between the
compositor's frames. Meanwhile the helper is started in a scope of its own, as Hopp
does, then given full 28 s windows to transcribe. Each slow frame is reported against
what the helper was doing at the time.

usage: frame_probe.py HELPER MODEL DATA_DIR [helper flags…]
"""
import json, os, struct, subprocess, sys, threading, time, wave

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import GLib, Gtk  # noqa: E402

helper, model, data, *flags = sys.argv[1:]
phase = ["idle"]
frames = []  # (time, interval_ms, phase)


def batch():
    samples = []
    for name in ("00", "01", "02", "03", "04"):
        with wave.open(f"{data}/{name}.wav") as audio:
            raw = audio.readframes(audio.getnframes())
        samples += [s / 32768 for s in struct.unpack(f"<{len(raw) // 2}h", raw)] + [0.0] * 16000
    samples = samples[: 28 * 16000]
    header = json.dumps({"id": 1, "samples": len(samples)}).encode()
    return struct.pack(">I", len(header)) + header + struct.pack(f"<{len(samples)}f", *samples)


def drive(app):
    request = batch()
    time.sleep(10)
    phase[0] = "loading"
    unit = f"hopp-frame-probe-{int(time.time() * 1000)}"
    process = subprocess.Popen(
        ["systemd-run", "--user", "--scope", "--quiet", "--collect", f"--unit={unit}",
         "--property", "CPUWeight=idle", "--property", "IOWeight=1", "--",
         helper, "--model", model, *flags],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
    )
    json.loads(process.stdout.readline())
    phase[0] = "loaded, quiet"
    time.sleep(5)
    for index in range(4):
        phase[0] = f"transcribing {index + 1}"
        process.stdin.write(request)
        process.stdin.flush()
        process.stdout.readline()
    phase[0] = "done"
    process.stdin.close()
    process.wait()
    time.sleep(3)
    GLib.idle_add(app.quit)


def activate(app):
    window = Gtk.ApplicationWindow(application=app, title="hopp frame probe")
    window.set_default_size(240, 120)
    area = Gtk.DrawingArea()
    area.set_draw_func(lambda area, cr, w, h: (cr.set_source_rgb((time.time() * 2) % 1, 0.3, 0.6), cr.paint()))
    window.set_child(area)
    last = [None]

    def tick(widget, clock):
        now = clock.get_frame_time() / 1000  # µs → ms
        if last[0] is not None:
            frames.append((time.time(), now - last[0], phase[0]))
        last[0] = now
        widget.queue_draw()
        return True

    area.add_tick_callback(tick)
    window.present()
    threading.Thread(target=drive, args=(app,), daemon=True).start()


app = Gtk.Application(application_id="dev.hopp.frameprobe")
app.connect("activate", activate)
app.run(None)

refresh = sorted(interval for _, interval, _ in frames)[len(frames) // 2]
print(f"{os.path.basename(model)} {' '.join(flags)}: frame interval median {refresh:.1f} ms")
by_phase = {}
for _, interval, name in frames:
    by_phase.setdefault(name, []).append(interval)
for name, intervals in by_phase.items():
    slow = [interval for interval in intervals if interval > 1.5 * refresh]
    intervals.sort()
    p99 = intervals[int(len(intervals) * 0.99) - 1] if len(intervals) > 100 else max(intervals)
    print(f"  {name:<16} frames {len(intervals):5}  dropped {len(slow):4} ({100 * len(slow) / len(intervals):4.1f}%)"
          f"  p99 {p99:6.1f} ms  worst {max(intervals):7.1f} ms")
