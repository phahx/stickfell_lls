// Particle liquids, drawn in two passes.
//
// 1. splat: every drop adds a soft round bump to a screen-sized field, in
//    the channel of its kind of liquid (r, g, b), and its foaminess to alpha.
// 2. resolve: wherever the field is over the threshold there's liquid. Its
//    colour comes from the kinds mixed there; how far down from the surface
//    a pixel is (looked up through the field) shades it from shallow to deep
//    and puts foam on top. Caustics, bubbles and a cracked crust come in as
//    each kind's look asks. It's blended over the scene's albedo and
//    emissive, so whatever is in it shows through, tinted, and gets lit.

// ---- resolve ----

struct Look {
    shallow: vec4<f32>,
    deep: vec4<f32>,
    foam: vec4<f32>,
    // depth, foam width, glow, sparkle
    a: vec4<f32>,
    // caustics, bubbles, crust, flow
    b: vec4<f32>,
    // seed, glow when dyed, how fast a little dye shows, unused
    c: vec4<f32>,
    dyed_shallow: vec4<f32>,
    dyed_deep: vec4<f32>,
    // unused (keeps it at nine vec4s, as the renderer writes it)
    pad: vec4<f32>,
};

struct Liquids {
    looks: array<Look, 3>,
    // camera centre, half its view
    view: vec4<f32>,
    // time, threshold, world units per pixel, unused
    params: vec4<f32>,
};

@group(0) @binding(0) var field: texture_2d<f32>;
@group(0) @binding(1) var field_samp: sampler;
@group(0) @binding(2) var<uniform> liq: Liquids;
@group(0) @binding(3) var dye_field: texture_2d<f32>;

struct FullOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> FullOut {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    var o: FullOut;
    o.clip = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, y);
    return o;
}

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q = q + dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    return vnoise(p) * 0.55 + vnoise(p * 2.03 + vec2<f32>(5.2, 1.3)) * 0.3 + vnoise(p * 4.11 + vec2<f32>(2.7, 9.1)) * 0.15;
}

fn bubbles(p: vec2<f32>, t: f32, density: f32, seed: f32, px: f32) -> f32 {
    let q = vec2<f32>(p.x * 5.0, p.y * 5.0 - t * 1.6);
    let cell = floor(q);
    let h = hash21(cell + seed);
    if h > density {
        return 0.0;
    }
    let wob = vec2<f32>(sin(t * 3.0 + h * 40.0) * 0.12, 0.0);
    let r = 0.12 + 0.18 * hash21(cell + 7.7 + seed);
    let d = length(fract(q) - vec2<f32>(0.5) - wob);
    let w = px * 5.0;
    let disc = clamp((r - d) / w + 0.5, 0.0, 1.0);
    let ring = disc * clamp((d - (r - max(0.05, 2.0 * w))) / w + 0.5, 0.0, 1.0);
    return ring * 0.85 + disc * 0.25;
}

fn total(f: vec4<f32>) -> f32 {
    return f.x + f.y + f.z;
}

// How far (m) a gap in the liquid can reach and still be a hole something
// in it has made (a body, a blade): the widest a body part clears is the
// head's, 0.27 m from its middle.
const HOLE: f32 = 0.4;

// Is there liquid at `uv`?
fn wet(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(field, field_samp, uv, 0.0);
}

// The liquid round a gap at `uv`, if it's shut in by it above and to both
// sides within HOLE (whatever's below: the floor will do): the field there,
// averaged; or zero where the gap is open (to the air above, or a wall).
fn closing(uv: vec2<f32>, per_m: vec2<f32>, thr: f32) -> vec4<f32> {
    // up, the two sides, the two upper diagonals (uv's y runs down)
    let dirs = array<vec2<f32>, 5>(vec2<f32>(0.0, -1.0), vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(-0.7071, -0.7071), vec2<f32>(0.7071, -0.7071));
    var sum = vec4<f32>(0.0);
    for (var d = 0; d < 5; d = d + 1) {
        var hit = vec4<f32>(0.0);
        for (var i = 1; i <= 5; i = i + 1) {
            let s = wet(uv + dirs[d] * per_m * (HOLE * f32(i) / 5.0));
            if total(s) >= thr {
                hit = s;
                break;
            }
        }
        if total(hit) < thr {
            return vec4<f32>(0.0);
        }
        sum = sum + hit;
    }
    return sum / 5.0;
}

struct SceneOut {
    @location(0) albedo: vec4<f32>,
    @location(1) emissive: vec4<f32>,
};

@fragment
fn fs_resolve(o: FullOut) -> SceneOut {
    var f = textureSampleLevel(field, field_samp, o.uv, 0.0);
    var tot = total(f);
    let thr = liq.params.y;
    let aa = max(fwidth(tot), 1e-3);
    var cov = clamp((tot - thr) / aa + 0.5, 0.0, 1.0);
    // uv per metre
    let per_m = vec2<f32>(0.5 / liq.view.z, 0.5 / liq.view.w);
    // at (or near) a gap: is it a hole something in the liquid has pushed
    // out? Then it's in the liquid, seen through it, not a bubble with a
    // rim of foam round it
    var shut = false;
    if tot < thr + aa * 2.5 {
        let round = closing(o.uv, per_m, thr);
        if total(round) >= thr {
            shut = true;
            if cov < 1.0 {
                f = round;
                tot = total(f);
                cov = 1.0;
            }
        }
    }
    if cov <= 0.0 {
        discard;
    }
    let t = liq.params.x;
    let px = liq.params.z;
    let p = liq.view.xy + vec2<f32>((o.uv.x * 2.0 - 1.0) * liq.view.z, (1.0 - o.uv.y * 2.0) * liq.view.w);
    // the kinds mixed here
    let wk = f.xyz / max(tot, 1e-5);
    let l0 = liq.looks[0];
    let l1 = liq.looks[1];
    let l2 = liq.looks[2];
    let shallow = l0.shallow * wk.x + l1.shallow * wk.y + l2.shallow * wk.z;
    let deep = l0.deep * wk.x + l1.deep * wk.y + l2.deep * wk.z;
    let foamc = l0.foam * wk.x + l1.foam * wk.y + l2.foam * wk.z;
    let la = l0.a * wk.x + l1.a * wk.y + l2.a * wk.z;
    let lb = l0.b * wk.x + l1.b * wk.y + l2.b * wk.z;
    let seed = l0.c.x * wk.x + l1.c.x * wk.y + l2.c.x * wk.z;
    // how far down from the surface (up through the field to where it
    // ends: past holes things in it have made)
    let up = 0.06 * per_m.y;
    var depth = 0.6;
    var dry = 0;
    for (var i = 1; i <= 17; i = i + 1) {
        let s = wet(o.uv - vec2<f32>(0.0, up * f32(i)));
        if total(s) < thr {
            dry = dry + 1;
            if f32(dry) * 0.06 > HOLE {
                depth = min((f32(i - dry) + 0.5) * 0.06, 0.6);
                break;
            }
        } else {
            dry = 0;
            if f32(i) * 0.06 >= 0.6 {
                break;
            }
        }
    }
    let k = clamp(depth / max(la.x, 1e-3), 0.0, 1.0);
    let ks = k * k * (3.0 - 2.0 * k);
    // dyed: as much of the liquid here as carries it
    // (each kind its own dyed colours, and how soon a little shows)
    let share = clamp(textureSampleLevel(dye_field, field_samp, o.uv, 0.0).x / max(tot, 1e-5), 0.0, 1.0);
    let curve = l0.c.z * wk.x + l1.c.z * wk.y + l2.c.z * wk.z;
    let dye = 1.0 - exp(-curve * share);
    let dsh = l0.dyed_shallow * wk.x + l1.dyed_shallow * wk.y + l2.dyed_shallow * wk.z;
    let ddp = l0.dyed_deep * wk.x + l1.dyed_deep * wk.y + l2.dyed_deep * wk.z;
    let dglow = l0.c.y * wk.x + l1.c.y * wk.y + l2.c.y * wk.z;
    let sh = mix(shallow, dsh, dye);
    let dp = mix(deep, ddp, dye);
    var col = mix(sh.rgb, dp.rgb, ks);
    var a = mix(sh.a, dp.a, ks);
    let flow = lb.w;
    // drifting murk
    let murk = fbm(vec2<f32>(p.x + t * 0.15 * flow, p.y) * 1.3 + seed);
    col = col * (0.88 + 0.24 * murk);
    var bright = 0.0;
    if lb.x > 0.0 {
        let n1 = vnoise(p * 3.4 + vec2<f32>(t * 0.35, t * 0.2) + seed);
        let n2 = vnoise(p * 4.3 - vec2<f32>(t * 0.27, -t * 0.31) + seed * 1.7);
        let line = 1.0 - clamp(abs(n1 - n2) / 0.035, 0.0, 1.0);
        let c = line * line * lb.x * exp(-depth * 1.1);
        col = col + vec3<f32>(c) * 0.6;
        bright = bright + c * 0.4;
    }
    if lb.y > 0.0 {
        let b = bubbles(p, t * flow, lb.y, seed, px);
        col = mix(col, foamc.rgb, b * 0.7);
        bright = bright + b;
    }
    if lb.z > 0.0 {
        let cp = vec2<f32>(p.x * 1.4 + t * 0.08 * flow, p.y * 1.9 - t * 0.04);
        let n = fbm(cp + seed);
        let reach = clamp(1.3 - depth * 0.7, 0.35, 1.0);
        let cover = clamp((n - 0.44) / 0.06, 0.0, 1.0) * lb.z * reach;
        let seam = (1.0 - clamp(abs(n - 0.44) / 0.02, 0.0, 1.0)) * lb.z * reach;
        col = mix(col, deep.rgb * 0.12, cover);
        col = mix(col, foamc.rgb, seam * 0.8);
        bright = bright + seam - cover;
    }
    // foam: along the top, and wherever it's churned up (fast drops)
    let churn = clamp(f.w / max(tot, 1e-3), 0.0, 1.0);
    let fw = max(la.y, px);
    let broken = 0.65 + 0.35 * vnoise(vec2<f32>(p.x * 9.0 + t * 0.9 * flow, p.y * 9.0 + t * 0.7) + seed);
    let top = (1.0 - clamp(depth / fw, 0.0, 1.0)) * broken;
    let edge = select(1.0 - clamp((tot - thr) / (aa * 2.5), 0.0, 1.0), 0.0, shut);
    let fo = clamp(max(max(top * top, edge * 0.8), churn * broken) * foamc.a, 0.0, 1.0);
    col = mix(col, foamc.rgb, fo);
    a = max(a, fo);
    bright = bright + fo * 0.5;
    a = a * cov;
    let glow = mix(la.z, dglow, dye) * max(0.0, 1.0 + bright * la.w * (1.0 - dye));
    var out: SceneOut;
    out.albedo = vec4<f32>(col, a);
    out.emissive = vec4<f32>(col * glow, a);
    return out;
}
