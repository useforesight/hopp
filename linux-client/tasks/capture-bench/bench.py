"""Benchmark hopp-whisper on LibriSpeech clips: work, latency, accuracy, memory.

usage: bench.py [--clips N] [--batch SECONDS] [--idle] HELPER MODEL DATA_DIR [helper flags…]

--batch packs consecutive clips (1 s of silence between) into one request of up to
SECONDS, then splits the returned word timings back to each clip, as the recorder does.
"WER" is per clip after splitting; "whole" is per request, before splitting.
--idle runs the helper with SCHED_IDLE (chrt -i 0).
"""
import glob, json, os, re, statistics, struct, subprocess, sys, time, wave

arguments = sys.argv[1:]
limit, batch, idle = None, 0.0, False
while arguments and arguments[0].startswith("--"):
    option = arguments.pop(0)
    if option == "--clips":
        limit = int(arguments.pop(0))
    elif option == "--batch":
        batch = float(arguments.pop(0))
    elif option == "--idle":
        idle = True
helper, model, data, *flags = arguments
refs = json.load(open(os.path.join(data, "refs.json")))
clips = sorted(glob.glob(os.path.join(data, "*.wav")))[:limit]
GAP = [0.0] * int(1.0 * 16000)
WORDS = {"mr": "mister", "mrs": "missus", "dr": "doctor", "st": "saint"}


def words(text):
    text = re.sub(r"[^a-z0-9' ]", " ", text.lower().replace("-", " "))
    return [WORDS.get(word, word) for word in text.split()]


def edits(reference, hypothesis):
    previous = list(range(len(hypothesis) + 1))
    for i, ref in enumerate(reference, 1):
        current = [i]
        for j, hyp in enumerate(hypothesis, 1):
            current.append(min(previous[j] + 1, current[j - 1] + 1, previous[j - 1] + (ref != hyp)))
        previous = current
    return previous[-1]


def proc(pid):
    """CPU seconds (user + system) and peak RSS in MB."""
    fields = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
    cpu = (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK")
    rss = next(int(line.split()[1]) for line in open(f"/proc/{pid}/status") if line.startswith("VmHWM:")) / 1024
    return cpu, rss


def load(clip):
    with wave.open(clip) as audio:
        raw = audio.readframes(audio.getnframes())
    return [sample / 32768 for sample in struct.unpack(f"<{len(raw) // 2}h", raw)]


# Requests: each is a list of (clip, samples); unbatched, one clip per request.
requests, current, length = [], [], 0.0
for clip in clips:
    samples = load(clip)
    seconds = len(samples) / 16000
    if current and (not batch or length + seconds + 1.0 > batch):
        requests.append(current)
        current, length = [], 0.0
    current.append((clip, samples))
    length += seconds + 1.0
requests.append(current)

command = [helper, "--model", model, *flags]
if idle:
    command = ["chrt", "-i", "0", *command]
started = time.time()
process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
ready = json.loads(process.stdout.readline())
load_time = time.time() - started
if "error" in ready:
    sys.exit(f"helper failed: {ready['error']}")
cpu_before, _ = proc(process.pid)

latencies, audio_total, compute_total, errors, reference_words, whole_errors = [], 0.0, 0.0, 0, 0, 0
for index, parts in enumerate(requests):
    samples, bounds = [], []
    for clip, clip_samples in parts:
        start = len(samples) / 16000
        samples += clip_samples + GAP
        bounds.append((clip, start, len(samples) / 16000))
    del samples[len(samples) - len(GAP):]
    header = json.dumps({"id": index, "samples": len(samples)}).encode()
    sent = time.time()
    process.stdin.write(struct.pack(">I", len(header)) + header + struct.pack(f"<{len(samples)}f", *samples))
    process.stdin.flush()
    reply = json.loads(process.stdout.readline())
    latencies.append(time.time() - sent)
    compute_total += reply["ms"] / 1000
    texts = {clip: [] for clip, _, _ in bounds}
    if len(bounds) == 1:
        texts[bounds[0][0]] = [reply["text"]]
    else:
        for segment in reply["segments"]:
            middle = (segment["t0_ms"] + segment["t1_ms"]) / 2000
            owner = next((clip for clip, start, end in bounds if start <= middle < end), bounds[-1][0])
            texts[owner].append(segment["text"])
    whole_reference = [word for clip, _, _ in bounds for word in words(refs[os.path.basename(clip)])]
    whole_errors += edits(whole_reference, words(reply["text"]))
    if os.environ.get("SHOW") and len(bounds) > 1:
        print("SEGMENTS", [(s["t0_ms"], s["t1_ms"], s["text"][:40]) for s in reply["segments"]], "BOUNDS", [(round(a, 1), round(b, 1)) for _, a, b in bounds], file=sys.stderr)
    for clip, start, end in bounds:
        audio_total += end - start - 1.0
        reference = words(refs[os.path.basename(clip)])
        errors += edits(reference, words("".join(texts[clip])))
        reference_words += len(reference)

cpu_after, memory = proc(process.pid)
process.stdin.close()
process.wait()
latencies.sort()
p95 = latencies[max(0, int(len(latencies) * 0.95) - 1)]
name = os.path.basename(model).removeprefix("ggml-").removesuffix(".bin")
label = " ".join(flags) + (f" batch={batch:g}s" if batch else "") + (" idle" if idle else "")
print(
    f"{name:<20} {label:<62} dev={ready.get('device', '?')[:13]:<13} "
    f"wall x{audio_total / compute_total:5.1f}  "
    f"CPU {100 * (cpu_after - cpu_before) / audio_total:5.1f}% of audio  "
    f"p50 {statistics.median(latencies) * 1000:5.0f}ms p95 {p95 * 1000:5.0f}ms  "
    f"WER {100 * errors / reference_words:5.2f}% (whole {100 * whole_errors / reference_words:5.2f}%)  RSS {memory:5.0f}MB  load {load_time:4.1f}s",
    flush=True,
)
