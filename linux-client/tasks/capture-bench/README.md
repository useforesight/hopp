# Capture benchmarks

How Hopp runs whisper: how much work, memory and time per minute of speech, and that
nothing Hopp does to save work costs accuracy. The point is the engineering (GPU or
CPU, batching, priorities, limits), not comparing whisper models.

```sh
./fetch.sh data                                   # 73 LibriSpeech clips, 8 minutes
H=../../target/release/hopp-whisper
M=~/.local/share/hopp/models/ggml-large-v3.bin
python3 -I bench.py $H $M data --language en      # one clip per request
python3 -I bench.py --batch 25 $H $M data --language en   # how the recorder sends them
python3 -I peak.py $H $M data                     # peak memory in a scope of its own
python3 -I frame_probe.py $H $M data --cpu --threads 8   # desktop frame pacing meanwhile
```

`bench.py` reports speed as a multiple of real time, the helper's CPU time as a share
of the audio, latency per request, word error rate and peak RSS. `peak.py` reports
the scope's `memory.peak`, which includes GPU memory.

## Results

Apple M2 Max, Asahi Linux, Honeykrisp Vulkan, 14 October 2026.

**GPU against CPU** (base.en, one clip per request): the GPU does the same work
3× faster with 3% of the CPU time (CPU: 8.5× real time, 0.47 CPU-seconds per second
of speech; GPU: 24×, 0.014). The helper therefore runs on the GPU when there is one.

**Batching** (large-v3-turbo, all 73 clips): packing utterances a second apart into
one 30-second request and splitting the reply by DTW word timings.

| | Word errors | Speed | Helper CPU per second of speech |
|---|---|---|---|
| One clip per request | 4.61% | 3.0× real time | 0.074 s |
| Batched (25 s) | **3.30%** | **7.5×** | **0.040 s** |

Every word lands in the right clip (the per-clip error rate equals the per-request
one). Without DTW, whisper's own timings put a fifth of the words in the wrong clip;
with 0.4 s gaps instead of 1 s, accuracy dropped. DTW does not work with flash
attention (whisper.cpp drops the timings silently), so the helper turns flash attention
off when it times words.

**Rejected:** a shorter encoder window (12% errors against 8.6% for base.en);
quantized models; a style prompt (more errors, 25% slower).

**large-v3** (default) batched: 2.78% errors, 4.1× real time. large-v3-turbo: 3.30%,
7.5×, for about half the GPU time.

**Memory** (`peak.py`, scope `memory.peak`, GPU): base.en 576 MB, large-v3-turbo
2.4 GB, large-v3 4.6 GB, flat across repeated full windows. Process RSS stays near
100 MB: the rest is GPU memory, which on Apple silicon is system RAM charged to the
helper's cgroup. Hopp therefore runs the helper in a scope of its own
(`app-hopp-whisper-*`, `MemoryMax` 1.5× the model + 512 MB, `CPUWeight=idle`), never in
the client's, and unloads it after 90 quiet seconds.

**Desktop smoothness** (`frame_probe.py`, 120 Hz, frames slower than 1.5× the
refresh): idle 0.4% dropped. large-v3 on the GPU: 12–17% dropped while transcribing,
stalls up to 233 ms (cursor and typing lag); unchanged with a low-priority Vulkan queue
or 10× smaller submissions. base.en on the GPU: 8–11%, stalls up to 50 ms.
large-v3-turbo on the CPU (helper scope `CPUWeight=idle`), 4 or 8 threads: 0.2–0.4%,
the same as idle. Loading a model drops nothing. The GPU stalls came from the
kernel: the Asahi driver pinned compute to the highest priority so the firmware
couldn't preempt it. With Aurora's G14X fix (2026.10.11.14 or later; measured on a
kernel built from 2026.10.13.18, 60 Hz display) large-v3 on the GPU: 0.0% dropped
while loading and transcribing, worst frame 19.7 ms, at 3.9× real time and 2.78% WER.
Hence the GPU by default.

**CPU speed** (idle priority, batched, while other work kept about six of twelve cores
busy): large-v3-turbo 1.4× real time on 4 threads, 1.6× on 8, about 3× on 8 when the
machine is quieter; large-v3 1.1× on 4 and erratic on 8. Memory on the CPU: turbo
2.0 GB, large-v3 3.8 GB.

**Hopp's own work** for a capture (the audio tap, utterance splitting, resampling,
the database): 0.6 CPU-seconds for ten minutes of audio.

**Soak and recovery** (`hopp capture transcribe` on the 73 clips as a 10-minute
conversation, large-v3): 81 lines, no gaps, flat memory. Killing the helper part way
through: Hopp restarts it in a new scope and requeues the batch in flight.
