#!/bin/sh
# The 73 LibriSpeech validation clips (CC BY 4.0) from Hugging Face, as 16 kHz WAV, with
# their transcripts in refs.json. Needs curl and ffmpeg.
set -eu
data=${1:-data}
mkdir -p "$data"
cd "$data"
for offset in 0 50; do
  curl -fsS "https://datasets-server.huggingface.co/rows?dataset=hf-internal-testing/librispeech_asr_dummy&config=clean&split=validation&offset=$offset&length=50"
  echo
done > rows.jsonl
python3 -I - <<'PY'
import json, os, subprocess
refs = {}
for line in open("rows.jsonl"):
    if not line.strip():
        continue
    for item in json.loads(line)["rows"]:
        index, row = item["row_idx"], item["row"]
        flac, wav = f"{index:02d}.flac", f"{index:02d}.wav"
        subprocess.run(["curl", "-fsSL", "-o", flac, row["audio"][0]["src"]], check=True)
        subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-i", flac, "-ar", "16000", "-ac", "1",
                        "-c:a", "pcm_s16le", wav], check=True)
        os.remove(flac)
        refs[wav] = row["text"]
json.dump(refs, open("refs.json", "w"), indent=1)
print(len(refs), "clips")
PY
