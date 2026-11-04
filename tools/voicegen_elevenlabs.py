# /// script
# requires-python = ">=3.10"
# dependencies = ["soundfile>=0.12", "numpy"]
# ///
"""Renders Stickfell's finisher voice lines (tools/voicelines.py) with
ElevenLabs v3, which acts out the [directions] in the lines, and writes
game/src/voice.rs. Only the rendered audio ships.

    uv run tools/voicegen_elevenlabs.py            # voices, credits, cost: spends nothing
    uv run tools/voicegen_elevenlabs.py --yes      # render (cached lines are free)
    uv run tools/voicegen_elevenlabs.py --list     # every voice on the account

The API key comes from the ELEVENLABS_API_KEY environment variable (or the
file ~/.elevenlabs-key). It is only ever sent to api.elevenlabs.io and never
printed or written anywhere. Rendered lines are cached by (voice, text,
settings) in ~/.cache/stickfell-voicegen/elevenlabs, so reruns only pay for
lines that changed.
"""

import argparse
import hashlib
import json
import os
import sys
import urllib.error
import urllib.request

import numpy as np
import soundfile as sf

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import voicelines as vl  # noqa: E402

API = "https://api.elevenlabs.io"
MODEL = "eleven_v3"
RATE = 24000
CACHE_ROOT = os.path.join(os.path.expanduser("~"), ".cache", "stickfell-voicegen")
_OLD_CACHE = os.path.join(os.path.expanduser("~"), ".cache", "scuffle-voicegen")
if os.path.isdir(_OLD_CACHE) and not os.path.exists(CACHE_ROOT):
    os.rename(_OLD_CACHE, CACHE_ROOT)
CACHE = os.path.join(CACHE_ROOT, "elevenlabs")

# voices by name (--list shows the account's library, if the key may read it)
ANNOUNCER = "Brian"
PLAYERS = ["Callum", "George", "Charlie", "Will"]
# ElevenLabs' built-in voices, for keys that can't list voices
BUILT_IN = {
    "Brian": "nPczCjzI2devNBz1zQrb",
    "Callum": "N2lVS1w4EtoT3dr4eOWO",
    "George": "JBFqnCBsd6RMkjVDRZzb",
    "Charlie": "IKne3meq5aSn9XLyUdCD",
    "Will": "bIHbv24MWmeRgasZH58o",
}
# v3 stability: 0.0 creative (most expressive), 0.5 natural, 1.0 robust
STABILITY = {"announce": 0.5, "quip": 0.0, "plea": 0.0}


def api_key():
    key = os.environ.get("ELEVENLABS_API_KEY", "").strip()
    if not key:
        path = os.path.join(os.path.expanduser("~"), ".elevenlabs-key")
        if os.path.exists(path):
            with open(path, encoding="utf-8") as f:
                key = f.read().strip()
    if not key:
        sys.exit("no API key: set ELEVENLABS_API_KEY (or put it in ~/.elevenlabs-key)")
    return key


def call(key, path, body=None, optional=False):
    req = urllib.request.Request(API + path, headers={"xi-api-key": key, "Content-Type": "application/json"})
    if body is not None:
        req.data = json.dumps(body).encode()
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            return r.read()
    except urllib.error.HTTPError as e:
        if optional and e.code in (401, 403):
            return None  # a restricted key: carry on without it
        # (the error body never contains the key)
        sys.exit(f"ElevenLabs {path.split('?')[0]}: HTTP {e.code}: {e.read()[:400].decode(errors='replace')}")


def trim(x, floor=0.01):
    loud = np.nonzero(np.abs(x) > floor)[0]
    if len(loud) == 0:
        return x
    return x[max(0, loud[0] - RATE // 100) : min(len(x), loud[-1] + RATE // 20)]


def echo(x, delay=0.11, feedback=0.28, taps=4):
    """A big-room slap-back for the announcer."""
    d = int(RATE * delay)
    out = np.concatenate([x, np.zeros(d * taps, dtype=x.dtype)])
    for k in range(1, taps + 1):
        out[d * k : d * k + len(x)] += x * feedback**k
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--yes", action="store_true", help="actually render (spends credits on uncached lines)")
    ap.add_argument("--list", action="store_true", help="list the account's voices")
    ap.add_argument("--sample", metavar="DIR", help="render just a few lines (one per voice and kind) into DIR to audition; the game is left alone")
    args = ap.parse_args()
    key = api_key()

    listing = call(key, "/v1/voices", optional=True)
    by_name = {n: {"voice_id": i} for n, i in BUILT_IN.items()}
    if listing is None:
        print("(this key can't list voices: using the built-in voice IDs)")
    else:
        for v in json.loads(listing)["voices"]:
            by_name.setdefault(v["name"].split(" - ")[0].strip(), v)
    if args.list:
        for name, v in sorted(by_name.items()):
            labels = ", ".join(f"{k}: {x}" for k, x in (v.get("labels") or {}).items())
            print(f"  {name:20} {v.get('category', ''):10} {labels}")
        return
    missing = [n for n in [ANNOUNCER, *PLAYERS] if n not in by_name]
    if missing:
        sys.exit(f"voices not on this account: {missing} (see --list)")
    ids = {n: by_name[n]["voice_id"] for n in [ANNOUNCER, *PLAYERS]}

    jobs = vl.jobs()
    if args.sample:
        keep = {"announce_spin.ogg", "announce_slap.ogg", "p0_plea_3.ogg", "p0_plea_5.ogg", "p0_quip_spin_0.ogg", "p1_plea_10.ogg", "p1_quip_any_2.ogg", "p2_plea_4.ogg", "p2_quip_dismember_0.ogg", "p3_plea_8.ogg", "p3_quip_spine_0.ogg"}
        jobs = [j for j in jobs if j[0] in keep]
    out_dir = args.sample or vl.OUT_DIR
    todo, cached = [], 0
    for fname, kind, slot, text in jobs:
        voice = ANNOUNCER if slot is None else PLAYERS[slot]
        settings = {"stability": STABILITY[kind]}
        tag = hashlib.sha256(json.dumps([MODEL, ids[voice], text, settings]).encode()).hexdigest()[:24]
        cache = os.path.join(CACHE, tag + ".pcm")
        if os.path.exists(cache):
            cached += 1
        todo.append((fname, kind, voice, text, settings, cache))
    cost = sum(len(t[3]) for t in todo if not os.path.exists(t[5]))
    sub = call(key, "/v1/user/subscription", optional=True)
    left = None
    if sub is not None:
        sub = json.loads(sub)
        left = sub.get("character_limit", 0) - sub.get("character_count", 0)
    print(f"voices: announcer {ANNOUNCER}, players {', '.join(PLAYERS)}")
    print(f"{len(todo)} lines ({cached} cached); about {cost} characters to render; " + (f"{left} left on the account" if left is not None else "credits unknown (key can't read the account)"))
    if not args.yes:
        print("dry run: pass --yes to render")
        return
    if left is not None and cost > left:
        sys.exit("not enough credits left")

    os.makedirs(CACHE, exist_ok=True)
    os.makedirs(out_dir, exist_ok=True)
    total = 0
    for fname, kind, voice, text, settings, cache in todo:
        if not os.path.exists(cache):
            pcm = call(key, f"/v1/text-to-speech/{ids[voice]}?output_format=pcm_{RATE}", {"text": text, "model_id": MODEL, "voice_settings": settings})
            with open(cache + ".part", "wb") as f:
                f.write(pcm)
            os.replace(cache + ".part", cache)
        with open(cache, "rb") as f:
            x = np.frombuffer(f.read(), dtype="<i2").astype(np.float32) / 32768.0
        x = trim(x)
        if kind == "announce":
            x = echo(x)
        m = np.max(np.abs(x)) if len(x) else 0.0
        x = x * (0.8 / m) if m > 0 else x
        path = os.path.join(out_dir, fname)
        sf.write(path, x, RATE, format="OGG", subtype="VORBIS")
        total += os.path.getsize(path)
        print(f"  {fname:28} {len(x) / RATE:4.2f} s  {voice:8} {text}")
    if not args.sample:
        vl.write_rust("tools/voicegen_elevenlabs.py (ElevenLabs v3)")
    print(f"{len(todo)} lines, {total / 1e6:.2f} MB -> {out_dir}")


if __name__ == "__main__":
    sys.exit(main())
