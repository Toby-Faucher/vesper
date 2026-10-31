#!/usr/bin/env python3
"""Print the gap between the first two clicks in a mono or stereo 16-bit WAV (stdlib only).

usage: click_offset.py recording.wav [threshold 0..1, default 0.3]
Also: click_offset.py --make-click click.wav   (writes a 1 s WAV with one 2 ms click at 0.2 s)
"""
import math, struct, sys, wave

RATE = 48000

def make_click(path):
    with wave.open(path, "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(RATE)
        n = RATE
        s = [0] * n
        for i in range(RATE // 500):  # 2 ms burst of 2 kHz
            s[RATE // 5 + i] = int(30000 * math.sin(2 * math.pi * 2000 * i / RATE))
        w.writeframes(struct.pack(f"<{n}h", *s))

def onsets(path, thr):
    with wave.open(path) as w:
        ch, rate, raw = w.getnchannels(), w.getframerate(), w.readframes(w.getnframes())
    s = struct.unpack(f"<{len(raw) // 2}h", raw)[::ch]
    lim = thr * 32768
    out, quiet_until = [], 0
    for i, v in enumerate(s):
        if i >= quiet_until and abs(v) > lim:
            out.append(i / rate)
            quiet_until = i + rate // 10  # ignore the rest of this click and its ringing
    return out

if len(sys.argv) < 2 or (sys.argv[1] == "--make-click" and len(sys.argv) < 3):
    print(__doc__ + "\nerror: missing arguments", file=sys.stderr)
    sys.exit(2)
if sys.argv[1] == "--make-click":
    make_click(sys.argv[2])
else:
    t = onsets(sys.argv[1], float(sys.argv[2]) if len(sys.argv) > 2 else 0.3)
    if len(t) < 2:
        sys.exit(f"found {len(t)} click(s); need 2 (lower the threshold?)")
    print(f"clicks at {', '.join(f'{x:.3f}s' for x in t)}; first gap {1000 * (t[1] - t[0]):.1f} ms")
