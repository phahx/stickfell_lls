# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Designs Stickfell's character voices with ElevenLabs voice design, so they
aren't stock voices everyone has heard.

    uv run tools/voicedesign.py previews OUT_DIR [ROLE ...]   # 3 previews per role
    uv run tools/voicedesign.py save ROLE N                   # keep preview N of ROLE

Previews are only kept on ElevenLabs' side for a while; `save` turns the
chosen one into a voice on the account and records its id in
tools/voices.json, which voicegen_elevenlabs.py uses. The API key comes from
ELEVENLABS_API_KEY and is only sent to api.elevenlabs.io.
"""

import base64
import json
import os
import sys
import urllib.error
import urllib.request

API = "https://api.elevenlabs.io"
HERE = os.path.dirname(os.path.abspath(__file__))
VOICES_JSON = os.path.join(HERE, "voices.json")
CACHE_ROOT = os.path.join(os.path.expanduser("~"), ".cache", "stickfell-voicegen")
_OLD_CACHE = os.path.join(os.path.expanduser("~"), ".cache", "scuffle-voicegen")
if os.path.isdir(_OLD_CACHE) and not os.path.exists(CACHE_ROOT):
    os.rename(_OLD_CACHE, CACHE_ROOT)
PENDING = os.path.join(CACHE_ROOT, "design-previews.json")

ROLES = {
    "announcer": (
        "An over-the-top 1980s pro wrestling ring announcer. Middle-aged man with a booming, gravelly chest voice, "
        "theatrical and larger than life, stretching and rolling his words for a roaring arena crowd.",
        "Ladies and gentlemen! In the red corner, weighing in at exactly one stick, he is flat, he is furious, and he "
        "has absolutely no idea what he is doing! Are you ready... for... STICKFELL?!",
    ),
    "squeaky": (
        "A squeaky, nervous, fast-talking little cartoon man. High-pitched and jittery, his voice cracks when he panics, "
        "always one second away from bursting into tears.",
        "Okay, okay, okay, nobody panic! I am totally fine, this is fine, I have never been more fine in my life. "
        "Wait, is that a sword? Why does he have a sword? Somebody hold me!",
    ),
    "brute": (
        "A big, dim, cheerful cartoon brute. Slow, deep and dopey, friendly and a little slurred, laughs at his own jokes "
        "and is very proud of himself.",
        "Heh. Heh heh. I hit him with a fish. A whole fish! He went flying, right over the wall. I am going to do it "
        "again. Can I do it again? I am gonna do it again.",
    ),
    "villain": (
        "A smug, posh British cartoon villain. Nasal and theatrical, with drawn-out vowels, sneering, delighted with "
        "himself, savouring every cruel word.",
        "Oh, how dreadfully unfortunate for you. You came all this way, in that ridiculous little body, only to be "
        "folded like a deck chair. Do send my regards to the floor.",
    ),
}


def key():
    k = os.environ.get("ELEVENLABS_API_KEY", "").strip()
    if not k:
        sys.exit("set ELEVENLABS_API_KEY")
    return k


def call(path, body):
    req = urllib.request.Request(API + path, data=json.dumps(body).encode(), headers={"xi-api-key": key(), "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=180) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        sys.exit(f"ElevenLabs {path}: HTTP {e.code}: {e.read()[:400].decode(errors='replace')}")


def load(path):
    if os.path.exists(path):
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    return {}


def save_json(path, data):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2)


def previews(out_dir, roles):
    os.makedirs(out_dir, exist_ok=True)
    pending = load(PENDING)
    for role in roles or ROLES:
        desc, text = ROLES[role]
        r = call("/v1/text-to-voice/design", {"voice_description": desc, "text": text, "model_id": "eleven_ttv_v3"})
        ids = []
        for n, p in enumerate(r["previews"], 1):
            ext = "mp3" if "mpeg" in p.get("media_type", "audio/mpeg") else "wav"
            path = os.path.join(out_dir, f"{role}_{n}.{ext}")
            with open(path, "wb") as f:
                f.write(base64.b64decode(p["audio_base_64"]))
            ids.append(p["generated_voice_id"])
            print(f"  {path}")
        pending[role] = ids
        save_json(PENDING, pending)


def keep(role, n):
    gid = load(PENDING).get(role, [])[int(n) - 1]
    desc, _ = ROLES[role]
    r = call("/v1/text-to-voice", {"voice_name": f"Stickfell {role}", "voice_description": desc, "generated_voice_id": gid})
    voices = load(VOICES_JSON)
    voices[role] = r["voice_id"]
    save_json(VOICES_JSON, voices)
    print(f"saved {role}: {r['voice_id']}")


if __name__ == "__main__":
    if len(sys.argv) >= 3 and sys.argv[1] == "previews":
        previews(sys.argv[2], sys.argv[3:])
    elif len(sys.argv) == 4 and sys.argv[1] == "save":
        keep(sys.argv[2], sys.argv[3])
    else:
        sys.exit(__doc__)
