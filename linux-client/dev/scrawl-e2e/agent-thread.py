#!/usr/bin/env python3
"""Turn a `claude -p --output-format stream-json` log into a readable Markdown thread:
the agent's messages, and each tool call with a one-line summary of its result."""

import json
import sys
import time


def main(path):
    start = None
    print("# Agent thread\n")
    for raw in open(path, encoding="utf-8"):
        try:
            event = json.loads(raw)
        except json.JSONDecodeError:
            continue
        now = time.time()
        start = start or now
        kind = event.get("type")
        if kind == "assistant":
            for block in event["message"].get("content", []):
                if block.get("type") == "text" and block["text"].strip():
                    print(block["text"].strip() + "\n")
                elif block.get("type") == "tool_use":
                    args = json.dumps(block.get("input", {}))
                    print(f"> `{block['name'].split('__')[-1]}` {args}\n")
        elif kind == "user":
            for block in event["message"].get("content", []):
                if block.get("type") != "tool_result":
                    continue
                content = block.get("content")
                parts = content if isinstance(content, list) else [{"type": "text", "text": str(content)}]
                images = sum(1 for p in parts if p.get("type") == "image")
                text = " ".join(p.get("text", "") for p in parts if p.get("type") == "text")
                lines = []
                try:
                    body = json.loads(text.split("\n{")[0] if text.startswith("{") else text)
                    lines = body.get("lines", [])
                except (json.JSONDecodeError, AttributeError):
                    pass
                summary = "; ".join(lines) if lines else text[:300].replace("\n", " ")
                extra = f" (+{images} image{'s' if images != 1 else ''})" if images else ""
                print(f">> {summary}{extra}\n")
        elif kind == "result":
            print(f"---\n\n_{event.get('num_turns')} turns, {event.get('duration_ms', 0)/1000:.0f} s, "
                  f"${event.get('total_cost_usd', 0):.2f}_")


if __name__ == "__main__":
    main(sys.argv[1])
