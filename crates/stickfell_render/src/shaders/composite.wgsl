// Full-screen composite: albedo * light + emissive + bloom, tonemap, vignette.


@group(0) @binding(0) var t_albedo: texture_2d<f32>;
@group(0) @binding(1) var t_emissive: texture_2d<f32>;
@group(0) @binding(2) var t_light: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var t_bloom: texture_2d<f32>;

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

fn aces(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_composite(i: FullOut) -> @location(0) vec4<f32> {
    let albedo = textureSample(t_albedo, samp, i.uv).rgb;
    let emissive = textureSample(t_emissive, samp, i.uv).rgb;
    // Soft shadows: small blur of the (half-res) light buffer.
    let ts = 1.0 / vec2<f32>(textureDimensions(t_light));
    var light = textureSample(t_light, samp, i.uv).rgb * 0.36;
    light += textureSample(t_light, samp, i.uv + vec2<f32>(ts.x, 0.0) * 1.5).rgb * 0.16;
    light += textureSample(t_light, samp, i.uv - vec2<f32>(ts.x, 0.0) * 1.5).rgb * 0.16;
    light += textureSample(t_light, samp, i.uv + vec2<f32>(0.0, ts.y) * 1.5).rgb * 0.16;
    light += textureSample(t_light, samp, i.uv - vec2<f32>(0.0, ts.y) * 1.5).rgb * 0.16;
    let bloom = textureSample(t_bloom, samp, i.uv).rgb;
    var c = albedo * light + emissive + bloom * 0.9;
    c = aces(c * 1.15);
    let v = i.uv - vec2<f32>(0.5);
    c = c * (1.0 - dot(v, v) * 0.55);
    return vec4<f32>(c, 1.0);
}
