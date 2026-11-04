// Bloom for emissive surfaces: 4x downsample, then separable gaussian blur.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

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

@fragment
fn fs_down(i: FullOut) -> @location(0) vec4<f32> {
    // 4 bilinear taps = 16-texel box from the full-res source
    let ts = 1.0 / vec2<f32>(textureDimensions(src));
    var c = textureSample(src, samp, i.uv + ts * vec2<f32>(-1.0, -1.0)).rgb;
    c += textureSample(src, samp, i.uv + ts * vec2<f32>(1.0, -1.0)).rgb;
    c += textureSample(src, samp, i.uv + ts * vec2<f32>(-1.0, 1.0)).rgb;
    c += textureSample(src, samp, i.uv + ts * vec2<f32>(1.0, 1.0)).rgb;
    return vec4<f32>(c * 0.25, 1.0);
}

fn blur(uv: vec2<f32>, dir: vec2<f32>) -> vec4<f32> {
    // 9-tap gaussian folded into 5 linear samples
    let ts = dir / vec2<f32>(textureDimensions(src));
    var c = textureSample(src, samp, uv).rgb * 0.227027;
    c += textureSample(src, samp, uv + ts * 1.384615).rgb * 0.316216;
    c += textureSample(src, samp, uv - ts * 1.384615).rgb * 0.316216;
    c += textureSample(src, samp, uv + ts * 3.230769).rgb * 0.070270;
    c += textureSample(src, samp, uv - ts * 3.230769).rgb * 0.070270;
    return vec4<f32>(c, 1.0);
}

@fragment
fn fs_blur_h(i: FullOut) -> @location(0) vec4<f32> {
    return blur(i.uv, vec2<f32>(1.0, 0.0));
}

@fragment
fn fs_blur_v(i: FullOut) -> @location(0) vec4<f32> {
    return blur(i.uv, vec2<f32>(0.0, 1.0));
}
