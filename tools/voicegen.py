# /// script
# requires-python = ">=3.10,<3.13"
# dependencies = ["kokoro-onnx>=0.4", "soundfile>=0.12", "numpy"]
# ///
"""Renders Stickfell's finisher voice lines (tools/voicelines.py) offline with
the Kokoro neural TTS model (Apache-2.0) and writes game/src/voice.rs.
Natural voices, but no acting: the [directions] in the lines are dropped.
For acted lines use tools/voicegen_elevenlabs.py instead.

    uv run tools/voicegen.py

The model (~350 MB) is downloaded once into ~/.cache/stickfell-voicegen.
"""

import os
import sys
import urllib.request

import numpy as np
import soundfile as sf
from kokoro_onnx import Kokoro

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import voicelines as vl  # noqa: E402

CACHE_ROOT = os.path.join(os.path.expanduser("~"), ".cache", "stickfell-voicegen")
_OLD_CACHE = os.path.join(os.path.expanduser("~"), ".cache", "scuffle-voicegen")
if os.path.isdir(_OLD_CACHE) and not os.path.exists(CACHE_ROOT):
    os.rename(_OLD_CACHE, CACHE_ROOT)
CACHE = CACHE_ROOT
RELEASE = "https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/"
MODEL_FILES = ["kokoro-v1.0.onnx", "voices-v1.0.bin"]

# the ring announcer (deep, unhurried, echo added) and one voice per player slot
ANNOUNCER = ("am_onyx", 0.85)
PLAYERS = [("am_fenrir", 1.0), ("bm_george", 1.0), ("am_puck", 1.05), ("am_echo", 1.0)]
PLEA_SPEED = 1.2  # begging is faster than gloating


def fetch_model():
    os.makedirs(CACHE, exist_ok=True)
    for name in MODEL_FILES:
        path = os.path.join(CACHE, name)
        if not os.path.exists(path):
            print(f"downloading {name} ...")
            urllib.request.urlretrieve(RELEASE + name, path + ".part")
            os.replace(path + ".part", path)
    return [os.path.join(CACHE, n) for n in MODEL_FILES]


def trim(x, rate, floor=0.01):
    loud = np.nonzero(np.abs(x) > floor)[0]
    if len(loud) == 0:
        return x
    return x[max(0, loud[0] - rate // 100) : min(len(x), loud[-1] + rate // 20)]


def echo(x, rate, delay=0.11, feedback=0.3, taps=4):
    """A big-room slap-back for the announcer."""
    d = int(rate * delay)
    out = np.concatenate([x, np.zeros(d * taps, dtype=x.dtype)])
    for k in range(1, taps + 1):
        out[d * k : d * k + len(x)] += x * feedback**k
    return out


def normalize(x, peak=0.8):
    m = np.max(np.abs(x)) if len(x) else 0.0
    return x * (peak / m) if m > 0 else x


def main():
    tts = Kokoro(*fetch_model())
    os.makedirs(vl.OUT_DIR, exist_ok=True)
    total = 0
    for fname, kind, slot, text in vl.jobs():
        voice, speed = ANNOUNCER if slot is None else PLAYERS[slot]
        if kind == "plea":
            speed *= PLEA_SPEED
        line = vl.plain(text)
        if kind == "announce":
            line = line.capitalize()
        x, rate = tts.create(line, voice=voice, speed=speed, lang="en-us")
        x = trim(np.asarray(x, dtype=np.float32), rate)
        if kind == "announce":
            x = echo(x, rate)
        path = os.path.join(vl.OUT_DIR, fname)
        sf.write(path, normalize(x), rate, format="OGG", subtype="VORBIS")
        total += os.path.getsize(path)
        print(f"  {fname:28} {len(x) / rate:4.2f} s  {voice:10} {line}")
    vl.write_rust("tools/voicegen.py (Kokoro)")
    print(f"{len(vl.jobs())} lines, {total / 1e6:.2f} MB -> {vl.OUT_DIR}")


if __name__ == "__main__":
    sys.exit(main())
