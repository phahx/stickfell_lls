// Shadow maps for 2D lights: one row per shadow-casting light, one texel per
// direction around it (-pi at the left edge to +pi at the right), holding how
// far the light gets that way before something stops it (as a fraction of the
// light's radius). Then each row is shrunk into coarser copies that keep the
// nearest blocker, so wide soft edges can be filtered with a few reads.

const W: f32 = 2048.0;
const PI: f32 = 3.14159265;
const TAU: f32 = 6.28318531;
const ROWS: f32 = 256.0;

// ---- casters: depth = distance to the occluder along each texel's ray ------

struct CasterIn {
    // xy = light position, z = light radius, w = row
    @location(0) light: vec4<f32>,
    // x..y = angle span (radians), z = kind (0 segment, 1 box, 2 circle)
    @location(1) span: vec4<f32>,
    // segment: a, b | box: centre, half size | circle: centre, radius
    @location(2) g0: vec4<f32>,
    // box: rotation (cos, sin)
    @location(3) g1: vec4<f32>,
};

struct CasterOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) light: vec4<f32>,
    @location(1) @interpolate(flat) kind: f32,
    @location(2) @interpolate(flat) g0: vec4<f32>,
    @location(3) @interpolate(flat) g1: vec4<f32>,
};

@vertex
fn vs_caster(@builtin(vertex_index) vi: u32, c: CasterIn) -> CasterOut {
    var corner = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let k = corner[vi];
    // a texel and a half wider each side, so every texel it touches is drawn
    let pad = 1.5 * TAU / W;
    let a = mix(c.span.x - pad, c.span.y + pad, k.x);
    let y = 1.0 - 2.0 * (c.light.w + k.y) / ROWS;
    var o: CasterOut;
    o.clip = vec4<f32>(a / PI, y, 0.0, 1.0);
    o.light = c.light;
    o.kind = c.span.z;
    o.g0 = c.g0;
    o.g1 = c.g1;
    return o;
}

fn cross2(a: vec2<f32>, b: vec2<f32>) -> f32 {
    return a.x * b.y - a.y * b.x;
}

// Distance along the ray (o, d) to where it's stopped, or -1 if it isn't.
// Boxes and circles stop light where it leaves them (their own lit side stays
// lit); segments where it crosses them.
fn hit(o: vec2<f32>, d: vec2<f32>, kind: f32, g0: vec4<f32>, g1: vec4<f32>) -> f32 {
    if kind < 0.5 {
        let e = g0.zw - g0.xy;
        let den = cross2(d, e);
        if abs(den) < 1e-9 {
            return -1.0;
        }
        let w = g0.xy - o;
        let t = cross2(w, e) / den;
        let s = cross2(w, d) / den;
        if t > 0.0 && s >= 0.0 && s <= 1.0 {
            return t;
        }
        return -1.0;
    }
    if kind < 1.5 {
        let r = g1.xy;
        let rp = vec2<f32>(-r.y, r.x);
        let p = o - g0.xy;
        let lo = vec2<f32>(dot(p, r), dot(p, rp));
        var ld = vec2<f32>(dot(d, r), dot(d, rp));
        ld = select(ld, vec2<f32>(1e-9), abs(ld) < vec2<f32>(1e-9));
        let t1 = (-g0.zw - lo) / ld;
        let t2 = (g0.zw - lo) / ld;
        let tn = max(min(t1.x, t2.x), min(t1.y, t2.y));
        let tf = min(max(t1.x, t2.x), max(t1.y, t2.y));
        if tf > 0.0 && tf >= tn {
            return tf;
        }
        return -1.0;
    }
    let oc = o - g0.xy;
    let b = dot(oc, d);
    let cc = dot(oc, oc) - g0.z * g0.z;
    let disc = b * b - cc;
    if disc < 0.0 {
        return -1.0;
    }
    let t = -b + sqrt(disc);
    return select(-1.0, t, t > 0.0);
}

struct DepthOut {
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_caster(o: CasterOut) -> DepthOut {
    let a = o.clip.x / W * TAU - PI;
    let t = hit(o.light.xy, vec2<f32>(cos(a), sin(a)), o.kind, o.g0, o.g1);
    if t < 0.0 {
        discard;
    }
    var out: DepthOut;
    out.depth = clamp(t / o.light.z, 0.0, 1.0);
    return out;
}

// ---- nearest-blocker copies: level k is 2048 >> k wide, packed side by side
// in one texture (level 1 at x 0, level 2 at 1024, ...) ----------------------

@group(0) @binding(0) var depth0: texture_depth_2d;
@group(0) @binding(1) var levels: texture_storage_2d<r32float, write>;

// Texels across all the copies (1024 + 512 + ... + 32).
const LEVEL_TEXELS: u32 = 2016u;

// One texel of one copy: the nearest blocker over the level-0 texels it
// covers. (Plain loops with run-time bounds and no shared memory: DirectX's
// FXC compiler takes minutes over the unrolled shared-memory version.)
@compute @workgroup_size(64)
fn cs_reduce(@builtin(global_invocation_id) id: vec3<u32>) {
    let x = id.x;
    let row = i32(id.y);
    if x >= LEVEL_TEXELS {
        return;
    }
    var level = 1u;
    var w = 1024u;
    var off = 0u;
    loop {
        if x < off + w {
            break;
        }
        off = off + w;
        w = w / 2u;
        level = level + 1u;
    }
    let span = 1u << level;
    let start = (x - off) * span;
    var m = 1.0;
    for (var j = 0u; j < span; j++) {
        m = min(m, textureLoad(depth0, vec2<i32>(i32(start + j), row), 0));
    }
    textureStore(levels, vec2<i32>(i32(x), row), vec4<f32>(m, 0.0, 0.0, 0.0));
}
