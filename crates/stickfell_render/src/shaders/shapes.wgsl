// SDF shapes: circles, capsules, rounded oriented boxes. One instanced quad each.

struct Camera {
    view_proj: mat4x4<f32>,
    // x = world units per pixel, y = time
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> cam: Camera;
// UI only: the font's signed-distance-field atlas (0.5 = glyph edge).
@group(1) @binding(0) var font_tex: texture_2d<f32>;
@group(1) @binding(1) var font_samp: sampler;
// Scene only: the sprite atlas (mipmapped, sRGB).
@group(1) @binding(2) var sprite_tex: texture_2d<f32>;
@group(1) @binding(3) var sprite_samp: sampler;

struct ShapeIn {
    @location(0) a: vec2<f32>,
    @location(1) b: vec2<f32>,
    @location(2) size: vec2<f32>,
    @location(3) rot: vec2<f32>,
    @location(4) color: vec4<f32>,
    @location(5) params: vec4<f32>,
};

struct ShapeOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec2<f32>,
    @location(1) @interpolate(flat) a: vec2<f32>,
    @location(2) @interpolate(flat) b: vec2<f32>,
    @location(3) @interpolate(flat) size: vec2<f32>,
    @location(4) @interpolate(flat) rot: vec2<f32>,
    @location(5) @interpolate(flat) color: vec4<f32>,
    @location(6) @interpolate(flat) params: vec4<f32>,
};

fn corner(i: u32) -> vec2<f32> {
    var c = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    return c[i];
}

@vertex
fn vs_shape(@builtin(vertex_index) vi: u32, s: ShapeIn) -> ShapeOut {
    let c = corner(vi);
    let aa = cam.params.x * 2.0;
    let kind = u32(s.params.x + 0.5);
    var w: vec2<f32>;
    if kind == 0u {
        w = s.a + c * (s.size.x + aa);
    } else if kind == 1u {
        let ax = s.b - s.a;
        let len = length(ax);
        var d = vec2<f32>(1.0, 0.0);
        if len > 1e-6 { d = ax / len; }
        let n = vec2<f32>(-d.y, d.x);
        let mid = (s.a + s.b) * 0.5;
        w = mid + d * c.x * (len * 0.5 + s.size.x + aa) + n * c.y * (s.size.x + aa);
    } else if kind == 5u {
        // ring corners: radius plus the line's half thickness
        w = s.a + c * (s.size.x + s.params.z + aa);
    } else if kind == 3u {
        // glyph: the quad already carries the distance-field margin
        w = s.a + c * s.size;
    } else if kind == 4u {
        // sprite: an oriented quad, its edges come from the texture's alpha
        let l = c * s.size;
        w = s.a + s.rot * l.x + vec2<f32>(-s.rot.y, s.rot.x) * l.y;
    } else {
        let l = c * (s.size + vec2<f32>(aa));
        w = s.a + s.rot * l.x + vec2<f32>(-s.rot.y, s.rot.x) * l.y;
    }
    var o: ShapeOut;
    o.clip = cam.view_proj * vec4<f32>(w, 0.0, 1.0);
    o.world = w;
    o.a = s.a;
    o.b = s.b;
    o.size = s.size;
    o.rot = s.rot;
    o.color = s.color;
    o.params = s.params;
    return o;
}

fn sdf(o: ShapeOut) -> f32 {
    let kind = u32(o.params.x + 0.5);
    if kind == 0u {
        return length(o.world - o.a) - o.size.x;
    } else if kind == 1u {
        let pa = o.world - o.a;
        let ba = o.b - o.a;
        let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-8), 0.0, 1.0);
        return length(pa - ba * h) - o.size.x;
    } else if kind == 5u {
        // four arcs of a ring, centred on the diagonals: fold into one
        // quadrant, turn the diagonal onto +y, then one arc of half-angle
        // params.w and half thickness params.z
        let p = abs(o.world - o.a);
        let q = vec2<f32>(abs(p.x - p.y), p.x + p.y) * 0.70710678;
        let sc = vec2<f32>(sin(o.params.w), cos(o.params.w));
        let ra = o.size.x;
        if sc.y * q.x > sc.x * q.y {
            return length(q - sc * ra) - o.params.z;
        }
        return abs(length(q) - ra) - o.params.z;
    }
    let d = o.world - o.a;
    let l = vec2<f32>(dot(d, o.rot), dot(d, vec2<f32>(-o.rot.y, o.rot.x)));
    let r = o.params.z;
    let q = abs(l) - (o.size - vec2<f32>(r));
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

struct SceneOut {
    @location(0) albedo: vec4<f32>,
    @location(1) emissive: vec4<f32>,
};

@fragment
fn fs_scene(o: ShapeOut) -> SceneOut {
    // sprite texel position; derivatives need uniform control flow, so
    // this runs for every shape (only sprites sample the atlas)
    let rel = o.world - o.a;
    let l = vec2<f32>(dot(rel, o.rot), dot(rel, vec2<f32>(-o.rot.y, o.rot.x))) / max(o.size, vec2<f32>(1e-6));
    let texel = o.b + vec2<f32>(l.x * 0.5 + 0.5, 0.5 - l.y * 0.5) * o.params.zw;
    let fw = fwidth(texel);
    if u32(o.params.x + 0.5) == 4u {
        let dims = vec2<f32>(textureDimensions(sprite_tex, 0));
        let lod = max(log2(max(max(fw.x, fw.y), 1e-6)), 0.0);
        let tex = textureSampleLevel(sprite_tex, sprite_samp, texel / dims, lod);
        let a = tex.a * o.color.a;
        if a <= 0.004 { discard; }
        var s: SceneOut;
        s.albedo = vec4<f32>(tex.rgb * o.color.rgb, a);
        s.emissive = vec4<f32>(tex.rgb * o.color.rgb * o.params.y, a);
        return s;
    }
    let d = sdf(o);
    let cov = clamp(0.5 - d / cam.params.x, 0.0, 1.0);
    if cov <= 0.0 { discard; }
    let a = o.color.a * cov;
    var out: SceneOut;
    out.albedo = vec4<f32>(o.color.rgb, a);
    out.emissive = vec4<f32>(o.color.rgb * o.params.y, a);
    return out;
}

@fragment
fn fs_ui(o: ShapeOut) -> @location(0) vec4<f32> {
    // the glyph lookup runs for every shape, since derivatives and texture
    // sampling must be in uniform control flow; only glyphs use it
    let local = (o.world - o.a) / max(o.size, vec2<f32>(1e-6)) * 0.5 + vec2<f32>(0.5);
    let texel = o.b + local * o.rot;
    let v = textureSampleLevel(font_tex, font_samp, texel / vec2<f32>(textureDimensions(font_tex)), 0.0).r;
    let texels_per_px = max(0.5 * (fwidth(texel).x + fwidth(texel).y), 1e-4);
    if u32(o.params.x + 0.5) == 3u {
        let dist = (v - 0.5) * 2.0 * o.params.z;
        let cov = clamp(dist / texels_per_px + 0.5, 0.0, 1.0);
        if cov <= 0.0 { discard; }
        return vec4<f32>(o.color.rgb, o.color.a * cov);
    }
    let d = sdf(o);
    let cov = clamp(0.5 - d / cam.params.x, 0.0, 1.0);
    if cov <= 0.0 { discard; }
    return vec4<f32>(o.color.rgb, o.color.a * cov);
}
