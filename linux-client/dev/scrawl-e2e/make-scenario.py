#!/usr/bin/env python3
"""Build the scripted pairing call for the Scrawl-in-Hopp end-to-end test.

Two people, Alex (shares the screen, like a Mac client) and Sam (the Linux client under
test, capturing), debug a checkout total that ignores a discount. This writes, into
OUT:

- slide-1..4.png   the shared screen over time, rendered by headless Chromium
- alex.wav sam.wav each person's voice on one shared timeline (Piper TTS)
- scenario.json    when slides change, who draws what where, and the bookmark

Usage: make-scenario.py OUT --piper PIPER --voices DIR --chromium CHROMIUM
"""

import argparse
import json
import math
import subprocess
import tempfile
import wave
from pathlib import Path

W, H = 1920, 1080
RATE = 22050

# Who says what, and when (seconds from the start of the call).
LINES = [
    ("alex", 1.5, "Okay, I'm sharing the checkout page. Something is off with the order total."),
    ("sam", 7.0, "Yeah, look here. The discount is applied, but the total is still one hundred and twenty."),
    ("alex", 13.5, "Right. And the discount line itself looks correct. Twelve pounds off."),
    ("sam", 19.0, "Let me drop a pin on the coupon field too. That's where save ten goes in."),
    ("alex", 25.0, "Here's the total function in cart dot T S."),
    ("sam", 30.0, "This line here returns the subtotal. It never subtracts the discount."),
    ("alex", 37.5, "Hey Claude, can you confirm that's the bug, and tell us the fix?"),
    ("sam", 44.0, "I'll bookmark this. The total ignores the discount."),
    ("alex", 50.0, "Okay, I've changed it to subtract the discount."),
    ("alex", 55.5, "And the checkout total now shows one hundred and eight pounds."),
    ("sam", 60.0, "Nice, that's right now. Ship it."),
]
SLIDES = [(0.0, "slide-1"), (24.0, "slide-2"), (49.0, "slide-3"), (54.5, "slide-4")]
DURATION = 68.0


def circle(cx, cy, rx, ry, steps=48):
    # Starts at the right, goes round once and a little past, like a hand-drawn ring.
    return [(cx + rx * math.cos(a), cy + ry * math.sin(a))
            for a in (i / steps * 2.15 * math.pi for i in range(steps + 1))]


def line(x0, y0, x1, y1, steps=30):
    return [(x0 + (x1 - x0) * i / steps, y0 + (y1 - y0) * i / steps) for i in range(steps + 1)]


# What gets drawn: the expected bounds are checked against the frames' ink.
MARKS = [
    {"at": 8.5, "by": "sam", "kind": "stroke", "about": "the wrong total",
     "points": circle(0.845, 0.595, 0.065, 0.045)},
    {"at": 15.0, "by": "alex", "kind": "stroke", "about": "the discount amount",
     "points": line(0.82, 0.525, 0.895, 0.525)},
    {"at": 21.0, "by": "sam", "kind": "pin", "about": "the coupon field", "point": (0.24, 0.435)},
    {"at": 31.5, "by": "sam", "kind": "stroke", "about": "the return line",
     "points": circle(0.115, 0.534, 0.05, 0.022)},
    {"at": 61.0, "by": "sam", "kind": "stroke", "about": "the fixed total",
     "points": circle(0.845, 0.595, 0.065, 0.045)},
]
BOOKMARKS = [{"at": 45.0, "by": "sam", "label": "total ignores the discount"}]
# Stamps: a pin, then words appearing over about a second, then done.
STAMPS = [
    {"at": 34.5, "by": "sam", "point": (0.16, 0.535), "text": "Return sub minus the discount here"},
    {"at": 57.5, "by": "alex", "point": (0.88, 0.595), "text": "Total is right now"},
]

STYLE = """
body { margin:0; width:1920px; height:1080px; font-family: 'Inter', 'Helvetica Neue', sans-serif; }
.abs { position:absolute; }
"""


def checkout(total, fixed):
    rows = [
        ("Subtotal", "£120.00", 0.40),
        ("Discount (SAVE10)", "−£12.00", 0.48),
    ]
    out = [f"<html><head><meta charset='utf-8'><style>{STYLE}</style></head><body style='background:#f5f5f7'>",
           "<div class='abs' style='left:0;top:0;width:1920px;height:90px;background:#1d1d1f;color:white;"
           "font-size:30px;line-height:90px;padding-left:60px'>shop.example · Checkout</div>",
           "<div class='abs' style='left:150px;top:370px;font-size:30px;color:#333'>Coupon code</div>",
           f"<div class='abs' style='left:{0.08*W}px;top:{0.40*H}px;width:{0.32*W}px;height:{0.07*H}px;"
           "border:2px solid #999;border-radius:10px;background:white;font-size:34px;line-height:75px;"
           "padding-left:20px;box-sizing:border-box'>SAVE10</div>",
           f"<div class='abs' style='left:{0.55*W}px;top:{0.30*H}px;width:{0.37*W}px;height:{0.45*H}px;"
           "background:white;border-radius:16px;box-shadow:0 4px 20px rgba(0,0,0,.08)'></div>",
           f"<div class='abs' style='left:{0.58*W}px;top:{0.32*H}px;font-size:34px;font-weight:600'>Order summary</div>"]
    for label, value, y in rows:
        out.append(f"<div class='abs' style='left:{0.58*W}px;top:{y*H}px;font-size:32px;color:#444'>{label}</div>")
        out.append(f"<div class='abs' style='left:{0.75*W}px;width:{0.14*W}px;text-align:right;top:{y*H}px;"
                   f"font-size:32px;color:#444'>{value}</div>")
    colour = "#1a7f37" if fixed else "#1d1d1f"
    out.append(f"<div class='abs' style='left:{0.58*W}px;top:{0.575*H}px;font-size:40px;font-weight:700'>Total</div>")
    out.append(f"<div class='abs' style='left:{0.72*W}px;width:{0.17*W}px;text-align:right;top:{0.575*H}px;"
               f"font-size:40px;font-weight:700;color:{colour}'>{total}</div>")
    out.append(f"<div class='abs' style='left:{0.58*W}px;top:{0.66*H}px;width:{0.31*W}px;height:70px;"
               "background:#0071e3;border-radius:12px;color:white;font-size:30px;text-align:center;"
               "line-height:70px'>Pay now</div>")
    out.append("</body></html>")
    return "".join(out)


CODE = [
    "import { Item } from './types';",
    "",
    "export function subtotal(items: Item[]): number {",
    "  return items.reduce((sum, item) => sum + item.price * item.qty, 0);",
    "}",
    "",
    "export function discountFor(code: string, amount: number): number {",
    "  return code === 'SAVE10' ? amount * 0.1 : 0;",
    "}",
    "",
    "export function total(items: Item[], code: string): number {",
    "  const sub = subtotal(items);",
    "  const discount = discountFor(code, sub);",
    "  return sub;",
    "}",
]


def editor(fixed):
    lines = list(CODE)
    if fixed:
        lines[13] = "  return sub - discount;"
    out = [f"<html><head><meta charset='utf-8'><style>{STYLE}</style></head>"
           "<body style='background:#1e1e2e;color:#cdd6f4'>",
           "<div class='abs' style='left:0;top:0;width:1920px;height:60px;background:#181825;"
           "font-size:24px;line-height:60px;padding-left:30px;color:#a6adc8'>cart.ts — checkout</div>"]
    top, step = 120, 34
    for index, text in enumerate(lines):
        y = top + index * step
        if index == 13:
            bg = "#2b4a33" if fixed else "#3a2f45"
            out.append(f"<div class='abs' style='left:0;top:{y-4}px;width:1920px;height:{step}px;background:{bg}'></div>")
        out.append(f"<div class='abs' style='left:30px;top:{y}px;width:60px;text-align:right;font-family:monospace;"
                   f"font-size:24px;color:#6c7086'>{index+1}</div>")
        safe = text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
        out.append(f"<div class='abs' style='left:130px;top:{y}px;font-family:monospace;font-size:24px;"
                   f"white-space:pre'>{safe}</div>")
    out.append("</body></html>")
    return "".join(out)


def render(chromium, html, png):
    with tempfile.TemporaryDirectory() as profile:
        page = Path(profile) / "page.html"
        page.write_text(html, encoding="utf-8")
        subprocess.run([chromium, "--headless=new", "--disable-gpu", f"--user-data-dir={profile}",
                        "--hide-scrollbars", f"--window-size={W},{H}", f"--screenshot={png}",
                        page.as_uri()], check=True, capture_output=True, timeout=60)


def speak(piper, voice, text):
    with tempfile.NamedTemporaryFile(suffix=".wav") as out:
        subprocess.run([piper, "-m", str(voice), "-f", out.name], input=text.encode(), check=True,
                       capture_output=True, timeout=120)
        with wave.open(out.name) as w:
            assert w.getframerate() == RATE and w.getnchannels() == 1 and w.getsampwidth() == 2
            return w.readframes(w.getnframes())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("out", type=Path)
    parser.add_argument("--piper", required=True)
    parser.add_argument("--voices", type=Path, required=True)
    parser.add_argument("--chromium", default="chromium")
    args = parser.parse_args()
    out = args.out
    out.mkdir(parents=True, exist_ok=True)

    render(args.chromium, checkout("£120.00", False), out / "slide-1.png")
    render(args.chromium, editor(False), out / "slide-2.png")
    render(args.chromium, editor(True), out / "slide-3.png")
    render(args.chromium, checkout("£108.00", True), out / "slide-4.png")

    voices = {"alex": args.voices / "en_GB-alan-medium.onnx", "sam": args.voices / "en_US-ryan-medium.onnx"}
    tracks = {who: bytearray(int(DURATION * RATE) * 2) for who in voices}
    spoken = []
    for who, at, text in LINES:
        pcm = speak(args.piper, voices[who], text)
        start = int(at * RATE) * 2
        end = start + len(pcm)
        assert end <= len(tracks[who]), f"{text!r} runs past the end"
        tracks[who][start:end] = pcm
        spoken.append({"who": who, "at": at, "until": round(at + len(pcm) / 2 / RATE, 2), "text": text})
    for who, pcm in tracks.items():
        with wave.open(str(out / f"{who}.wav"), "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(RATE)
            w.writeframes(bytes(pcm))

    scenario = {
        "duration": DURATION,
        "slides": [{"at": at, "png": str(out / f"{name}.png")} for at, name in SLIDES],
        "voices": {who: str(out / f"{who}.wav") for who in voices},
        "marks": [{**mark, "points": [list(p) for p in mark.get("points", [])],
                   "point": list(mark["point"]) if "point" in mark else None} for mark in MARKS],
        "bookmarks": BOOKMARKS,
        "stamps": [{**stamp, "point": list(stamp["point"])} for stamp in STAMPS],
        "lines": spoken,
    }
    (out / "scenario.json").write_text(json.dumps(scenario, indent=2))
    print(out / "scenario.json")


if __name__ == "__main__":
    main()
