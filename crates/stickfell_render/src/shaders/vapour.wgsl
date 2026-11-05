// Vapour (steam, smoke), and fire: a grid of how thick it is (how hot, for
// fire), drawn as one continuous
// volume over the scene. The grid is sampled smoothly (its cells never
// show), looked up through drifting noise so it curls and rises, broken up
// by finer billows, and given its opacity by how much of it a ray goes
// through (1 - e^-k*thickness, as light through fog). Lit from above: its
// top is bright, its thick middle and underside dimmer. Drawn into the
// scene's albedo before lighting, so the lights (lava glow) light it too.

struct Vapour {
    // camera centre, half its view
    view: vec4<f32>,
    // the grid's corner, its cell size, time
    grid: vec4<f32>,
    // cells across and up, how dense it looks, the most opaque it gets
    size: vec4<f32>,
};

@group(0) @binding(0) var thick_tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> v: Vapour;

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
    return vnoise(p) * 0.5 + vnoise(p * 2.03 + vec2<f32>(5.2, 1.3)) * 0.3 + vnoise(p * 4.11 + vec2<f32>(2.7, 9.1)) * 0.2;
}

// How thick it is at world point `p` (nothing outside the grid).
fn thick(p: vec2<f32>) -> f32 {
    let uv = (p - v.grid.xy) / v.grid.z / v.size.xy;
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return 0.0;
    }
    return textureSampleLevel(thick_tex, samp, uv, 0.0).r;
}

// Smoothed over about a cell, so the grid's corners never show.
fn soft(p: vec2<f32>) -> f32 {
    let c = v.grid.z * 0.55;
    return (thick(p) * 2.0 + thick(p + vec2<f32>(c, 0.0)) + thick(p - vec2<f32>(c, 0.0)) + thick(p + vec2<f32>(0.0, c)) + thick(p - vec2<f32>(0.0, c))) / 6.0;
}

struct SceneOut {
    @location(0) albedo: vec4<f32>,
    @location(1) emissive: vec4<f32>,
};

@fragment
fn fs_vapour(o: FullOut) -> SceneOut {
    let p = v.view.xy + vec2<f32>((o.uv.x * 2.0 - 1.0) * v.view.z, (1.0 - o.uv.y * 2.0) * v.view.w);
    let t = v.grid.w;
    let cell = v.grid.z;
    // nothing anywhere near: done
    if soft(p) < 0.002 && thick(p + vec2<f32>(0.0, -cell)) < 0.002 {
        discard;
    }
    // curling and rising: the field looked up through drifting noise
    let warp = vec2<f32>(fbm(p * 0.45 + vec2<f32>(0.0, -t * 0.35)), fbm(p * 0.45 + vec2<f32>(5.2, 1.3) - vec2<f32>(t * 0.05, t * 0.3)));
    let q = p + (warp - 0.5) * 1.6 * cell;
    var d = soft(q);
    if d < 0.002 {
        discard;
    }
    // billows, rising through it
    let detail = fbm(q * 1.6 + vec2<f32>(t * 0.08, -t * 0.55));
    d = d * (0.4 + 1.2 * detail);
    let a = min(1.0 - exp(-v.size.z * d), v.size.w);
    // lit from above: thinner overhead, brighter
    let above = soft(q + vec2<f32>(0.0, 0.8 * cell));
    let lit = clamp(0.82 + (d - above) * 0.45 - d * 0.06, 0.5, 1.05);
    let col = mix(vec3<f32>(0.52, 0.54, 0.58), vec3<f32>(0.95, 0.95, 0.93), lit);
    var out: SceneOut;
    out.albedo = vec4<f32>(col, a);
    // (it hides glows behind it a little; it doesn't glow)
    out.emissive = vec4<f32>(col * 0.03, a * 0.5);
    return out;
}

// Fire: the grid holds how hot it is (about 1 in a flame's heart, falling
// away above and around what's burning). Looked up through noise racing
// upward, so tongues of it lick up and break off; cut away where the noise
// is low, more so where it's cooler; coloured along a flame's temperatures
// (deep red at its ragged edges, orange, yellow, nearly white at its
// heart), and glowing (into the bloom) far more than it shows as paint.
@fragment
fn fs_flame(o: FullOut) -> SceneOut {
    let p = v.view.xy + vec2<f32>((o.uv.x * 2.0 - 1.0) * v.view.z, (1.0 - o.uv.y * 2.0) * v.view.w);
    let t = v.grid.w;
    let cell = v.grid.z;
    if soft(p) < 0.003 && thick(p - vec2<f32>(0.0, cell * 3.0)) < 0.003 {
        discard;
    }
    // racing upward: sampling from below stretches the flames up
    let n1 = fbm(p * 4.5 + vec2<f32>(0.0, -t * 3.4));
    let n2 = fbm(p * 9.0 + vec2<f32>(3.1, -t * 5.6));
    let warp = vec2<f32>((n1 - 0.5) * 0.14, -n2 * 0.22);
    var heat = soft(p + warp);
    // tongues and gaps (more of them the cooler it is, so the edges and
    // tips break up)
    heat = heat * (0.35 + 1.0 * n2) - 0.28 * (1.0 - n1);
    if heat <= 0.02 {
        discard;
    }
    let x = clamp(heat / 1.2, 0.0, 1.0);
    let red = vec3<f32>(0.62, 0.07, 0.02);
    let orange = vec3<f32>(1.0, 0.38, 0.05);
    let yellow = vec3<f32>(1.0, 0.78, 0.28);
    let white = vec3<f32>(1.0, 0.96, 0.82);
    var col = mix(red, orange, smoothstep(0.0, 0.35, x));
    col = mix(col, yellow, smoothstep(0.35, 0.7, x));
    col = mix(col, white, smoothstep(0.75, 1.0, x));
    let a = smoothstep(0.02, 0.3, heat) * 0.85;
    var out: SceneOut;
    out.albedo = vec4<f32>(col * 0.7, a * 0.55);
    out.emissive = vec4<f32>(col * (0.55 + 0.9 * x), a * 0.9);
    return out;
}
