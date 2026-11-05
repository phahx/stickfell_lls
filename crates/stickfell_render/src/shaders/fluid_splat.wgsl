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

struct Camera {
    view_proj: mat4x4<f32>,
    params: vec4<f32>,
};

// ---- splat ----

@group(0) @binding(0) var<uniform> cam: Camera;

struct SplatIn {
    @location(0) pos: vec2<f32>,
    // radius, kind, foam, dye
    @location(1) params: vec4<f32>,
    @location(2) vel: vec2<f32>,
};

struct SplatOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) params: vec4<f32>,
};

@vertex
fn vs_splat(@builtin(vertex_index) vi: u32, s: SplatIn) -> SplatOut {
    var c = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let k = c[vi];
    // stretched along its motion by half the way it goes in a 60th of a
    // second, so fast drops in a row run together into a stream
    let speed = length(s.vel);
    var along = vec2<f32>(1.0, 0.0);
    if speed > 1e-4 {
        along = s.vel / speed;
    }
    let across = vec2<f32>(-along.y, along.x);
    let r = s.params.x;
    let world = s.pos + along * (k.x * (r + speed * (0.5 / 60.0))) + across * (k.y * r);
    var o: SplatOut;
    o.clip = cam.view_proj * vec4<f32>(world, 0.0, 1.0);
    o.local = k;
    o.params = s.params;
    return o;
}

struct SplatField {
    // each kind's share (r, g, b), foam (a)
    @location(0) kinds: vec4<f32>,
    // dye (r)
    @location(1) dye: vec4<f32>,
};

@fragment
fn fs_splat(o: SplatOut) -> SplatField {
    let r2 = dot(o.local, o.local);
    if r2 >= 1.0 {
        discard;
    }
    let a = 1.0 - r2;
    let w = a * a * a;
    let kind = u32(o.params.y + 0.5);
    var v = vec4<f32>(0.0, 0.0, 0.0, w * o.params.z);
    if kind == 0u {
        v.x = w;
    } else if kind == 1u {
        v.y = w;
    } else {
        v.z = w;
    }
    var out: SplatField;
    out.kinds = v;
    out.dye = vec4<f32>(w * o.params.w, 0.0, 0.0, 0.0);
    return out;
}

