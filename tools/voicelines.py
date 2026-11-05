"""Stickfell's finisher voice lines, shared by the voice generators, plus the
code that writes game/src/voice.rs from the rendered clips.

[Bracketed] parts are acting directions: ElevenLabs v3 performs them,
Kokoro (which can't) has them stripped.
"""

import os
import re

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT_DIR = os.path.join(ROOT, "game", "assets", "voice")
RUST_OUT = os.path.join(ROOT, "game", "src", "voice.rs")

PLAYER_SLOTS = 4
NL = chr(10)

# in `Finish` order
FINISHERS = ["behead", "javelin", "lift", "smash", "spin", "slap", "bonk", "execute", "shank", "dismember", "spine", "hang", "pin", "stomp"]

ANNOUNCE = {
    "behead": "[booming announcer voice] BEHEADED!",
    "javelin": "[booming announcer voice] HARPOONED!",
    "lift": "[booming announcer voice] SKEWERED!",
    "smash": "[booming announcer voice] FLATTENED!",
    "spin": "[booming announcer voice] LAUNCHED!",
    "slap": "[booming announcer voice] SLAPPED... SILLY!",
    "bonk": "[booming announcer voice] BONK!",
    "execute": "[booming announcer voice] EXECUTED!",
    "shank": "[booming announcer voice] SHANKED!",
    "dismember": "[booming announcer voice] TORN... APART!",
    "spine": "[booming announcer voice] SPINE... RIPPED!",
    "hang": "[booming announcer voice] STRUNG... UP!",
    "pin": "[booming announcer voice] PINNED!",
    "stomp": "[booming announcer voice] CURB... STOMPED!",
}

QUIPS = {
    "behead": ["[smug] Heads up.", "[chuckles] Lost your head there."],
    "javelin": ["[shouting] Catch!", "[sarcastic] Think fast."],
    "lift": ["[gleeful] Going up!", "[laughs] And down you go."],
    "smash": ["[deadpan] Pancake.", "[smug] Flat as a stick."],
    "spin": ["[cheerfully] Bon voyage!", "[mocking] Write when you land."],
    "slap": ["[proudly] Fresh fish. Delivered.", "[sniffs] Smells like victory."],
    "bonk": ["[deadpan] Bonk.", "[whispers] Go to sleep."],
    "execute": ["[cold] Nothing personal.", "[deadpan] Bang."],
    "shank": ["[laughs] Shanks for playing.", "[playfully] Pokey pokey."],
    "dismember": ["[sarcastic] Need a hand? Keep it.", "[smug] Some assembly required."],
    "spine": ["[evil laugh] I'm keeping this.", "[mocking] Spineless."],
    "hang": ["[smug] Just hang in there.", "[mocking] Should've learned the ropes."],
    "pin": ["[smug] Stay put.", "[mocking] Wall art."],
    "stomp": ["[smug] Watch your step.", "[deadpan] Mind the curb."],
}

QUIPS_ANY = [
    "[shouting] Next!",
    "[yawns] Too easy.",
    "[sighs] Clean up on aisle stick.",
    "[proudly] That's going on the fridge.",
    "[mocking] Should have stretched.",
    "[laughs] Who's next?",
]

PLEAS = [
    "[panicked] Wait, wait, wait, wait!",
    "[nervous laugh] Can we talk about this?",
    "[desperate] I yield! I yield!",
    "[screaming] Not the face! Not the face!",
    "[sobbing] I have a family! They're also stick figures!",
    "[hopefully] Best two out of three?",
    "[panicked, talking fast] This is all a big misunderstanding!",
    "[whimpering] Okay, okay, you win!",
    "[screaming] Mommy!",
    "[nervously] I was only stretching!",
    "[pleading] Please, I'm mostly lines!",
]


def plain(text):
    """The line without its acting directions."""
    return re.sub(r"\[[^\]]*\]\s*", "", text).strip()


def jobs():
    """Every clip to render: (file name, kind, player slot or None, text)."""
    out = [(f"announce_{f}.ogg", "announce", None, ANNOUNCE[f]) for f in FINISHERS]
    for p in range(PLAYER_SLOTS):
        for f in FINISHERS:
            out += [(f"p{p}_quip_{f}_{i}.ogg", "quip", p, t) for i, t in enumerate(QUIPS[f])]
        out += [(f"p{p}_quip_any_{i}.ogg", "quip", p, t) for i, t in enumerate(QUIPS_ANY)]
        out += [(f"p{p}_plea_{i}.ogg", "plea", p, t) for i, t in enumerate(PLEAS)]
    return out


def write_rust(made_by):
    """Writes game/src/voice.rs from the clips actually in game/assets/voice
    (any subset: missing lines are simply left out, extra takes such as
    p0_quip_spine_2.ogg are picked up). Player slots with no clips of their
    own share the recorded ones (the game picks speaker % slots)."""
    files = set(f for f in os.listdir(OUT_DIR) if f.endswith(".ogg")) if os.path.isdir(OUT_DIR) else set()
    inc = lambda f: f'include_bytes!("../assets/voice/{f}")'
    lst = lambda fs: "&[" + ", ".join(inc(f) for f in fs) + "]"

    def takes(prefix):
        """prefix0.ogg, prefix1.ogg, ... in order (gaps allowed)."""
        found = [f for f in files if re.fullmatch(re.escape(prefix) + r"\d+\.ogg", f)]
        return sorted(found, key=lambda f: int(re.search(r"(\d+)\.ogg$", f).group(1)))

    slots = sorted({int(m.group(1)) for f in files if (m := re.match(r"p(\d+)_", f))})
    announce = [[f"announce_{f}.ogg"] if f"announce_{f}.ogg" in files else [] for f in FINISHERS]
    quips = [[takes(f"p{p}_quip_{f}_") for f in FINISHERS] for p in slots]
    quips_any = [takes(f"p{p}_quip_any_") for p in slots]
    pleas = [takes(f"p{p}_plea_") for p in slots]
    chokes = [takes(f"p{p}_hanging_choke_") for p in slots]
    choke_end = [[f"p{p}_hanging_choke_end.ogg"] if f"p{p}_hanging_choke_end.ogg" in files else [] for p in slots]
    pains = [takes(f"p{p}_pain_") for p in slots]
    burning = [takes(f"p{p}_plea_burning_") for p in slots]
    out = [
        f"//! Finisher voice lines ({made_by}). Generated by tools/voicelines.py",
        "//! from whatever is in assets/voice: don't edit by hand.",
        "",
        "/// The announcer calling each finisher, in `Finish` order (empty: none).",
        f"pub static ANNOUNCE: [&[&[u8]]; {len(FINISHERS)}] = [",
        *[f"    {lst(fs)}," for fs in announce],
        "];",
        "",
        "/// Per recorded voice, per `Finish`: the finisher's one-liners after the kill.",
        f"pub static QUIPS: &[[&[&[u8]]; {len(FINISHERS)}]] = &[",
        *["    [" + NL + "".join(f"        {lst(fs)}," + NL for fs in per) + "    ]," for per in quips],
        "];",
        "",
        "/// Per recorded voice: one-liners that fit any finisher.",
        "pub static QUIPS_ANY: &[&[&[u8]]] = &[",
        *[f"    {lst(fs)}," for fs in quips_any],
        "];",
        "",
        "/// Per recorded voice: begging as they're grabbed.",
        "pub static PLEAS: &[&[&[u8]]] = &[",
        *[f"    {lst(fs)}," for fs in pleas],
        "];",
        "",
        "/// Per recorded voice: choking while strung up (looped, one after another).",
        "pub static CHOKES: &[&[&[u8]]] = &[",
        *[f"    {lst(fs)}," for fs in chokes],
        "];",
        "",
        "/// Per recorded voice: the last choke and breath out as they die hanging.",
        "pub static CHOKE_END: &[&[&[u8]]] = &[",
        *[f"    {lst(fs)}," for fs in choke_end],
        "];",
        "",
        "/// Per recorded voice: cries of pain when they're hurt mid-finisher.",
        "pub static PAINS: &[&[&[u8]]] = &[",
        *[f"    {lst(fs)}," for fs in pains],
        "];",
        "",
        "/// Per recorded voice: screaming while burning in acid or lava (looped).",
        "pub static BURNING: &[&[&[u8]]] = &[",
        *[f"    {lst(fs)}," for fs in burning],
        "];",
        "",
    ]
    with open(RUST_OUT, "w", encoding="utf-8", newline=NL) as f:
        f.write(NL.join(out))
    n = sum(len(a) for a in announce) + sum(len(x) for per in quips for x in per) + sum(map(len, quips_any)) + sum(map(len, pleas)) + sum(map(len, chokes)) + sum(map(len, choke_end)) + sum(map(len, pains)) + sum(map(len, burning))
    print(f"voice.rs: {n} clips, {len(slots)} player voice(s)")
