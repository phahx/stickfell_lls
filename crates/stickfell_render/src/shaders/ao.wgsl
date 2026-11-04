// Contact shadows (ambient occlusion): soft darkening of the wall wherever
// something stands close in front of it.
//  - cs_bake: once per level, how enclosed each spot is by the level's solid
//    boxes (rays out in every direction; the nearer what they hit, the darker).
//  - fs_ao: every frame at half resolution, that baked map times the
//    darkening around moving things (fighters' limbs and heads, weapons).

const TAU: f32 = 6.28318531;
// How far out the level's boxes darken the wall (m), and how dark it gets.
const BAKE_REACH: f32 = 1.2;
const BAKE_DEPTH: f32 = 0.6;
const RAYS: u32 = 32u;
// The same for moving things: reach beyond their surface (m), darkest.
const DYN_REACH: f32 = 0.32;
const DYN_DEPTH: f32 = 0.3;

// ---- bake --------------------------------------------------------------------

struct BakeParams {
    // xy = the map's lower corner (world), z = texel size (m), w = box count
    origin: vec4<f32>,
    // xy = texels across, up
    dims: vec4<u32>,
};

@group(0) @binding(0) var<uniform> bake: BakeParams;
// two per box: (centre, half size), (rotation cos, sin, 0, 0)
@group(0) @binding(1) var<storage, read> boxes: array<vec4<f32>>;
@group(0) @binding(2) var baked_out: texture_storage_2d<rgba8unorm, write>;

// Where the ray (o, d) first enters box `i` (or a huge number if it doesn't).
fn enter_box(o: vec2<f32>, d: vec2<f32>, i: u32) -> f32 {
    let cb = boxes[2u * i];
    let r = boxes[2u * i + 1u].xy;
    let rp = vec2<f32>(-r.y, r.x);
    let p = o - cb.xy;
    let lo = vec2<f32>(dot(p, r), dot(p, rp));
    var ld = vec2<f32>(dot(d, r), dot(d, rp));
    ld = select(ld, vec2<f32>(1e-9), abs(ld) < vec2<f32>(1e-9));
    let t1 = (-cb.zw - lo) / ld;
    let t2 = (cb.zw - lo) / ld;
    let tn = max(min(t1.x, t2.x), min(t1.y, t2.y));
    let tf = min(max(t1.x, t2.x), max(t1.y, t2.y));
    return select(1e9, tn, tf >= max(tn, 0.0) && tn >= 0.0);
}

fn inside_box(p: vec2<f32>, i: u32) -> bool {
    let cb = boxes[2u * i];
    let r = boxes[2u * i + 1u].xy;
    let q = p - cb.xy;
    return abs(dot(q, r)) <= cb.z && abs(dot(q, vec2<f32>(-r.y, r.x))) <= cb.w;
}

@compute @workgroup_size(8, 8)
fn cs_bake(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= bake.dims.x || id.y >= bake.dims.y {
        return;
    }
    let p = bake.origin.xy + (vec2<f32>(id.xy) + 0.5) * bake.origin.z;
    let n = u32(bake.origin.w);
    var ao = 1.0;
    var solid = false;
    for (var i = 0u; i < n; i++) {
        solid = solid || inside_box(p, i);
    }
    // (a box's own face is out in front: nothing darkens it)
    if !solid {
        var occ = 0.0;
        for (var k = 0u; k < RAYS; k++) {
            let a = (f32(k) + 0.5) / f32(RAYS) * TAU;
            let d = vec2<f32>(cos(a), sin(a));
            var t = BAKE_REACH;
            for (var i = 0u; i < n; i++) {
                t = min(t, enter_box(p, d, i));
            }
            let near = 1.0 - t / BAKE_REACH;
            occ += near * near;
        }
        ao = 1.0 - BAKE_DEPTH * clamp(occ / f32(RAYS) * 2.0, 0.0, 1.0);
    }
    textureStore(baked_out, vec2<i32>(id.xy), vec4<f32>(ao, ao, ao, 1.0));
}

// ---- per frame ---------------------------------------------------------------

struct Params {
    // xy = camera centre, zw = half the view (m)
    view: vec4<f32>,
    // xy = the baked map's lower corner, zw = its size (m); size 0 = none
    bake: vec4<f32>,
    // x = moving things, y = how much contact shadow reaches direct light
    misc: vec4<f32>,
    ambient: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var baked: texture_2d<f32>;
@group(0) @binding(2) var baked_samp: sampler;
// two per moving thing: (a, b), (radius, 0, 0, 0); a circle has a == b
@group(0) @binding(3) var<storage, read> movers: array<vec4<f32>>;

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

fn seg_dist(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-8), 0.0, 1.0);
    return length(p - a - ab * t);
}

@fragment
fn fs_ao(o: FullOut) -> @location(0) vec4<f32> {
    let p = params.view.xy + vec2<f32>(o.uv.x * 2.0 - 1.0, 1.0 - o.uv.y * 2.0) * params.view.zw;
    var ao = 1.0;
    if params.bake.z > 0.0 {
        let uv = (p - params.bake.xy) / params.bake.zw;
        if all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0)) {
            ao = textureSampleLevel(baked, baked_samp, uv, 0.0).r;
        }
    }
    // moving things darken what's just around them, not themselves
    var dyn = 1.0;
    let n = u32(params.misc.x);
    for (var i = 0u; i < n; i++) {
        let s = movers[2u * i];
        let d = seg_dist(p, s.xy, s.zw) - movers[2u * i + 1u].x;
        if d <= 0.0 {
            dyn = 1.0;
            break;
        }
        if d < DYN_REACH {
            let k = 1.0 - d / DYN_REACH;
            dyn *= 1.0 - DYN_DEPTH * k * k;
        }
    }
    return vec4<f32>(ao * dyn, 0.0, 0.0, 1.0);
}
