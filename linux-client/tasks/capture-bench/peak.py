"""Peak memory of hopp-whisper in a scope of its own, GPU memory included.

On Apple silicon GPU memory is system RAM, charged to the cgroup that allocates it, so
the scope's memory.peak is the number that matters for limits (process RSS is not).
Loads the model, transcribes three full 28 s windows, and reports the peak.

usage: peak.py HELPER MODEL DATA_DIR [helper flags…]
"""
import json, struct, subprocess, sys, time, wave

helper, model, data, *flags = sys.argv[1:]
unit = f"hopp-peak-probe-{int(time.time() * 1000)}"
process = subprocess.Popen(
    ["systemd-run", "--user", "--scope", "--quiet", "--collect", f"--unit={unit}", "--",
     helper, "--model", model, *flags],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE,
)
ready = json.loads(process.stdout.readline())
path = subprocess.run(["systemctl", "--user", "show", "-P", "ControlGroup", f"{unit}.scope"],
                      capture_output=True, text=True).stdout.strip()
cgroup = f"/sys/fs/cgroup{path}"
loaded = int(open(f"{cgroup}/memory.current").read())

samples = []
for name in ("00", "01", "02", "03", "04"):
    with wave.open(f"{data}/{name}.wav") as audio:
        raw = audio.readframes(audio.getnframes())
    samples += [sample / 32768 for sample in struct.unpack(f"<{len(raw) // 2}h", raw)] + [0.0] * 16000
samples = samples[: 28 * 16000]
for index in range(3):
    header = json.dumps({"id": index, "samples": len(samples)}).encode()
    process.stdin.write(struct.pack(">I", len(header)) + header + struct.pack(f"<{len(samples)}f", *samples))
    process.stdin.flush()
    json.loads(process.stdout.readline())
peak = int(open(f"{cgroup}/memory.peak").read())
process.stdin.close()
process.wait()
megabytes = lambda value: f"{value / 2**20:5.0f} MB"
print(f"{model.rsplit('/', 1)[-1]:<26} loaded {megabytes(loaded)}  peak {megabytes(peak)}  device {ready['device']}")
