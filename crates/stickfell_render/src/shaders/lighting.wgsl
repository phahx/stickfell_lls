// Light accumulation: ambient light (darkened by contact shadows), then
// additive light quads, each shadowed by its row of the shadow map (soft edges
// that widen the further they fall past the blocker).

struct Camera {
    view_proj: mat4x4<f32>,
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> cam: Camera;

// level 0 of the shadow map, and the nearest-blocker copies (see shadows.wgsl)
@group(1) @binding(0) var depth0: texture_depth_2d;
@group(1) @binding(1) var levels: texture_2d<f32>;
// contact shadows this frame (same size as the light buffer, see ao.wgsl)
@group(1) @binding(2) var ao_tex: texture_2d<f32>;

struct Params {
    view: vec4<f32>,
    bake: vec4<f32>,
    // y = how much contact shadow reaches direct light
    misc: vec4<f32>,
    ambient: vec4<f32>,
};
@group(1) @binding(3) var<uniform> params: Params;

fn contact(clip: vec4<f32>) -> f32 {
    return textureLoad(ao_tex, vec2<i32>(clip.xy), 0).r;
}

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_ambient(@builtin(position) clip: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(params.ambient.rgb * contact(clip), 1.0);
}

const W: f32 = 2048.0;
const PI: f32 = 3.14159265;
const TAU: f32 = 6.28318531;
const MAX_LEVEL: i32 = 6;
// The widest soft edge, from the middle to either side (m).
const MAX_PENUMBRA: f32 = 1.2;
// Nothing nearer the light than this counts as a blocker when guessing how
// wide the soft edge can get (m).
const NEAR_BLOCKER: f32 = 0.35;

struct LightIn {
    // xy = position, z = radius, w = cos(cone) (-1 = point light)
    @location(0) p: vec4<f32>,
    // xy = spot direction, z = source size (radius, m), w = shadow row (-1 = none)
    @location(1) dir: vec4<f32>,
    @location(2) color: vec4<f32>,
};

struct LightOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec2<f32>,
    @location(1) @interpolate(flat) p: vec4<f32>,
    @location(2) @interpolate(flat) dir: vec4<f32>,
    @location(3) @interpolate(flat) color: vec4<f32>,
};

fn corner(i: u32) -> vec2<f32> {
    var c = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    return c[i];
}

@vertex
fn vs_light(@builtin(vertex_index) vi: u32, l: LightIn) -> LightOut {
    let w = l.p.xy + corner(vi) * l.p.z;
    var o: LightOut;
    o.clip = cam.view_proj * vec4<f32>(w, 0.0, 1.0);
    o.world = w;
    o.p = l.p;
    o.dir = l.dir;
    o.color = l.color;
    return o;
}

// The nearest blocker (fraction of the radius) in texel `i` of `level`.
fn blocker(level: i32, row: i32, i: i32) -> f32 {
    let w = i32(W) >> u32(level);
    let x = ((i % w) + w) % w;
    if level == 0 {
        return textureLoad(depth0, vec2<i32>(x, row), 0);
    }
    let off = i32(W) - 2 * w;
    return textureLoad(levels, vec2<i32>(off + x, row), 0).x;
}

// How much of the light reaches `d` metres out at texel position `u` (level
// 0 texels, continuous): 1 if it all does, 0 in full shadow. Nearest-blocker
// copies are compared at the level whose texels match the spacing, blended
// between neighbouring texels so edges glide rather than step.
fn lit_at(level: i32, row: i32, u: f32, d: f32) -> f32 {
    let s = u / f32(1 << u32(level)) - 0.5;
    let i = i32(floor(s));
    let f = s - floor(s);
    let a = select(0.0, 1.0, blocker(level, row, i) >= d);
    let b = select(0.0, 1.0, blocker(level, row, i + 1) >= d);
    return mix(a, b, f);
}

fn visibility(light: vec4<f32>, size: f32, row: i32, to: vec2<f32>) -> f32 {
    let r = light.z;
    let dist = length(to);
    if dist < 1e-3 {
        return 1.0;
    }
    let u = (atan2(to.y, to.x) + PI) / TAU * W;
    let texels_per_m = W / (TAU * dist);
    let bias = 0.025 + dist * 0.004;
    let d = (dist - bias) / r;
    // 1. the nearest blocker that could shade this point at all
    let reach = clamp(size * (dist - NEAR_BLOCKER) / NEAR_BLOCKER, 0.0, MAX_PENUMBRA) * texels_per_m;
    let sl = clamp(i32(ceil(log2(max(reach * 0.5, 1.0)))), 0, MAX_LEVEL);
    let step = f32(1 << u32(sl));
    var near = 1.0;
    for (var k = -2; k <= 2; k++) {
        let s = (u + f32(k) * step) / step - 0.5;
        near = min(near, blocker(sl, row, i32(round(s))));
    }
    if near >= d {
        return 1.0;
    }
    // 2. how wide the soft edge is here: wider the further past the blocker
    let db = max(near * r, 0.02);
    let pen = clamp(size * (dist - db) / db, 0.0, MAX_PENUMBRA) * texels_per_m;
    if pen < 1.0 {
        return lit_at(0, row, u, d);
    }
    // 3. average the light over that width
    let n = 12;
    let level = clamp(i32(floor(log2(2.0 * pen / f32(n)))), 0, MAX_LEVEL);
    var lit = 0.0;
    for (var k = 0; k < n; k++) {
        let o = (f32(k) + 0.5) / f32(n) * 2.0 - 1.0;
        lit += lit_at(level, row, u + o * pen, d);
    }
    let v = lit / f32(n);
    return v * v * (3.0 - 2.0 * v);
}

@fragment
fn fs_light(o: LightOut) -> @location(0) vec4<f32> {
    let d = o.world - o.p.xy;
    let dist = length(d);
    let x = clamp(dist / o.p.z, 0.0, 1.0);
    var att = 1.0 - x * x;
    att = att * att;
    // brighter core so lamps read as light sources
    att = att + 0.6 * exp(-dist * dist / max(o.p.z * o.p.z * 0.01, 1e-4)) * (1.0 - x);
    if o.p.w > -0.999 && dist > 1e-4 {
        let c = dot(d / dist, o.dir.xy);
        att = att * smoothstep(o.p.w, mix(o.p.w, 1.0, 0.25), c);
    }
    if att > 1e-4 && o.dir.w >= 0.0 {
        att = att * visibility(o.p, o.dir.z, i32(o.dir.w), d);
    }
    att = att * mix(1.0, contact(o.clip), params.misc.y);
    return vec4<f32>(o.color.rgb * att, 1.0);
}
