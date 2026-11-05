//! GPU side: pipelines, render targets and the per-frame pass sequence.
//!
//! Passes:
//! 1. **Scene** — SDF shapes into `albedo` (sRGB) + `emissive` (HDR) targets.
//! 2. **Shadow maps** — one row per shadow-casting light, one texel per
//!    direction round it, holding how far its light gets that way; then
//!    coarser nearest-blocker copies of each row (a compute pass).
//! 3. **Contact shadows** — half-res: the level's baked map (worked out once
//!    per level, the first time it's shown) times soft darkening around
//!    moving things.
//! 4. **Lights** — half-res HDR light buffer: ambient light darkened by the
//!    contact shadows, then each light quad added, shadowed by its row with
//!    edges that soften the further they fall past the blocker.
//! 5. **Composite** — `albedo * light + emissive (+glow)`, tonemapped, into
//!    the output target.
//! 6. **UI** — unlit screen-space shapes/text on top.

use bytemuck::{Pod, Zeroable};
use glam::Vec2;
use wgpu::util::DeviceExt;

use crate::frame::{Frame, Light, Occluder, ShapeInstance, StaticScene};

const ALBEDO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const MAX_LIGHTS: usize = 255;
/// Shadow map: directions per light (must match the shaders' `W`), and rows
/// (lights that cast shadows; must match `ROWS`).
const SHADOW_W: u32 = 2048;
const SHADOW_ROWS: u32 = 256;
/// Contact shadow map texel size (m), and the most texels along a side.
const BAKE_TEXEL: f32 = 0.05;
const BAKE_MAX: u32 = 4096;
/// How many levels' bakes are kept.
const BAKES_KEPT: usize = 16;
/// How much of the contact shadow reaches direct light (all of it reaches
/// ambient light).
const DIRECT_CONTACT: f32 = 0.35;
/// Moving things that cast contact shadows, at most.
const MAX_MOVERS: usize = 512;
/// A limb's thickness for its contact shadow (m).
const LIMB_RADIUS: f32 = 0.06;

/// A level's baked contact shadows.
struct Bake {
    view: wgpu::TextureView,
    min: Vec2,
    size: Vec2,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightInstance {
    p: [f32; 4],
    /// spot direction, source size, shadow row (-1 for none)
    dir: [f32; 4],
    color: [f32; 4],
}

/// One occluder as seen from one light, drawn into that light's shadow row.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CasterInstance {
    /// light position, radius, row
    light: [f32; 4],
    /// angle span (radians, within -pi..pi), kind (0 segment, 1 box, 2 circle)
    span: [f32; 4],
    g0: [f32; 4],
    g1: [f32; 4],
}

struct DynBuffer {
    buf: wgpu::Buffer,
    cap: u64,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

impl DynBuffer {
    fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages) -> Self {
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        let cap = 4096;
        let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: cap, usage, mapped_at_creation: false });
        Self { buf, cap, usage, label }
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let len = data.len() as u64;
        if len > self.cap {
            self.cap = len.next_power_of_two();
            self.buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: self.cap,
                usage: self.usage,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(&self.buf, 0, data);
    }
}

struct Targets {
    size: (u32, u32),
    albedo: wgpu::TextureView,
    emissive: wgpu::TextureView,
    light: wgpu::TextureView,
    bloom_a: wgpu::TextureView,
    bloom_b: wgpu::TextureView,
    composite_bg: wgpu::BindGroup,
    /// Post-process sources: emissive, bloom_a, bloom_b.
    post_bg: [wgpu::BindGroup; 3],
    /// Contact shadows this frame (light buffer size).
    ao: wgpu::TextureView,
    /// Shadow maps, contact shadows and params, for the light pass.
    light_bg: wgpu::BindGroup,
    /// The liquids' drops, splatted (see shaders/fluid_*.wgsl): kinds and
    /// foam, and dye; and those for the resolve.
    fluid: wgpu::TextureView,
    fluid_dye: wgpu::TextureView,
    fluid_bg: wgpu::BindGroup,
}

pub struct Renderer {
    /// Everything but the UI's pipeline, once compiled (see `pending`).
    pipes: Option<Pipelines>,
    pending: Option<std::sync::mpsc::Receiver<Pipelines>>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    output_format: wgpu::TextureFormat,
    cam_buf: wgpu::Buffer,
    cam_bg: wgpu::BindGroup,
    ui_cam_buf: wgpu::Buffer,
    ui_cam_bg: wgpu::BindGroup,
    ui_pipe: wgpu::RenderPipeline,
    font_bg: wgpu::BindGroup,
    /// For remaking `font_bg` when the sprite atlas changes.
    font_bgl: wgpu::BindGroupLayout,
    font_view: wgpu::TextureView,
    font_samp: wgpu::Sampler,
    sprite_bgl: wgpu::BindGroupLayout,
    sprite_samp: wgpu::Sampler,
    sprite_bg: wgpu::BindGroup,
    /// The atlas currently on the GPU (compared by pointer).
    sprite_atlas: Option<std::sync::Arc<crate::SpriteAtlas>>,
    shadow_depth: wgpu::TextureView,
    reduce_bg: wgpu::BindGroup,
    shadow_levels: wgpu::TextureView,
    light_bgl: wgpu::BindGroupLayout,
    params_buf: wgpu::Buffer,
    bake_bgl: wgpu::BindGroupLayout,
    ao_bgl: wgpu::BindGroupLayout,
    /// Baked levels (each `Arc` is kept so its pointer can't be reused).
    bakes: Vec<(std::sync::Arc<StaticScene>, Bake)>,
    /// White: no contact shadows (no level baked).
    no_bake: wgpu::TextureView,
    movers_buf: DynBuffer,
    movers: Vec<[f32; 4]>,
    composite_bgl: wgpu::BindGroupLayout,
    post_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    targets: Option<Targets>,
    shape_buf: DynBuffer,
    splat_buf: DynBuffer,
    fluid_bgl: wgpu::BindGroupLayout,
    /// Each kind of liquid's look, the view and the time (for the resolve).
    fluid_buf: wgpu::Buffer,
    /// Vapour: its layout, its params (view, grid, look), and its grid as a
    /// texture (sized to the grid, remade when that changes) with its bind
    /// group.
    vapour_bgl: wgpu::BindGroupLayout,
    vapour_buf: wgpu::Buffer,
    vapour_tex: Option<((u32, u32), wgpu::Texture, wgpu::BindGroup)>,
    ui_buf: DynBuffer,
    light_buf: DynBuffer,
    caster_buf: DynBuffer,
    casters: Vec<CasterInstance>,
    light_insts: Vec<LightInstance>,
    /// Shadow rows in use this frame.
    shadow_rows: u32,
}

/// The vapour shader's params: view, grid, look (three vec4s).
const VAPOUR_UNIFORM: usize = 12;

/// An f32 as a half float's bits (what an R16Float texture holds): rounded
/// to nearest, too small to hold going to zero, too big to the largest.
fn f32_to_f16(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let man = b & 0x7f_ffff;
    if x.is_nan() || exp <= 0 {
        return sign;
    }
    if exp >= 31 {
        return sign | 0x7bff;
    }
    let h = ((exp as u32) << 10) | (man >> 13);
    // (round to nearest: the dropped bits' top one)
    let h = if man & 0x1000 != 0 { h + 1 } else { h };
    sign | h.min(0x7bff) as u16
}

/// Uploads a sprite atlas (with its mips).
fn upload_atlas(device: &wgpu::Device, queue: &wgpu::Queue, atlas: &crate::SpriteAtlas) -> wgpu::TextureView {
    let levels = atlas.mips.len().max(1) as u32;
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sprite atlas"),
        size: wgpu::Extent3d { width: atlas.width.max(1), height: atlas.height.max(1), depth_or_array_layers: 1 },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, px) in atlas.mips.iter().enumerate() {
        let w = (atlas.width >> level).max(1);
        let h = (atlas.height >> level).max(1);
        assert_eq!(px.len(), (w * h * 4) as usize, "sprite atlas mip {level} is the wrong size");
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            px,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: None },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }
    tex.create_view(&Default::default())
}

/// The scene pass's sprites: the atlas and its sampler.
fn sprite_bind_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, view: &wgpu::TextureView) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sprites"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(view) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

/// The UI's textures: the font atlas, and the sprite atlas (for pictures).
fn ui_bind_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, font: (&wgpu::TextureView, &wgpu::Sampler), sprites: (&wgpu::TextureView, &wgpu::Sampler)) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ui"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(font.0) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(font.1) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(sprites.0) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(sprites.1) },
        ],
    })
}

fn ortho(center: Vec2, half: Vec2) -> [f32; 16] {
    let sx = 1.0 / half.x;
    let sy = 1.0 / half.y;
    [sx, 0.0, 0.0, 0.0, 0.0, sy, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -center.x * sx, -center.y * sy, 0.0, 1.0]
}

fn camera_uniform(m: [f32; 16], units_per_px: f32, time: f32) -> [f32; 20] {
    let mut u = [0.0; 20];
    u[..16].copy_from_slice(&m);
    u[16] = units_per_px;
    u[17] = time;
    u
}

fn shape_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRS: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
        0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x2, 4 => Float32x4, 5 => Float32x4
    ];
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ShapeInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRS,
    }
}

fn splat_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    const ATTRS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4, 2 => Float32x2];
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<crate::frame::FluidSplat>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &ATTRS,
    }
}

/// The liquid field: each kind's share in r, g, b, foam in a.
const FLUID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Floats in the liquids' uniform: three looks of nine vec4s, the view, params.
const FLUID_LOOK: usize = 36;
const FLUID_UNIFORM: usize = 3 * FLUID_LOOK + 8;
/// Where the field says there's liquid.
const FLUID_THRESHOLD: f32 = 0.35;

/// Bind group layouts the pipelines are made against (shared with the
/// thread that compiles them).
#[derive(Clone)]
struct Layouts {
    fluid: wgpu::BindGroupLayout,
    vapour: wgpu::BindGroupLayout,
    cam: wgpu::BindGroupLayout,
    sprite: wgpu::BindGroupLayout,
    light: wgpu::BindGroupLayout,
    bake: wgpu::BindGroupLayout,
    ao: wgpu::BindGroupLayout,
    reduce: wgpu::BindGroupLayout,
    post: wgpu::BindGroupLayout,
    composite: wgpu::BindGroupLayout,
}

/// Every pipeline but the UI's: the slow part of starting up (each shader is
/// compiled for the GPU here), so it can run on another thread while a
/// loading screen draws.
struct Pipelines {
    shape: wgpu::RenderPipeline,
    splat: wgpu::RenderPipeline,
    resolve: wgpu::RenderPipeline,
    vapour: wgpu::RenderPipeline,
    caster: wgpu::RenderPipeline,
    reduce: wgpu::ComputePipeline,
    ambient: wgpu::RenderPipeline,
    light: wgpu::RenderPipeline,
    bake: wgpu::ComputePipeline,
    ao: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    bloom_down: wgpu::RenderPipeline,
    blur_h: wgpu::RenderPipeline,
    blur_v: wgpu::RenderPipeline,
}

fn build_pipelines(device: &wgpu::Device, l: &Layouts, output_format: wgpu::TextureFormat) -> Pipelines {
    let shapes_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("stickfell shapes"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shapes.wgsl").into()),
    });
    let blend_target = |format| {
        Some(wgpu::ColorTargetState { format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })
    };
    let tri = wgpu::PrimitiveState::default();
    let light_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("stickfell lighting"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/lighting.wgsl").into()),
    });
    let comp_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("stickfell composite"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/composite.wgsl").into()),
    });
    let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("scene layout"),
        bind_group_layouts: &[Some(&l.cam), Some(&l.sprite)],
        immediate_size: 0,
    });
    let shape_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("shapes"),
        layout: Some(&scene_layout),
        vertex: wgpu::VertexState {
            module: &shapes_sm,
            entry_point: Some("vs_shape"),
            compilation_options: Default::default(),
            buffers: &[Some(shape_vertex_layout())],
        },
        primitive: tri,
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shapes_sm,
            entry_point: Some("fs_scene"),
            compilation_options: Default::default(),
            targets: &[blend_target(ALBEDO_FORMAT), blend_target(HDR_FORMAT)],
        }),
        multiview_mask: None,
        cache: None,
    });
    // liquids: drops splatted into a field, then resolved over the scene
    let splat_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("stickfell liquid splat"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fluid_splat.wgsl").into()),
    });
    let resolve_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("stickfell liquid resolve"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fluid_resolve.wgsl").into()),
    });
    let splat_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("liquid splat layout"),
        bind_group_layouts: &[Some(&l.cam)],
        immediate_size: 0,
    });
    let add = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add };
    let splat_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("liquid splat"),
        layout: Some(&splat_layout),
        vertex: wgpu::VertexState {
            module: &splat_sm,
            entry_point: Some("vs_splat"),
            compilation_options: Default::default(),
            buffers: &[Some(splat_vertex_layout())],
        },
        primitive: tri,
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &splat_sm,
            entry_point: Some("fs_splat"),
            compilation_options: Default::default(),
            targets: &[
                Some(wgpu::ColorTargetState { format: FLUID_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::ALL }),
                Some(wgpu::ColorTargetState { format: FLUID_FORMAT, blend: Some(wgpu::BlendState { color: add, alpha: add }), write_mask: wgpu::ColorWrites::ALL }),
            ],
        }),
        multiview_mask: None,
        cache: None,
    });
    let resolve_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("liquid resolve layout"),
        bind_group_layouts: &[Some(&l.fluid)],
        immediate_size: 0,
    });
    let resolve_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("liquid resolve"),
        layout: Some(&resolve_layout),
        vertex: wgpu::VertexState {
            module: &resolve_sm,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: tri,
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &resolve_sm,
            entry_point: Some("fs_resolve"),
            compilation_options: Default::default(),
            targets: &[blend_target(ALBEDO_FORMAT), blend_target(HDR_FORMAT)],
        }),
        multiview_mask: None,
        cache: None,
    });
    // vapour: its grid drawn as one volume over the scene
    let vapour_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("stickfell vapour"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/vapour.wgsl").into()),
    });
    let vapour_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("vapour layout"),
        bind_group_layouts: &[Some(&l.vapour)],
        immediate_size: 0,
    });
    let vapour_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("vapour"),
        layout: Some(&vapour_layout),
        vertex: wgpu::VertexState { module: &vapour_sm, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
        primitive: tri,
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &vapour_sm,
            entry_point: Some("fs_vapour"),
            compilation_options: Default::default(),
            targets: &[blend_target(ALBEDO_FORMAT), blend_target(HDR_FORMAT)],
        }),
        multiview_mask: None,
        cache: None,
    });
    // shadow maps: occluders drawn into each light's row as depth, then
    // shrunk into nearest-blocker copies
    let shadow_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("stickfell shadows"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shadows.wgsl").into()),
    });
    let light_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("light layout"),
        bind_group_layouts: &[Some(&l.cam), Some(&l.light)],
        immediate_size: 0,
    });
    let ambient_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ambient light"),
        layout: Some(&light_layout),
        vertex: wgpu::VertexState { module: &light_sm, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
        primitive: tri,
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &light_sm,
            entry_point: Some("fs_ambient"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
        }),
        multiview_mask: None,
        cache: None,
    });
    // contact shadows: the per-level bake and the per-frame pass
    let ao_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("stickfell contact shadows"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/ao.wgsl").into()),
    });
    let bake_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("contact bake layout"),
        bind_group_layouts: &[Some(&l.bake)],
        immediate_size: 0,
    });
    let bake_pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("contact bake"),
        layout: Some(&bake_layout),
        module: &ao_sm,
        entry_point: Some("cs_bake"),
        compilation_options: Default::default(),
        cache: None,
    });
    let ao_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("contact shadows layout"),
        bind_group_layouts: &[Some(&l.ao)],
        immediate_size: 0,
    });
    let ao_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("contact shadows"),
        layout: Some(&ao_layout),
        vertex: wgpu::VertexState { module: &ao_sm, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
        primitive: tri,
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &ao_sm,
            entry_point: Some("fs_ao"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::R8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    const CASTER_ATTRS: [wgpu::VertexAttribute; 4] =
        wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4];
    let empty_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("no bindings"),
        bind_group_layouts: &[],
        immediate_size: 0,
    });
    let caster_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("shadow casters"),
        layout: Some(&empty_layout),
        vertex: wgpu::VertexState {
            module: &shadow_sm,
            entry_point: Some("vs_caster"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<CasterInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &CASTER_ATTRS,
            })],
        },
        primitive: tri,
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SHADOW_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shadow_sm,
            entry_point: Some("fs_caster"),
            compilation_options: Default::default(),
            targets: &[],
        }),
        multiview_mask: None,
        cache: None,
    });
    let reduce_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("shadow reduce layout"),
        bind_group_layouts: &[Some(&l.reduce)],
        immediate_size: 0,
    });
    let reduce_pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("shadow reduce"),
        layout: Some(&reduce_layout),
        module: &shadow_sm,
        entry_point: Some("cs_reduce"),
        compilation_options: Default::default(),
        cache: None,
    });
    const LIGHT_ATTRS: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];
    let additive = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    let light_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("lights"),
        layout: Some(&light_layout),
        vertex: wgpu::VertexState {
            module: &light_sm,
            entry_point: Some("vs_light"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<LightInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &LIGHT_ATTRS,
            })],
        },
        primitive: tri,
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &light_sm,
            entry_point: Some("fs_light"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: Some(wgpu::BlendState { color: additive, alpha: additive }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let bloom_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("stickfell bloom"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/bloom.wgsl").into()),
    });
    let post_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("post layout"),
        bind_group_layouts: &[Some(&l.post)],
        immediate_size: 0,
    });
    let post_pipe = |label, entry| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&post_layout),
            vertex: wgpu::VertexState {
                module: &bloom_sm,
                entry_point: Some("vs_full"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: tri,
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &bloom_sm,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    let bloom_down_pipe = post_pipe("bloom downsample", "fs_down");
    let blur_h_pipe = post_pipe("bloom blur h", "fs_blur_h");
    let blur_v_pipe = post_pipe("bloom blur v", "fs_blur_v");
    let comp_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("composite layout"),
        bind_group_layouts: &[Some(&l.composite)],
        immediate_size: 0,
    });
    let composite_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("composite"),
        layout: Some(&comp_layout),
        vertex: wgpu::VertexState {
            module: &comp_sm,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: tri,
        depth_stencil: None,
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState {
            module: &comp_sm,
            entry_point: Some("fs_composite"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    Pipelines {
        shape: shape_pipe,
        splat: splat_pipe,
        resolve: resolve_pipe,
        vapour: vapour_pipe,
        caster: caster_pipe,
        reduce: reduce_pipe,
        ambient: ambient_pipe,
        light: light_pipe,
        bake: bake_pipe,
        ao: ao_pipe,
        composite: composite_pipe,
        bloom_down: bloom_down_pipe,
        blur_h: blur_h_pipe,
        blur_v: blur_v_pipe,
    }
}

impl Renderer {
    /// `output_format` is the format of the texture passed to [`Renderer::render`]
    /// (usually the sRGB swapchain format).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, output_format: wgpu::TextureFormat) -> Self {
        let mut r = Self::preloading(device, queue, output_format);
        r.wait_ready();
        r
    }

    /// Like [`Renderer::new`], but the shaders compile on another thread:
    /// until [`Renderer::ready`], frames draw only their UI (enough for a
    /// loading screen) and the window stays responsive.
    pub fn preloading(device: &wgpu::Device, queue: &wgpu::Queue, output_format: wgpu::TextureFormat) -> Self {
        let shapes_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("stickfell shapes"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shapes.wgsl").into()),
        });

        let cam_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let mk_cam = |label: &str| {
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(&[0.0f32; 20]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &cam_bgl,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
            });
            (buf, bg)
        };
        let (cam_buf, cam_bg) = mk_cam("world camera");
        let (ui_cam_buf, ui_cam_bg) = mk_cam("ui camera");

        // the scene can also draw sprites from an atlas (a blank one until set)
        let sprite_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sprite_samp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sprite sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let blank = crate::SpriteAtlas { width: 1, height: 1, mips: vec![vec![255; 4]] };
        let blank_view = upload_atlas(device, queue, &blank);
        let sprite_bg = sprite_bind_group(device, &sprite_bgl, &sprite_samp, &blank_view);

        let blend_target = |format| {
            Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })
        };
        let tri = wgpu::PrimitiveState::default();


        // the UI also reads the font atlas, and the sprite atlas for pictures
        let font_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let (fw, fh, font_px) = crate::font::atlas();
        let font_size = wgpu::Extent3d { width: fw, height: fh, depth_or_array_layers: 1 };
        let font_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("font atlas"),
            size: font_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &font_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &font_px,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(fw), rows_per_image: None },
            font_size,
        );
        let font_samp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("font sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let font_view = font_tex.create_view(&Default::default());
        let font_bg = ui_bind_group(device, &font_bgl, (&font_view, &font_samp), (&blank_view, &sprite_samp));
        let ui_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui layout"),
            bind_group_layouts: &[Some(&cam_bgl), Some(&font_bgl)],
            immediate_size: 0,
        });
        let ui_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui"),
            layout: Some(&ui_layout),
            vertex: wgpu::VertexState {
                module: &shapes_sm,
                entry_point: Some("vs_shape"),
                compilation_options: Default::default(),
                buffers: &[Some(shape_vertex_layout())],
            },
            primitive: tri,
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shapes_sm,
                entry_point: Some("fs_ui"),
                compilation_options: Default::default(),
                targets: &[blend_target(output_format)],
            }),
            multiview_mask: None,
            cache: None,
        });

        let mk_tex = |label, format, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: SHADOW_W, height: SHADOW_ROWS, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: usage | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let shadow_depth = mk_tex("shadow map", SHADOW_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let shadow_levels = mk_tex("shadow map levels", wgpu::TextureFormat::R32Float, wgpu::TextureUsages::STORAGE_BINDING);
        let depth_entry = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let reduce_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow reduce bgl"),
            entries: &[
                depth_entry(0, wgpu::ShaderStages::COMPUTE),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let reduce_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow reduce"),
            layout: &reduce_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&shadow_depth) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&shadow_levels) },
            ],
        });
        let frag_tex = |binding, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uniform_entry = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let storage_entry = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        // the light pass: shadow maps, this frame's contact shadows, params
        let light_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("light maps bgl"),
            entries: &[
                depth_entry(0, wgpu::ShaderStages::FRAGMENT),
                frag_tex(1, false),
                frag_tex(2, false),
                uniform_entry(3, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("light params"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bake_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("contact bake bgl"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::COMPUTE),
                storage_entry(1, wgpu::ShaderStages::COMPUTE),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let ao_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("contact shadows bgl"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                frag_tex(1, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                storage_entry(3, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let no_bake = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("no contact shadows"),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[255; 4],
            )
            .create_view(&Default::default());



        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let composite_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite bgl"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                tex_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex_entry(4),
            ],
        });
        let post_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post bgl"),
            entries: &[
                tex_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let fluid_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("liquid resolve bgl"),
            entries: &[
                tex_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                tex_entry(3),
            ],
        });
        let vapour_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vapour bgl"),
            entries: &[
                tex_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let vapour_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vapour params"),
            contents: bytemuck::cast_slice(&[0.0f32; VAPOUR_UNIFORM]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let fluid_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("liquid looks"),
            contents: bytemuck::cast_slice(&[0.0f32; FLUID_UNIFORM]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let layouts = Layouts {
            fluid: fluid_bgl.clone(),
            vapour: vapour_bgl.clone(),
            cam: cam_bgl.clone(),
            sprite: sprite_bgl.clone(),
            light: light_bgl.clone(),
            bake: bake_bgl.clone(),
            ao: ao_bgl.clone(),
            reduce: reduce_bgl.clone(),
            post: post_bgl.clone(),
            composite: composite_bgl.clone(),
        };
        let (send, pending) = std::sync::mpsc::channel();
        let dev = device.clone();
        std::thread::Builder::new()
            .name("shader compile".into())
            .spawn(move || {
                let _ = send.send(build_pipelines(&dev, &layouts, output_format));
            })
            .expect("couldn't start the shader compile thread");

        let vb = wgpu::BufferUsages::VERTEX;
        Self {
            pipes: None,
            pending: Some(pending),
            device: device.clone(),
            queue: queue.clone(),
            output_format,
            cam_buf,
            cam_bg,
            ui_cam_buf,
            ui_cam_bg,
            ui_pipe,
            font_bg,
            font_bgl,
            font_view,
            font_samp,
            sprite_bgl,
            sprite_samp,
            sprite_bg,
            sprite_atlas: None,
            shadow_depth,
            reduce_bg,
            shadow_levels,
            light_bgl,
            params_buf,
            bake_bgl,
            ao_bgl,
            bakes: Vec::new(),
            no_bake,
            movers_buf: DynBuffer::new(device, "contact shadow movers", wgpu::BufferUsages::STORAGE),
            movers: Vec::new(),
            composite_bgl,
            post_bgl,
            sampler,
            targets: None,
            shape_buf: DynBuffer::new(device, "shape instances", vb),
            splat_buf: DynBuffer::new(device, "liquid drops", vb),
            fluid_bgl,
            fluid_buf,
            vapour_bgl,
            vapour_buf,
            vapour_tex: None,
            ui_buf: DynBuffer::new(device, "ui instances", vb),
            light_buf: DynBuffer::new(device, "light instances", vb),
            caster_buf: DynBuffer::new(device, "shadow casters", vb),
            casters: Vec::new(),
            light_insts: Vec::new(),
            shadow_rows: 0,
        }
    }

    pub fn output_format(&self) -> wgpu::TextureFormat {
        self.output_format
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    fn ensure_targets(&mut self, w: u32, h: u32) {
        if self.targets.as_ref().is_some_and(|t| t.size == (w, h)) {
            return;
        }
        let mk = |label, format, tw: u32, th: u32, sampled: bool| {
            let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
            if sampled {
                usage |= wgpu::TextureUsages::TEXTURE_BINDING;
            }
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: tw.max(1), height: th.max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let (lw, lh) = ((w / 2).max(1), (h / 2).max(1));
        let albedo = mk("albedo", ALBEDO_FORMAT, w, h, true);
        let emissive = mk("emissive", HDR_FORMAT, w, h, true);
        let light = mk("light", HDR_FORMAT, lw, lh, true);
        let ao = mk("contact shadows", wgpu::TextureFormat::R8Unorm, lw, lh, true);
        let light_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("light maps"),
            layout: &self.light_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.shadow_depth) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.shadow_levels) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&ao) },
                wgpu::BindGroupEntry { binding: 3, resource: self.params_buf.as_entire_binding() },
            ],
        });
        let (bw, bh) = ((w / 4).max(1), (h / 4).max(1));
        let bloom_a = mk("bloom a", HDR_FORMAT, bw, bh, true);
        let bloom_b = mk("bloom b", HDR_FORMAT, bw, bh, true);
        let composite_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("composite bg"),
            layout: &self.composite_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&albedo) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&emissive) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&light) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&bloom_a) },
            ],
        });
        let post = |view: &wgpu::TextureView| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post bg"),
                layout: &self.post_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            })
        };
        let post_bg = [post(&emissive), post(&bloom_a), post(&bloom_b)];
        let fluid = mk("liquid field", FLUID_FORMAT, w, h, true);
        let fluid_dye = mk("liquid dye", FLUID_FORMAT, w, h, true);
        let fluid_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("liquid resolve"),
            layout: &self.fluid_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&fluid) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: self.fluid_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&fluid_dye) },
            ],
        });
        self.targets = Some(Targets { size: (w, h), albedo, emissive, light, bloom_a, bloom_b, composite_bg, post_bg, ao, light_bg, fluid, fluid_dye, fluid_bg });
    }

    /// The vapour's grid into its texture (made, or remade at a new size),
    /// and its params.
    fn upload_vapour(&mut self, v: &crate::VapourField, width: u32, height: u32, frame: &Frame) {
        let (w, h) = (v.w.max(1), v.h.max(1));
        if self.vapour_tex.as_ref().is_none_or(|(size, _, _)| *size != (w, h)) {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("vapour grid"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vapour"),
                layout: &self.vapour_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: self.vapour_buf.as_entire_binding() },
                ],
            });
            self.vapour_tex = Some(((w, h), tex, bg));
        }
        let Some((_, tex, _)) = &self.vapour_tex else { return };
        let n = (w * h) as usize;
        let half: Vec<u16> = (0..n).map(|k| f32_to_f16(v.thick.get(k).copied().unwrap_or(0.0))).collect();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&half),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 2), rows_per_image: None },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let half_view = Vec2::new(frame.camera.height * width.max(1) as f32 / height.max(1) as f32, frame.camera.height) * 0.5;
        let u: [f32; VAPOUR_UNIFORM] = [
            frame.camera.center.x, frame.camera.center.y, half_view.x, half_view.y,
            v.origin.x, v.origin.y, v.cell, frame.time,
            w as f32, h as f32, v.density, v.max_opacity,
        ];
        self.queue.write_buffer(&self.vapour_buf, 0, bytemuck::cast_slice(&u));
    }

    fn build_shadows(&mut self, frame: &Frame, aspect: f32) {
        self.casters.clear();
        self.light_insts.clear();
        self.shadow_rows = 0;
        // lights that can't reach the screen cost nothing
        let half = Vec2::new(frame.camera.height * aspect, frame.camera.height) * 0.5;
        let (lo, hi) = (frame.camera.center - half, frame.camera.center + half);
        let visible = |l: &&Light| {
            let r = Vec2::splat(l.radius);
            (l.pos + r).cmpge(lo).all() && (l.pos - r).cmple(hi).all()
        };
        for l in frame.lights.iter().filter(visible).take(MAX_LIGHTS) {
            let mut row = -1.0;
            if l.shadows && self.shadow_rows < SHADOW_ROWS {
                row = self.shadow_rows as f32;
                self.shadow_rows += 1;
                for o in &frame.occluders {
                    push_casters(&mut self.casters, l, row, o);
                }
            }
            self.light_insts.push(LightInstance {
                p: [l.pos.x, l.pos.y, l.radius, l.cone],
                dir: [l.dir.x, l.dir.y, l.size, row],
                color: [l.color.r, l.color.g, l.color.b, 0.0],
            });
        }
    }

    /// The contact shadows baked for `scene` (baking them now if they haven't
    /// been): its index in `bakes`.
    fn bake(&mut self, scene: &std::sync::Arc<StaticScene>) -> usize {
        if let Some(i) = self.bakes.iter().position(|(s, _)| std::sync::Arc::ptr_eq(s, scene)) {
            return i;
        }
        let size = (scene.max - scene.min).max(Vec2::splat(BAKE_TEXEL));
        let texel = BAKE_TEXEL.max(size.max_element() / BAKE_MAX as f32);
        let (w, h) = ((size.x / texel).ceil().max(1.0) as u32, (size.y / texel).ceil().max(1.0) as u32);
        let mut boxes: Vec<[f32; 4]> = Vec::with_capacity(scene.boxes.len() * 2 + 2);
        for &(c, half, rot) in &scene.boxes {
            boxes.push([c.x, c.y, half.x, half.y]);
            boxes.push([rot.x, rot.y, 0.0, 0.0]);
        }
        // (a storage buffer can't be empty)
        boxes.extend([[0.0; 4]; 2]);
        let mut params = [0u32; 8];
        params[0] = scene.min.x.to_bits();
        params[1] = scene.min.y.to_bits();
        params[2] = texel.to_bits();
        params[3] = (scene.boxes.len() as f32).to_bits();
        params[4] = w;
        params[5] = h;
        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("contact bake params"),
            contents: bytemuck::cast_slice(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let boxes = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("contact bake boxes"),
            contents: bytemuck::cast_slice(&boxes),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("contact shadows (baked)"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("contact bake"),
            layout: &self.bake_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: boxes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view) },
            ],
        });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("contact bake") });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("contact bake"), timestamp_writes: None });
            pass.set_pipeline(&self.pipes.as_ref().expect("bakes run once the shaders are ready").bake);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        self.queue.submit([enc.finish()]);
        if self.bakes.len() >= BAKES_KEPT {
            self.bakes.remove(0);
        }
        let real = Vec2::new(w as f32, h as f32) * texel;
        self.bakes.push((scene.clone(), Bake { view, min: scene.min, size: real }));
        self.bakes.len() - 1
    }

    /// Whether `scene`'s contact shadows are baked already.
    pub fn is_baked(&self, scene: &std::sync::Arc<StaticScene>) -> bool {
        self.bakes.iter().any(|(s, _)| std::sync::Arc::ptr_eq(s, scene))
    }

    /// Moving things on screen (limbs, heads, weapons) for their contact
    /// shadows: segments and circles from the occluders, not the level's boxes.
    fn build_movers(&mut self, frame: &Frame, half_view: Vec2) {
        self.movers.clear();
        let margin = Vec2::splat(0.6);
        let (lo, hi) = (frame.camera.center - half_view - margin, frame.camera.center + half_view + margin);
        let on_screen = |a: Vec2, b: Vec2| a.min(b).cmple(hi).all() && a.max(b).cmpge(lo).all();
        for o in &frame.occluders {
            let (a, b, r) = match *o {
                Occluder::Segment { a, b } => (a, b, LIMB_RADIUS),
                Occluder::Circle { center, radius } => (center, center, radius),
                Occluder::Box { .. } => continue,
            };
            if on_screen(a, b) && self.movers.len() < MAX_MOVERS * 2 {
                self.movers.push([a.x, a.y, b.x, b.y]);
                self.movers.push([r, 0.0, 0.0, 0.0]);
            }
        }
        // (a storage buffer can't be empty: one that never reaches anything)
        if self.movers.is_empty() {
            self.movers.extend([[0.0; 4], [-1e9, 0.0, 0.0, 0.0]]);
        }
    }

    /// Renders `frame` into `target` (which must have `output_format`).
    pub fn render(&mut self, frame: &Frame, target: &wgpu::TextureView, width: u32, height: u32) {
        if let Some(atlas) = &frame.sprite_atlas {
            if !self.sprite_atlas.as_ref().is_some_and(|a| std::sync::Arc::ptr_eq(a, atlas)) {
                let view = upload_atlas(&self.device, &self.queue, atlas);
                self.sprite_bg = sprite_bind_group(&self.device, &self.sprite_bgl, &self.sprite_samp, &view);
                self.font_bg = ui_bind_group(&self.device, &self.font_bgl, (&self.font_view, &self.font_samp), (&view, &self.sprite_samp));
                self.sprite_atlas = Some(atlas.clone());
            }
        }
        let (w, h) = (width.max(1), height.max(1));
        if !self.ready() {
            self.render_ui_only(frame, target, w, h);
            return;
        }
        self.ensure_targets(w, h);
        let aspect = w as f32 / h as f32;
        let cam = frame.camera;
        let world = camera_uniform(
            ortho(cam.center, Vec2::new(cam.height * aspect, cam.height) * 0.5),
            cam.height / h as f32,
            frame.time,
        );
        let ui = camera_uniform(
            ortho(Vec2::new(w as f32, h as f32) * 0.5, Vec2::new(w as f32, -(h as f32)) * 0.5),
            1.0,
            frame.time,
        );
        self.queue.write_buffer(&self.cam_buf, 0, bytemuck::cast_slice(&world));
        self.queue.write_buffer(&self.ui_cam_buf, 0, bytemuck::cast_slice(&ui));

        self.build_shadows(frame, aspect);
        let bake = frame.statics.as_ref().map(|sc| self.bake(sc));
        let half_view = Vec2::new(cam.height * aspect, cam.height) * 0.5;
        self.build_movers(frame, half_view);
        self.movers_buf.upload(&self.device, &self.queue, bytemuck::cast_slice(&self.movers));
        let (bake_min, bake_size) = bake.map_or((Vec2::ZERO, Vec2::ZERO), |i| (self.bakes[i].1.min, self.bakes[i].1.size));
        let a = frame.ambient;
        let params: [f32; 16] = [
            cam.center.x, cam.center.y, half_view.x, half_view.y,
            bake_min.x, bake_min.y, bake_size.x, bake_size.y,
            (self.movers.len() / 2) as f32, DIRECT_CONTACT, 0.0, 0.0,
            a.r, a.g, a.b, 1.0,
        ];
        self.queue.write_buffer(&self.params_buf, 0, bytemuck::cast_slice(&params));
        let baked = bake.map_or(&self.no_bake, |i| &self.bakes[i].1.view);
        let ao_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("contact shadows"),
            layout: &self.ao_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.params_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(baked) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: self.movers_buf.buf.as_entire_binding() },
            ],
        });
        self.shape_buf.upload(&self.device, &self.queue, bytemuck::cast_slice(&frame.shapes));
        self.splat_buf.upload(&self.device, &self.queue, bytemuck::cast_slice(&frame.fluid));
        if !frame.fluid.is_empty() {
            let mut u = [0.0f32; FLUID_UNIFORM];
            for (k, l) in frame.fluid_looks.iter().enumerate() {
                let o = k * FLUID_LOOK;
                u[o..o + 4].copy_from_slice(&l.shallow.to_array());
                u[o + 4..o + 8].copy_from_slice(&l.deep.to_array());
                u[o + 8..o + 12].copy_from_slice(&l.foam.to_array());
                u[o + 12..o + 16].copy_from_slice(&[l.depth, l.foam_width, l.glow, l.sparkle]);
                u[o + 16..o + 20].copy_from_slice(&[l.caustics, l.bubbles, l.crust, l.flow]);
                u[o + 20..o + 24].copy_from_slice(&[l.seed, l.dyed_glow, l.dye_curve, 0.0]);
                u[o + 24..o + 28].copy_from_slice(&l.dyed_shallow.to_array());
                u[o + 28..o + 32].copy_from_slice(&l.dyed_deep.to_array());
            }
            let v = 3 * FLUID_LOOK;
            let half = Vec2::new(frame.camera.height * width.max(1) as f32 / height.max(1) as f32, frame.camera.height) * 0.5;
            u[v..v + 8].copy_from_slice(&[frame.camera.center.x, frame.camera.center.y, half.x, half.y, frame.time, FLUID_THRESHOLD, frame.camera.height / height.max(1) as f32, 0.0]);
            self.queue.write_buffer(&self.fluid_buf, 0, bytemuck::cast_slice(&u));
        }
        if let Some(v) = &frame.vapour {
            self.upload_vapour(v, width, height, frame);
        }
        self.ui_buf.upload(&self.device, &self.queue, bytemuck::cast_slice(&frame.ui));
        self.light_buf.upload(&self.device, &self.queue, bytemuck::cast_slice(&self.light_insts));
        self.caster_buf.upload(&self.device, &self.queue, bytemuck::cast_slice(&self.casters));

        let t = self.targets.as_ref().unwrap();
        let p = self.pipes.as_ref().expect("render runs once the shaders are ready");
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("stickfell frame") });

        let clear = |c: crate::Color| wgpu::Color { r: c.r as f64, g: c.g as f64, b: c.b as f64, a: c.a as f64 };
        let color_att = |view, load| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
            })
        };

        // 1. scene
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[
                    color_att(&t.albedo, wgpu::LoadOp::Clear(clear(frame.clear.alpha(1.0)))),
                    color_att(&t.emissive, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !frame.shapes.is_empty() {
                pass.set_pipeline(&p.shape);
                pass.set_bind_group(0, &self.cam_bg, &[]);
                pass.set_bind_group(1, &self.sprite_bg, &[]);
                pass.set_vertex_buffer(0, self.shape_buf.buf.slice(..));
                pass.draw(0..6, 0..frame.shapes.len() as u32);
            }
        }
        // 1a. liquids: the drops into their field, then that over the scene
        if !frame.fluid.is_empty() {
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("liquid splat"),
                    color_attachments: &[color_att(&t.fluid, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)), color_att(&t.fluid_dye, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT))],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&p.splat);
                pass.set_bind_group(0, &self.cam_bg, &[]);
                pass.set_vertex_buffer(0, self.splat_buf.buf.slice(..));
                pass.draw(0..6, 0..frame.fluid.len() as u32);
            }
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("liquid resolve"),
                color_attachments: &[color_att(&t.albedo, wgpu::LoadOp::Load), color_att(&t.emissive, wgpu::LoadOp::Load)],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&p.resolve);
            pass.set_bind_group(0, &t.fluid_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // 1a'. vapour, over the scene and its liquids
        if let (true, Some((_, _, bg))) = (frame.vapour.is_some(), &self.vapour_tex) {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vapour"),
                color_attachments: &[color_att(&t.albedo, wgpu::LoadOp::Load), color_att(&t.emissive, wgpu::LoadOp::Load)],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&p.vapour);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // 1b. bloom: emissive -> A, then blur A->B->A twice
        {
            let steps: [(&wgpu::RenderPipeline, usize, &wgpu::TextureView); 5] = [
                (&p.bloom_down, 0, &t.bloom_a),
                (&p.blur_h, 1, &t.bloom_b),
                (&p.blur_v, 2, &t.bloom_a),
                (&p.blur_h, 1, &t.bloom_b),
                (&p.blur_v, 2, &t.bloom_a),
            ];
            for (pipe, src, dst) in steps {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bloom"),
                    color_attachments: &[color_att(dst, wgpu::LoadOp::Clear(wgpu::Color::BLACK))],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, &t.post_bg[src], &[]);
                pass.draw(0..3, 0..1);
            }
        }

        // 2. shadow maps (cleared even with nothing in them: the rows are read)
        if self.shadow_rows > 0 {
            {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("shadow casters"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.shadow_depth,
                        depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                if !self.casters.is_empty() {
                    pass.set_pipeline(&p.caster);
                    pass.set_vertex_buffer(0, self.caster_buf.buf.slice(..));
                    pass.draw(0..6, 0..self.casters.len() as u32);
                }
            }
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("shadow reduce"), timestamp_writes: None });
            pass.set_pipeline(&p.reduce);
            pass.set_bind_group(0, &self.reduce_bg, &[]);
            pass.dispatch_workgroups(2016u32.div_ceil(64), self.shadow_rows, 1);
        }

        // 3. contact shadows
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("contact shadows"),
                color_attachments: &[color_att(&t.ao, wgpu::LoadOp::Clear(wgpu::Color::WHITE))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&p.ao);
            pass.set_bind_group(0, &ao_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // 4. lights: ambient (with contact shadows), then each light
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lights"),
                color_attachments: &[color_att(&t.light, wgpu::LoadOp::Clear(wgpu::Color::BLACK))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.cam_bg, &[]);
            pass.set_bind_group(1, &t.light_bg, &[]);
            pass.set_pipeline(&p.ambient);
            pass.draw(0..3, 0..1);
            if !self.light_insts.is_empty() {
                pass.set_pipeline(&p.light);
                pass.set_vertex_buffer(0, self.light_buf.buf.slice(..));
                pass.draw(0..6, 0..self.light_insts.len() as u32);
            }
        }

        // 5. composite
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite"),
                color_attachments: &[color_att(target, wgpu::LoadOp::Clear(wgpu::Color::BLACK))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&p.composite);
            pass.set_bind_group(0, &t.composite_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // 6. UI
        self.ui_pass(&mut enc, frame, target, wgpu::LoadOp::Load);
        self.queue.submit([enc.finish()]);
    }

    /// Draws the frame's UI over `target` (cleared first, or not).
    fn ui_pass(&self, enc: &mut wgpu::CommandEncoder, frame: &Frame, target: &wgpu::TextureView, load: wgpu::LoadOp<wgpu::Color>) {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if !frame.ui.is_empty() {
            pass.set_pipeline(&self.ui_pipe);
            pass.set_bind_group(0, &self.ui_cam_bg, &[]);
            pass.set_bind_group(1, &self.font_bg, &[]);
            pass.set_vertex_buffer(0, self.ui_buf.buf.slice(..));
            pass.draw(0..6, 0..frame.ui.len() as u32);
        }
    }

    /// Whether the shaders have finished compiling (frames draw only their UI
    /// until then).
    pub fn ready(&mut self) -> bool {
        if self.pipes.is_none() {
            if let Some(Ok(p)) = self.pending.as_ref().map(|r| r.try_recv()) {
                self.pipes = Some(p);
                self.pending = None;
            }
        }
        self.pipes.is_some()
    }

    /// Waits for the shaders to finish compiling.
    pub fn wait_ready(&mut self) {
        if let Some(r) = self.pending.take() {
            self.pipes = Some(r.recv().expect("the shader compile thread failed"));
        }
    }

    /// Before the shaders are ready: the UI alone, on black.
    fn render_ui_only(&mut self, frame: &Frame, target: &wgpu::TextureView, w: u32, h: u32) {
        let ui = camera_uniform(ortho(Vec2::new(w as f32, h as f32) * 0.5, Vec2::new(w as f32, -(h as f32)) * 0.5), 1.0, frame.time);
        self.queue.write_buffer(&self.ui_cam_buf, 0, bytemuck::cast_slice(&ui));
        self.ui_buf.upload(&self.device, &self.queue, bytemuck::cast_slice(&frame.ui));
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("stickfell loading frame") });
        self.ui_pass(&mut enc, frame, target, wgpu::LoadOp::Clear(wgpu::Color::BLACK));
        self.queue.submit([enc.finish()]);
    }

    /// Renders `frame` offscreen `n` times and waits for the GPU; returns the
    /// average seconds per frame (GPU throughput, for profiling).
    pub fn bench(&mut self, frame: &Frame, width: u32, height: u32, n: u32) -> f64 {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bench"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.output_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        self.render(frame, &view, width, height);
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("device poll");
        let t = std::time::Instant::now();
        for _ in 0..n {
            self.render(frame, &view, width, height);
        }
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("device poll");
        t.elapsed().as_secs_f64() / n.max(1) as f64
    }

    /// (lights drawn, occluder-from-a-light shadow casters) in the last
    /// rendered frame.
    pub fn last_stats(&self) -> (usize, usize) {
        (self.light_insts.len(), self.casters.len())
    }

    /// Renders into an offscreen texture and reads it back as tightly packed
    /// RGBA8 rows. The renderer's output format must be an 8-bit RGBA format.
    pub fn capture(&mut self, frame: &Frame, width: u32, height: u32) -> Vec<u8> {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.output_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        self.render(frame, &view, width, height);

        let row = width * 4;
        let padded = row.div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture readback"),
            size: (padded * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map capture buffer"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("device poll");
        let data = slice.get_mapped_range().expect("mapped capture range");
        let mut out = Vec::with_capacity((row * height) as usize);
        for y in 0..height as usize {
            let s = y * padded as usize;
            out.extend_from_slice(&data[s..s + row as usize]);
        }
        drop(data);
        buf.unmap();
        if matches!(self.output_format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) {
            for px in out.chunks_exact_mut(4) {
                px.swap(0, 2);
            }
        }
        out
    }
}

/// The directions (an angle span, radians, possibly past +-pi) from `l` that
/// `o` covers, or None when it's out of the light's reach. A light inside a
/// box or circle is shadowed by it all the way round.
fn caster_span(l: &Light, o: &Occluder) -> Option<(f32, f32)> {
    use std::f32::consts::{PI, TAU};
    let around = |c: Vec2, pts: &[Vec2]| {
        let mid = (c - l.pos).to_angle();
        let (mut a0, mut a1) = (0.0f32, 0.0f32);
        for p in pts {
            let a = ((*p - l.pos).to_angle() - mid + PI).rem_euclid(TAU) - PI;
            a0 = a0.min(a);
            a1 = a1.max(a);
        }
        (mid + a0, mid + a1)
    };
    match *o {
        Occluder::Box { center, half, rot } => {
            if (center - l.pos).length() > l.radius + half.length() {
                return None;
            }
            let rel = l.pos - center;
            if rel.dot(rot).abs() <= half.x && rel.dot(rot.perp()).abs() <= half.y {
                return Some((-PI, PI));
            }
            let (ax, ay) = (rot * half.x, rot.perp() * half.y);
            Some(around(center, &[center - ax - ay, center + ax - ay, center + ax + ay, center - ax + ay]))
        }
        Occluder::Segment { a, b } => {
            let (dist, _) = crate::seg_dist(l.pos, a, b);
            if dist > l.radius || dist < 1e-4 {
                return None;
            }
            Some(around((a + b) * 0.5, &[a, b]))
        }
        Occluder::Circle { center, radius } => {
            let d = (center - l.pos).length();
            if d > l.radius + radius {
                return None;
            }
            if d <= radius {
                return Some((-PI, PI));
            }
            let mid = (center - l.pos).to_angle();
            let half = (radius / d).asin();
            Some((mid - half, mid + half))
        }
    }
}

/// Adds `o`, seen from light `l`, to the light's shadow row (split in two
/// where it straddles the +-pi seam).
fn push_casters(out: &mut Vec<CasterInstance>, l: &Light, row: f32, o: &Occluder) {
    use std::f32::consts::{PI, TAU};
    let Some((a0, a1)) = caster_span(l, o) else { return };
    let (kind, g0, g1) = match *o {
        Occluder::Segment { a, b } => (0.0, [a.x, a.y, b.x, b.y], [0.0; 4]),
        Occluder::Box { center, half, rot } => (1.0, [center.x, center.y, half.x, half.y], [rot.x, rot.y, 0.0, 0.0]),
        Occluder::Circle { center, radius } => (2.0, [center.x, center.y, radius, 0.0], [0.0; 4]),
    };
    let light = [l.pos.x, l.pos.y, l.radius, row];
    let mut push = |s: f32, e: f32| out.push(CasterInstance { light, span: [s, e, kind, 0.0], g0, g1 });
    if a1 - a0 >= TAU - 1e-4 {
        push(-PI, PI);
        return;
    }
    let s = (a0 + PI).rem_euclid(TAU) - PI;
    let e = s + (a1 - a0);
    push(s, e.min(PI));
    if e > PI {
        push(-PI, e - TAU);
    }
}

#[cfg(test)]
mod f16_tests {
    #[test]
    fn halves() {
        for (x, want) in [(0.0f32, 0x0000u16), (1.0, 0x3c00), (0.5, 0x3800), (2.0, 0x4000), (65504.0, 0x7bff), (1e9, 0x7bff), (1e-9, 0x0000), (-1.0, 0xbc00)] {
            assert_eq!(super::f32_to_f16(x), want, "{x}");
        }
    }
}
