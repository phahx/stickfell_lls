# /// script
# requires-python = ">=3.10"
# dependencies = ["soundfile>=0.12", "numpy"]
# ///
"""Imports voice lines you recorded yourself (e.g. downloaded from
ElevenLabs) into the game, replacing whatever was there.

    uv run tools/voiceimport.py FOLDER [FOLDER ...]

Files must be named as on the voice script (announce_spin, p0_plea_3,
p0_quip_spine_0, ...; hanging: p0_hanging_choke_0, _1, ... looped, and
p0_hanging_choke_end for the death; cries of pain: p0_pain_0, _1, ...), in any format libsndfile reads (MP3, WAV, OGG, FLAC).
Extra takes like p0_quip_spine_2 are fine. Each clip gets its silence
trimmed and its level evened out, is saved as OGG in game/assets/voice, and
game/src/voice.rs is rewritten to list exactly what's there.
"""

import os
import re
import sys

import numpy as np
import soundfile as sf

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import voicelines as vl  # noqa: E402

NAME = re.compile(r"^(announce_(%s)|p\d+_(plea_\d+|pain_\d+|quip_any_\d+|quip_(%s)_\d+|hanging_choke_(\d+|end)))$" % ("|".join(vl.FINISHERS), "|".join(vl.FINISHERS)))
AUDIO = (".mp3", ".wav", ".ogg", ".flac")
FIN = "|".join(vl.FINISHERS)


def normalize(stem):
    """Near-miss names mean the obvious line: announce_hang_0 is
    announce_hang, p0_hang_1 is p0_quip_hang_1."""
    if m := re.fullmatch(r"announce_(%s)_\d+" % FIN, stem):
        return f"announce_{m.group(1)}"
    if m := re.fullmatch(r"(p\d+)_(%s)_(\d+)" % FIN, stem):
        return f"{m.group(1)}_quip_{m.group(2)}_{m.group(3)}"
    return stem


def trim(x, rate):
    """Cuts the silence off both ends (keeps a little air)."""
    env = np.abs(x)
    loud = np.nonzero(env > max(0.02 * env.max(), 1e-4))[0]
    if len(loud) == 0:
        return x
    return x[max(0, loud[0] - rate // 50) : min(len(x), loud[-1] + rate // 12)]


def level(x, target_rms=0.12, peak=0.9):
    """Evens out loudness between clips without clipping."""
    rms = float(np.sqrt(np.mean(x**2))) if len(x) else 0.0
    if rms <= 0:
        return x
    g = min(target_rms / rms, peak / float(np.abs(x).max()))
    return x * g


def main():
    folders = sys.argv[1:]
    if not folders:
        sys.exit(__doc__)
    found = {}
    for d in folders:
        for f in sorted(os.listdir(d)):
            stem, ext = os.path.splitext(f)
            if ext.lower() not in AUDIO:
                continue
            if normalize(stem) != stem:
                print(f"  {f}: taken as {normalize(stem)}")
                stem = normalize(stem)
            if not NAME.match(stem):
                print(f"  skipped {f}: not a name from the voice script")
                continue
            found[stem] = os.path.join(d, f)
    if not found:
        sys.exit("no voice lines found")
    os.makedirs(vl.OUT_DIR, exist_ok=True)
    for f in os.listdir(vl.OUT_DIR):
        if f.endswith((".ogg", ".wav")):
            os.remove(os.path.join(vl.OUT_DIR, f))
    for stem, path in sorted(found.items()):
        x, rate = sf.read(path, dtype="float32", always_2d=True)
        x = level(trim(x.mean(axis=1), rate))
        sf.write(os.path.join(vl.OUT_DIR, stem + ".ogg"), x, rate, format="OGG", subtype="VORBIS")
        print(f"  {stem:26} {len(x) / rate:4.2f} s")
    vl.write_rust("your recordings, tools/voiceimport.py")


if __name__ == "__main__":
    main()
