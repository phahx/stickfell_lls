# /// script
# requires-python = ">=3.10"
# dependencies = ["soundfile>=0.12", "numpy"]
# ///
"""Imports environment sounds (recorded or downloaded) into the game, each
cut and levelled for the job it does. Only the files named below are taken;
nothing else in the folder (or in game/assets/sfx) is touched.

    uv run tools/envimport.py FOLDER

One-shots (water_splash, long_slide_friction_sound) are cut down to the
sound itself and matched in loudness to the game's own sounds that play in
the same places. Loops (calm_water_loop, bubbling_lava, underwater_drone,
p0_action_tread_water_0) are all stored equally loud (the game mixes them),
and where the end doesn't run smoothly back into the start it's crossfaded
into it so the loop never clicks. Everything is saved mono (the game places
it left/right itself) as OGG in game/assets/sfx, prefixed env_.
"""

import os
import sys

import numpy as np
import soundfile as sf

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SFX = os.path.join(ROOT, "game", "assets", "sfx")

# Loudness is measured over the part of a sound that's actually sounding:
# 10 ms windows within ACTIVE_DB of the loudest one.
WINDOW = 0.01
ACTIVE_DB = 30.0
# Every loop is stored this loud (RMS, dBFS); the game sets how loud each plays.
LOOP_RMS_DB = -24.0
# Never louder than this at any instant (dBFS).
PEAK_DB = -1.0
# A loop whose end jumps to its start by more than this many times its
# usual sample-to-sample step gets crossfaded, over CROSSFADE seconds.
SEAM_RATIO = 3.0
CROSSFADE = 1.0
# The peak limiter for one-shots: how far ahead it looks, how fast it lets go (s).
LOOKAHEAD = 0.005
RELEASE = 0.05

# file -> (saved as, what it is, the game's own sounds to match in loudness)
JOBS = {
    "water_splash": ("env_water_splash", "oneshot", ["impact_body_ufx_1.ogg", "impact_body_ufx_2.ogg"]),
    "long_slide_friction_sound": ("env_slide", "slide", ["universal_swing_miss_heavy_ufx_1.ogg", "universal_swing_miss_heavy_ufx_2.ogg"]),
    "calm_water_loop": ("env_water_loop", "loop", None),
    "bubbling_lava": ("env_lava_loop", "loop", None),
    "underwater_drone": ("env_underwater_loop", "loop", None),
    "p0_action_tread_water_0": ("env_tread_water_loop", "loop", None),
}
AUDIO = (".wav", ".mp3", ".ogg", ".flac")
# A floor slide lasts SLIDE_FRAMES (40) frames at 60 a second; the sound
# runs that long and then fades out.
SLIDE_SECONDS = 40 / 60
SLIDE_FADE = 0.12


def db(x):
    return 20.0 * np.log10(max(float(x), 1e-12))


def envelope(x, rate):
    """RMS in WINDOW-long windows."""
    w = max(1, int(rate * WINDOW))
    n = len(x) // w
    return np.sqrt(np.mean(x[: n * w].reshape(n, w) ** 2, axis=1)), w


def active_rms(x, rate):
    env, _ = envelope(x, rate)
    loud = env[env >= env.max() * 10 ** (-ACTIVE_DB / 20)]
    return float(np.sqrt(np.mean(loud**2)))


def mono(path):
    x, rate = sf.read(path, dtype="float32", always_2d=True)
    return x.mean(axis=1), rate


def fade(x, rate, fade_in, fade_out):
    x = x.copy()
    a, b = int(rate * fade_in), int(rate * fade_out)
    if a:
        x[:a] *= np.linspace(0.0, 1.0, a)
    if b:
        x[-b:] *= np.linspace(1.0, 0.0, b)
    return x


def onset_to_tail(x, rate, tail_db):
    """From just before it starts sounding to where it has died away to
    `tail_db` under its loudest."""
    env, w = envelope(x, rate)
    start = int(np.nonzero(env >= env.max() * 10 ** (-ACTIVE_DB / 20))[0][0])
    end = int(np.nonzero(env >= env.max() * 10 ** (-tail_db / 20))[0][-1]) + 1
    pre = int(rate * 0.01)
    return x[max(0, start * w - pre) : min(len(x), end * w)]


def seam(x):
    return abs(float(x[-1] - x[0])), float(np.mean(np.abs(np.diff(x))))


def crossfade_loop(x, rate):
    """Ends the loop CROSSFADE early, its last stretch faded (equal power)
    into its first: the end now runs straight on into the start."""
    n = int(rate * CROSSFADE)
    head, tail = x[:n], x[-n:]
    t = np.linspace(0.0, np.pi / 2, n, endpoint=False)
    out = x[: len(x) - n].copy()
    out[:n] = head * np.sin(t) + tail * np.cos(t)
    return out


def limit(x, rate, ceiling):
    """Holds every peak to `ceiling`: the gain dips just ahead of a peak
    (LOOKAHEAD) and comes back over RELEASE, so it's turned down, not
    clipped."""
    ahead = max(1, int(rate * LOOKAHEAD))
    a = np.abs(x)
    padded = np.concatenate([a, np.zeros(ahead)])
    # the loudest sample in the next `ahead` samples, for every sample
    peak = np.lib.stride_tricks.sliding_window_view(padded, ahead + 1).max(axis=1)[: len(x)]
    want = np.minimum(1.0, ceiling / np.maximum(peak, 1e-9))
    back = np.exp(-1.0 / (rate * RELEASE))
    g = np.empty_like(want)
    cur = 1.0
    for i, w in enumerate(want):
        cur = w if w < cur else w + (cur - w) * back
        g[i] = cur
    return np.clip(x * g, -ceiling, ceiling)


def save(path, x, rate):
    """As OGG, a block at a time (libsndfile's Vorbis encoder falls over
    handed a long recording in one go)."""
    with sf.SoundFile(path, "w", samplerate=rate, channels=1, format="OGG", subtype="VORBIS") as f:
        for i in range(0, len(x), 4096):
            f.write(x[i : i + 4096])


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    src = sys.argv[1]
    found = {}
    for f in sorted(os.listdir(src)):
        stem, ext = os.path.splitext(f)
        if ext.lower() in AUDIO and stem in JOBS:
            found[stem] = os.path.join(src, f)
    if not found:
        sys.exit("none of the environment sounds found")
    for stem, path in found.items():
        name, job, refs = JOBS[stem]
        x, rate = mono(path)
        before = len(x) / rate
        note = ""
        if job == "oneshot":
            x = fade(onset_to_tail(x, rate, 50.0), rate, 0.005, 0.05)
        elif job == "slide":
            x = onset_to_tail(x, rate, 50.0)
            x = fade(x[: int(rate * (SLIDE_SECONDS + SLIDE_FADE))], rate, 0.005, SLIDE_FADE)
        else:
            jump, step = seam(x)
            if jump > SEAM_RATIO * step:
                x = crossfade_loop(x, rate)
                note = f", seam {jump / step:.1f}x its step: crossfaded -> {seam(x)[0] / seam(x)[1]:.1f}x"
            else:
                note = f", seam {jump / step:.1f}x its step: left as it is"
        if refs:
            want = float(np.mean([active_rms(*mono(os.path.join(SFX, r))) for r in refs]))
        else:
            want = 10 ** (LOOP_RMS_DB / 20)
        have = active_rms(x, rate)
        if refs:
            # as loud as the sounds it stands in for, its peaks held down
            # (they're far peakier than the game's own) to PEAK_DB
            for _ in range(4):
                x = limit(x * (want / active_rms(x, rate)), rate, 10 ** (PEAK_DB / 20))
        else:
            x = x * min(want / have, 10 ** (PEAK_DB / 20) / float(np.abs(x).max()))
        save(os.path.join(SFX, name + ".ogg"), x, rate)
        got = active_rms(x, rate)
        print(f"  {stem} -> {name}.ogg: {before:.2f} s -> {len(x) / rate:.2f} s, loudness {db(have):.1f} -> {db(got):.1f} dBFS (wanted {db(want):.1f}), peak {db(np.abs(x).max()):.1f} dBFS{note}")


if __name__ == "__main__":
    main()
