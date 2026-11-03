#!/usr/bin/env bash
# Run the Scrawl-in-Hopp end-to-end check: a scripted two-person call on the local
# Hopp server, captured by the real engine, with an agent following along.
#
# Nothing here touches your own Hopp: config and captures live in $RUN/config and
# $RUN/data, the speaker is silent, the microphones are WAV files, and the agent reads
# only this run's captures.
#
#   dev/scrawl-e2e/run.sh [RUN_DIR]       default /tmp/scrawl-e2e/call-<time>
#
# Needs: dev/local-stack.py started and seeded (state in $HOPP_LOCAL, default
# /tmp/hopp-local-scrawl), a scenario from make-scenario.py in $SCENARIO (default
# /tmp/scrawl-e2e/run/scenario.json), `claude` on PATH for the agent (set NO_AGENT=1
# to skip it), and hopp-transcribe with Hex's Parakeet model.
set -euo pipefail
here="$(cd "$(dirname "$0")/../.." && pwd)"
RUN="${1:-/tmp/scrawl-e2e/call-$(date +%H%M%S)}"
HOPP_LOCAL="${HOPP_LOCAL:-/tmp/hopp-local-scrawl}"
SCENARIO="${SCENARIO:-/tmp/scrawl-e2e/run/scenario.json}"
case "$RUN" in /tmp/?*) ;; *) echo "RUN must be under /tmp" >&2; exit 1 ;; esac
mkdir -p "$RUN"/{config,data,out,agent}
chmod 700 "$RUN"
# Hopp looks for Hex's models next to its data folder; reuse them rather than download.
mkdir -p "$RUN/data/voice-control"
ln -sfn "$HOME/.local/share/voice-control/models" "$RUN/data/voice-control/models"

export XDG_CONFIG_HOME="$RUN/config" XDG_DATA_HOME="$RUN/data"
export HOPP_API_TEST_FIXTURE="$HOPP_LOCAL/client-test.json"
export HOPP_SCRAWL_SCENARIO="$SCENARIO" HOPP_SCRAWL_OUT="$RUN/out"
export PATH="$HOME/.local/bin:$PATH"

cd "$here"
nice -n 10 cargo build --bin hopp -j 4 --quiet
nice -n 10 cargo test --no-run --bin hopp -j 4 --quiet 2>/dev/null
echo "Run: $RUN"

agent() {
  # Wait for the live capture, then follow it like a teammate would.
  until "$here/target/debug/hopp" capture list --json 2>/dev/null \
    | python3 -c 'import json,sys; sys.exit(0 if any("ended_ms" not in c for c in json.load(sys.stdin)) else 1)'; do
    sleep 1
  done
  cat > "$RUN/agent/mcp.json" <<JSON
{"mcpServers": {"hopp": {"command": "$here/target/debug/hopp", "args": ["mcp"],
  "env": {"XDG_DATA_HOME": "$RUN/data", "XDG_CONFIG_HOME": "$RUN/config"}}}}
JSON
  cd "$RUN/agent"
  claude -p --strict-mcp-config --mcp-config "$RUN/agent/mcp.json" \
    --allowedTools "mcp__hopp" --output-format stream-json --verbose --max-turns 60 \
    "$(cat "$here/dev/scrawl-e2e/agent-prompt.md")" > "$RUN/agent/stream.jsonl" 2> "$RUN/agent/stderr.log" || true
}

if [ -z "${NO_AGENT:-}" ]; then
  agent & agent_pid=$!
fi
status=0
nice -n 10 cargo test --bin hopp -j 4 --quiet scripted_call_becomes_frames_and_transcript \
  -- --ignored --nocapture 2>&1 | tee "$RUN/out/test.log" || status=$?
if [ -n "${agent_pid:-}" ]; then
  # The agent stops by itself when the capture ends; give it a little longer.
  for _ in $(seq 120); do kill -0 "$agent_pid" 2>/dev/null || break; sleep 1; done
  kill "$agent_pid" 2>/dev/null || true
  python3 "$here/dev/scrawl-e2e/agent-thread.py" "$RUN/agent/stream.jsonl" > "$RUN/out/agent-thread.md" || true
fi
echo "Run: $RUN (test exit $status)"
exit "$status"
