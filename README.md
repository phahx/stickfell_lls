# Stickfell: Last Limb Standing

An original, online-only stick-figure ragdoll fighting game. You swing
physical weapons with your mouse, in the spirit of Half Sword and Blade &
Sorcery but in 2D. It runs on its own small Rust engine: deterministic
active-ragdoll physics, dynamic lights with real-time shadows, rollback
netcode, and a relay server with room codes.

```
cargo run --release -p stickfell_relay            # 1. a relay (any machine both players can reach)
cargo run --release -p stickfell -- --relay HOST:7777    # 2. everyone: HOST GAME or JOIN GAME + code
```

## Layout

| crate | what it does |
|---|---|
| `stickfell_physics` | Small-step XPBD: particles, distance constraints (toggleable, used for grips, grabs and severing), hinge limits/motors, world-angle **muscles** (damped, speed-capped), hand **drives**, shape-matched **rigid bodies**, broadphased capsule-vs-capsule contacts with impact speed, capsule- and particle-vs-oriented-box statics, ray casts. `dmath` has deterministic `sin/cos/atan2` and an RNG. |
| `stickfell_net` | Online play: `RelayServer`/`LobbyClient`/`RelayTransport` (rooms with 4-letter codes; players only talk to the relay, so no port forwarding). Rollback netcode: `P2PSession` (input delay, 8-frame prediction, snapshot/restore by `Clone`, resimulation, time sync, disconnect and checksum desync detection), `SyncTestSession` (rolls back every frame and checks determinism), `UdpTransport`, and `SimNetwork` (seeded latency/jitter/loss for tests). |
| `stickfell_render` | wgpu renderer. Anti-aliased SDF circles/capsules/rounded boxes go into albedo and emissive targets. A half-res HDR light buffer takes point/spot lights with **soft shadows**: each shadow-casting light gets a row of a 1D polar shadow map (distance to the nearest occluder in 2048 directions, occluders being boxes, segments and circles), plus nearest-blocker copies built by a compute pass; each pixel estimates how far it is past the blocker and averages over a penumbra that widens with that distance and the light's source `size` (sharp at contact, soft further out). **Contact shadows** (ambient occlusion) darken ambient light, and partly direct light: each level's boxes are baked once by a compute pass (32 rays per 5 cm texel; the renderer remembers bakes by the level's `Arc<StaticScene>`), times soft darkening around moving limbs, heads and weapons each frame. Every pipeline except the UI's compiles on a background thread (`Renderer::preloading`), so the window shows a loading screen at once; the game's loading screen also decodes art and sound off the main thread and shows each level once, hidden, to bake it. Keep shaders friendly to DirectX's FXC compiler (no big shared-memory arrays or fixed loops it will unroll): one such compute shader once took 13 s to compile on DX12. Then emissive bloom, an ACES-tonemapped composite, and unlit UI text from a distance-field font atlas. |
| `stickfell_app` | winit runner: fixed 60 Hz ticks, keyboard, mouse and text input, cursor control, and `capture_headless` for PNG screenshots without a window. |
| `stickfell_relay` | The relay server binary. |
| `game` (`stickfell`) | Fighters, weapons, arenas, rounds, menus/lobby, HUD. |

### The determinism contract

Everything that runs inside `RollbackGame::step` must be bit-identical on every
peer:

* Use only `+ - * / sqrt floor abs min max` on `f32`. For trig, use
  `stickfell_physics::dmath` and never `f32::sin/cos/atan2/powf`, which come from
  the platform math library and can differ between OSes and CPUs.
* Keep all state in the cloned `GameState`, with no globals, no wall-clock time,
  and no `HashMap` iteration.
* Bots and other input sources may read the state, but they run outside `step`.

`stickfell --verify N` and the `rollback_resimulation_is_deterministic` test run
real matches under `SyncTestSession`. That session rolls back 7–8 frames every
frame and compares checksums, so most determinism bugs show up there before they
can desync an online match.

## Playing

Stickfell is online only: there's no couch or shared-keyboard mode. The one
offline mode is **Practice**, a training yard with every weapon on a rack,
three harmless dummies to try things on, and a rope course: wall-jump up the
chimney on the left, then swing across to the yellow platform.

**How it plays**

* Fighters are floppy active ragdolls held up by weak muscles. Big hits, or
  simply falling over, knock you down limp until you wobble back up.
* **Your mouse aims your weapon.** The hand follows the cursor around your
  shoulder at a steady pace, so waving the mouse swings the weapon. **Left
  click** swings: a wind-up and a hard strike toward the cursor (spears and
  fists stab instead). **Right click** stabs: a lunge that reaches well past
  arm's length. Heavy weapons turn slower and pull on your body.
* **Impaling:** a fast stab that lands with a sword or spear tip runs them
  through. They go limp and hang on the blade, sliding along it. Click to
  swing and fling them off, or point the blade down to let them slide off.
  They come off by themselves after 3 seconds.
* Your own weapon never hurts you, and dropped weapons don't hurt anyone
  (thrown ones do).
* **Hits land hard.** A solid hit stops you dead and throws you back, upper
  body first; big ones (and anything thrown) ragdoll you for a moment, lighter
  ones make you stagger.
* Damage comes from **impact speed × weapon heft**, with head hits counting
  extra. Blades can **sever limbs and heads**, blunt weapons send people
  flying, and blood splatters stick to the level.
* **Finishers:** a hit that leaves someone at 40 health or less dazes them for
  4 seconds (stars, slow and wobbly) and puts a skull over them. Walk up and
  press F for your weapon's finisher (sword beheads, or with a wall at their
  back shoves them against it and pins them there through the chest, spear either
  harpoons them from a few steps back or runs them through, lifts them high and
  slams them down, sledgehammer flattens, flail stands them up and launches them, fish slaps silly,
  frying pan bonks, revolver executes, knives shank, bare hands either tear them apart limb by limb or rip out their head and
  spine and hold it up). The grapple gun pushes them to their knees and
  ties its rope round their neck instead: fire the hook (left click) up at a
  ledge or ceiling and they're dragged along behind it (limp, and nothing can
  hurt them while it flies). It bites, reels them up off the ground, and they
  claw at the rope and slowly choke. Cut the rope with a blade (or shoot it)
  and they drop, saved. A hook that hits too low, or nothing, flies back to
  the gun with them still tied; fire again.
  The camera pushes in with cinematic bars, and the final blow drops the
  whole match into a moment of slow motion. The victim begs, a ring
  announcer calls the move, and the winner gets a one-liner. Getting hit
  mid-finisher interrupts it.
* **Blade locks:** when two swords meet just right, mid-swing and crossed, they
  lock together and the camera swoops in on the pair. A key (W, A, S or D)
  flashes up; whoever presses it first takes the point, and a wrong key locks
  you out for a moment. First to 3 throws the other's sword aside and sends
  them flying. Everyone else can keep fighting (and can stab the pair).
* **Stuck blades:** a hard stab point-first into the floor or a wall (or a
  thrown spear) sticks there. You're stuck holding it until you let go (E) or
  wrench it out by pulling or swinging hard. Stab a wall in mid-air and you
  hang off it: W pulls you up and takes the blade with you, so you can stab
  again higher and climb.
* **Bare hands:** left click jabs, and clicking again as it lands throws the
  other fist. S + click is a low sweep that takes their legs out; clicking in
  mid-air is a dropkick that flattens them and you. Grab someone (right
  click) and you can drag them about, then Q throws them, and slamming into
  a wall or the floor hurts. Grab their weapon hand instead and Q takes the
  weapon off them. Getting hit makes you let go, and anyone held can mash
  A / D to break free. Grab in mid-air near someone and you dive at them
  head first; catch their head and you flip right over them, swinging them
  end over end and flinging them on. Press Shift as you come over and you
  whip them away hard wherever you're aiming.
* **Cut-off arms and legs** are weapons too: pick one up and swing it.
* **Weapons:** sword, sledgehammer (two-handed), spear (two-handed), flail
  (swinging chain), fish (floppy, slaps hard), frying pan, a revolver (six
  shots, then throw it), and a bundle of three throwing knives (Q throws one
  at a time, and they spin end over end), and a grapple gun. Some lie around at round start and more drop in
  every 7 s.
* **Crouch** drops you into a deep squat (about half your height) that still scuttles along quickly; crouching while running slides you feet first. The mouse stays inside the window during a fight.
* **Double jump:** one extra jump in mid-air, topped up whenever you land
  (or hang from a ledge or a stuck blade).
* **Wall jumps:** jump again while touching a wall, up to twice before you
  land (pushing into a wall slows your fall). Nothing climbs or steps up for
  you; grab a ledge (Shift) and jump to pull up.
* **Free hand (Shift, or right click when unarmed):** grab people to drag them
  around, grab a ledge to hang, or grab a rope or lamp chain to swing on it
  (A/D pumps the swing, S slides down, jump lets go).
* Four arenas with a follow camera: Foundry, Rooftops, The Keep (with a
  spiked moat) and the Gauntlet (up to eight players, every hazard). Last
  one standing takes the round.

| | |
|---|---|
| move / jump / crouch (slide when running) | A D / Space / S |
| aim weapon arm | mouse |
| swing (spear: stab) | left click |
| stab (impales) | right click |
| grab with free hand | hold Left Shift (or right click when unarmed) |
| pick up / swap / drop | E |
| throw weapon / the person you're holding | Q |
| punch / leg sweep / dropkick (unarmed) | click / S + click / click in the air |
| shoot (revolver) | left click |
| finish someone (skull over them) | F |
| reset the practice yard | R |
| mute | M |
| leave match | Esc (asks first) |

**Hosting online**

1. Run `stickfell_relay` somewhere reachable. A $5 VPS is plenty, since it only
   forwards packets (UDP 7777).
2. Everyone starts `stickfell --relay that.host:7777`.
3. One player clicks **HOST GAME** and shares the 4-letter code. Others click
   **JOIN GAME** and type it. The host clicks **START MATCH** (2–4 players).

**Dev tools**

```
stickfell --host --autostart 2 --bot    # skip menus; a CPU drives your fighter (netcode soak test)
stickfell --join ABCD --bot
stickfell --net --player 0 --bind 0.0.0.0:7000 --peer <ip>:7001   # direct IP, no relay
stickfell --verify 3600 --players 4 --level 2   # headless CPU match under per-frame rollback checking
stickfell --screenshot shot.png --ticks 600 --players 4 --level 1 [--size 1920x1080] [--menu]
stickfell --bench-render 300 --size 3840x2160 --level 2   # GPU time per frame
STICKFELL_PROFILE=1 stickfell ...                    # log fps and per-frame acquire/render/present
cargo test --workspace
cargo test --release -p stickfell feel -- --nocapture   # scripted feel checks: jitter, aim lag, swing speed, climbing
```

## Character

The fighters are built from a character kit
(`S:/asset-art/stickfell-character-base`: `stickfigure-rig.json` plus one PNG
per body part). `uv run tools/make_character.py [KIT_FOLDER]` bakes it into
the game: the part textures (`game/assets/character/atlas.png`, tinted per
player) and the body they describe (`game/src/character_data.rs`; the legs
keep the game's length, the rest follows the art). Joints have realistic
limits (elbows and knees bend one way). Poses that reach or hold things were
tuned on the original stick figure and scale with the build.

Weapon art goes in `game/assets/weapons/<weapon>.png` (drawn at the
character's scale, about 420 px per metre, pointing right) with a
`<weapon>.json` beside it giving the `grip` and `tip` markers in the PNG's
pixels; the same bake packs it into the atlas. Weapons without art are drawn
as plain shapes. Lopsided weapons (guns) flip over in the hand instead of
being held upside down. The art workspace is `S:/asset-art/stickfell-weapons`.

### Character lab

A separate program for trying a new kit before baking it in:

```
cargo run --release -p stickfell_character_lab -- [KIT_FOLDER]
```

It runs the game in the training yard (or any arena) with the kit's art and
proportions, and reloads when the files change. Keys: 1 art (or plain
capsules), 2 faces, 3 outline, 4 thickness (hitboxes or the art's own),
5 body (the kit's or the original stick figure's), 6 joint limits, Z zoom,
L level, B dummies or CPU opponents, R reset, F5 reload, Esc twice to quit.

## Roadmap ideas

* More weapons (bows and guns via deterministic raycasts), armor that
  deflects blades, and throwable props built as particle bodies.
* Moving and falling level pieces (kinematic statics) and level hazards.
* Audio driven by `GameState::hits` events (bonks, clangs, squelches).
* Public matchmaking and region relays, or Steam networking behind `Transport`.
* Rejoining mid-match (send a state snapshot to the late joiner).
* Spectators, and replays (store inputs, re-run the deterministic sim).
* Soft penumbra shadows, normal-mapped sprites, and a texture/sprite path in
  the renderer.

Sound effects are from the *Rust & Blood* SFX library and *ROT - Horror Audio Bundle* by Pizza Doggy
(pizzadoggy.itch.io), used under its game license.

Voice lines are rendered at build time and only the audio ships. The lines
live in `tools/voicelines.py`; `uv run tools/voicegen.py` renders them offline
with the [Kokoro](https://huggingface.co/hexgrad/Kokoro-82M) model (Apache-2.0),
and `uv run tools/voicegen_elevenlabs.py` renders acted versions with
ElevenLabs v3 (needs `ELEVENLABS_API_KEY`; a dry run shows the cost first).

The fonts are FreeCheese (titles) and Jupiteroid (everything else), both by
GGBotNet (CC0). `uv run tools/make_font.py` bakes them
into the renderer's distance-field atlas, and `uv run tools/make_icon.py`
builds the app icon from `game/assets/icon/icon-source.png`.
