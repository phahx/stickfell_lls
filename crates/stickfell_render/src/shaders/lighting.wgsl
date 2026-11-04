// Light accumulation: shadow volumes into stencil, then additive light quads,
// followed by the full-screen composite (albedo * light + emissive, tonemap).

struct Camera {
    view_proj: mat4x4<f32>,
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> cam: Camera;

// ---- shadow volumes (stencil only) ----------------------------------------

@vertex
fn vs_shadow(@location(0) pos: vec2<f32>) -> @builtin(position) vec4<f32> {
    return cam.view_proj * vec4<f32>(pos, 0.0, 1.0);
}

@fragment
fn fs_shadow() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}

// ---- lights ---------------------------------------------------------------

struct LightIn {
    // xy = position, z = radius, w = cos(cone) (-1 = point light)
    @location(0) p: vec4<f32>,
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
    return vec4<f32>(o.color.rgb * att, 1.0);
}
