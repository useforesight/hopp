#!/usr/bin/env bash
# Start the publisher and viewer fixture clients in the background, logging the call
# lifecycle to /tmp/hopp-live-<peer>.log. Usage: dev/run-both.sh [client-test.json]
set -euo pipefail
cd "$(dirname "$0")/.."
fixture="${1:-/tmp/hopp-local-$(id -u)/client-test.json}"
for peer in publisher viewer; do
  nohup python dev/run-client.py --fixture "$fixture" --peer "$peer" \
    --binary target/release/hopp >"/tmp/hopp-live-$peer.log" 2>&1 &
done
