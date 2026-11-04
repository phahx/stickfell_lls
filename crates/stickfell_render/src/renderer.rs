//! GPU side: pipelines, render targets and the per-frame pass sequence.
//!
//! Passes:
//! 1. **Scene** — SDF shapes into `albedo` (sRGB) + `emissive` (HDR) targets.
//! 2. **Lights** — half-res HDR light buffer cleared to ambient. For each
//!    light: its shadow volumes are written into the stencil buffer with a
//!    unique id, then the light quad is added where stencil != id.
//! 3. **Composite** — `albedo * light + emissive (+glow)`, tonemapped, into
//!    the output target.
//! 4. **UI** — unlit screen-space shapes/text on top.

use bytemuck::{Pod, Zeroable};
use glam::Vec2;
use wgpu::util::DeviceExt;

use crate::frame::{Frame, Light, Occluder, ShapeInstance};

const ALBEDO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const STENCIL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Stencil8;
const MAX_LIGHTS: usize = 255;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightInstance {
    p: [f32; 4],
    dir: [f32; 4],
    color: [f32; 4],
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
    stencil: wgpu::TextureView,
    bloom_a: wgpu::TextureView,
    bloom_b: wgpu::TextureView,
    composite_bg: wgpu::BindGroup,
    /// Post-process sources: emissive, bloom_a, bloom_b.
    post_bg: [wgpu::BindGroup; 3],
}

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    output_format: wgpu::TextureFormat,
    cam_buf: wgpu::Buffer,
    cam_bg: wgpu::BindGroup,
    ui_cam_buf: wgpu::Buffer,
    ui_cam_bg: wgpu::BindGroup,
    shape_pipe: wgpu::RenderPipeline,
    ui_pipe: wgpu::RenderPipeline,
    font_bg: wgpu::BindGroup,
    sprite_bgl: wgpu::BindGroupLayout,
    sprite_samp: wgpu::Sampler,
    sprite_bg: wgpu::BindGroup,
    /// The atlas currently on the GPU (compared by pointer).
    sprite_atlas: Option<std::sync::Arc<crate::SpriteAtlas>>,
    shadow_pipe: wgpu::RenderPipeline,
    light_pipe: wgpu::RenderPipeline,
    composite_pipe: wgpu::RenderPipeline,
    composite_bgl: wgpu::BindGroupLayout,
    bloom_down_pipe: wgpu::RenderPipeline,
    blur_h_pipe: wgpu::RenderPipeline,
    blur_v_pipe: wgpu::RenderPipeline,
    post_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    targets: Option<Targets>,
    shape_buf: DynBuffer,
    ui_buf: DynBuffer,
    light_buf: DynBuffer,
    shadow_buf: DynBuffer,
    shadow_verts: Vec<[f32; 2]>,
    light_insts: Vec<LightInstance>,
    light_ranges: Vec<(u32, u32)>,
}

/// Uploads a sprite atlas (with its mips) and binds it for the scene pass.
fn sprite_bind_group(device: &wgpu::Device, queue: &wgpu::Queue, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, atlas: &crate::SpriteAtlas) -> wgpu::BindGroup {
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
    let view = tex.create_view(&Default::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sprites"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(sampler) },
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

impl Renderer {
    /// `output_format` is the format of the texture passed to [`Renderer::render`]
    /// (usually the sRGB swapchain format).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, output_format: wgpu::TextureFormat) -> Self {
        let shapes_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("stickfell shapes"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shapes.wgsl").into()),
        });
        let light_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("stickfell lighting"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/lighting.wgsl").into()),
        });
        let comp_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("stickfell composite"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/composite.wgsl").into()),
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

        let cam_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("camera layout"),
            bind_group_layouts: &[Some(&cam_bgl)],
            immediate_size: 0,
        });

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
        let sprite_bg = sprite_bind_group(device, queue, &sprite_bgl, &sprite_samp, &blank);
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene layout"),
            bind_group_layouts: &[Some(&cam_bgl), Some(&sprite_bgl)],
            immediate_size: 0,
        });

        let blend_target = |format| {
            Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })
        };
        let tri = wgpu::PrimitiveState::default();

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

        // the UI also reads the font atlas
        let font_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("font bgl"),
            entries: &[
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
        let font_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("font"),
            layout: &font_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&font_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&font_samp) },
            ],
        });
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

        let face = |compare, pass_op| wgpu::StencilFaceState {
            compare,
            fail_op: wgpu::StencilOperation::Keep,
            depth_fail_op: wgpu::StencilOperation::Keep,
            pass_op,
        };
        let stencil_state = |compare, pass_op, write_mask| wgpu::DepthStencilState {
            format: STENCIL_FORMAT,
            depth_write_enabled: None,
            depth_compare: None,
            stencil: wgpu::StencilState {
                front: face(compare, pass_op),
                back: face(compare, pass_op),
                read_mask: 0xff,
                write_mask,
            },
            bias: Default::default(),
        };

        const SHADOW_ATTRS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x2];
        let shadow_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow volumes"),
            layout: Some(&cam_layout),
            vertex: wgpu::VertexState {
                module: &light_sm,
                entry_point: Some("vs_shadow"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 8,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &SHADOW_ATTRS,
                })],
            },
            primitive: tri,
            depth_stencil: Some(stencil_state(
                wgpu::CompareFunction::Always,
                wgpu::StencilOperation::Replace,
                0xff,
            )),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &light_sm,
                entry_point: Some("fs_shadow"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
            }),
            multiview_mask: None,
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
            layout: Some(&cam_layout),
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
            depth_stencil: Some(stencil_state(
                wgpu::CompareFunction::NotEqual,
                wgpu::StencilOperation::Keep,
                0,
            )),
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
        let bloom_sm = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("stickfell bloom"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/bloom.wgsl").into()),
        });
        let post_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post layout"),
            bind_group_layouts: &[Some(&post_bgl)],
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
            bind_group_layouts: &[Some(&composite_bgl)],
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

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let vb = wgpu::BufferUsages::VERTEX;
        Self {
            device: device.clone(),
            queue: queue.clone(),
            output_format,
            cam_buf,
            cam_bg,
            ui_cam_buf,
            ui_cam_bg,
            shape_pipe,
            ui_pipe,
            font_bg,
            sprite_bgl,
            sprite_samp,
            sprite_bg,
            sprite_atlas: None,
            shadow_pipe,
            light_pipe,
            composite_pipe,
            composite_bgl,
            bloom_down_pipe,
            blur_h_pipe,
            blur_v_pipe,
            post_bgl,
            sampler,
            targets: None,
            shape_buf: DynBuffer::new(device, "shape instances", vb),
            ui_buf: DynBuffer::new(device, "ui instances", vb),
            light_buf: DynBuffer::new(device, "light instances", vb),
            shadow_buf: DynBuffer::new(device, "shadow volumes", vb),
            shadow_verts: Vec::new(),
            light_insts: Vec::new(),
            light_ranges: Vec::new(),
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
        let stencil = mk("shadow stencil", STENCIL_FORMAT, lw, lh, false);
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
        self.targets = Some(Targets { size: (w, h), albedo, emissive, light, stencil, bloom_a, bloom_b, composite_bg, post_bg });
    }

    fn build_shadows(&mut self, frame: &Frame, aspect: f32) {
        self.shadow_verts.clear();
        self.light_insts.clear();
        self.light_ranges.clear();
        // lights that can't reach the screen cost nothing
        let half = Vec2::new(frame.camera.height * aspect, frame.camera.height) * 0.5;
        let (lo, hi) = (frame.camera.center - half, frame.camera.center + half);
        let visible = |l: &&Light| {
            let r = Vec2::splat(l.radius);
            (l.pos + r).cmpge(lo).all() && (l.pos - r).cmple(hi).all()
        };
        for l in frame.lights.iter().filter(visible).take(MAX_LIGHTS) {
            let start = self.shadow_verts.len() as u32;
            if l.shadows {
                for o in &frame.occluders {
                    extrude_occluder(&mut self.shadow_verts, l, o);
                }
            }
            self.light_ranges.push((start, self.shadow_verts.len() as u32));
            self.light_insts.push(LightInstance {
                p: [l.pos.x, l.pos.y, l.radius, l.cone],
                dir: [l.dir.x, l.dir.y, 0.0, 0.0],
                color: [l.color.r, l.color.g, l.color.b, 0.0],
            });
        }
    }

    /// Renders `frame` into `target` (which must have `output_format`).
    pub fn render(&mut self, frame: &Frame, target: &wgpu::TextureView, width: u32, height: u32) {
        if let Some(atlas) = &frame.sprite_atlas {
            if !self.sprite_atlas.as_ref().is_some_and(|a| std::sync::Arc::ptr_eq(a, atlas)) {
                self.sprite_bg = sprite_bind_group(&self.device, &self.queue, &self.sprite_bgl, &self.sprite_samp, atlas);
                self.sprite_atlas = Some(atlas.clone());
            }
        }
        let (w, h) = (width.max(1), height.max(1));
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
        self.shape_buf.upload(&self.device, &self.queue, bytemuck::cast_slice(&frame.shapes));
        self.ui_buf.upload(&self.device, &self.queue, bytemuck::cast_slice(&frame.ui));
        self.light_buf.upload(&self.device, &self.queue, bytemuck::cast_slice(&self.light_insts));
        self.shadow_buf.upload(&self.device, &self.queue, bytemuck::cast_slice(&self.shadow_verts));

        let t = self.targets.as_ref().unwrap();
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
                pass.set_pipeline(&self.shape_pipe);
                pass.set_bind_group(0, &self.cam_bg, &[]);
                pass.set_bind_group(1, &self.sprite_bg, &[]);
                pass.set_vertex_buffer(0, self.shape_buf.buf.slice(..));
                pass.draw(0..6, 0..frame.shapes.len() as u32);
            }
        }

        // 1b. bloom: emissive -> A, then blur A->B->A twice
        {
            let steps: [(&wgpu::RenderPipeline, usize, &wgpu::TextureView); 5] = [
                (&self.bloom_down_pipe, 0, &t.bloom_a),
                (&self.blur_h_pipe, 1, &t.bloom_b),
                (&self.blur_v_pipe, 2, &t.bloom_a),
                (&self.blur_h_pipe, 1, &t.bloom_b),
                (&self.blur_v_pipe, 2, &t.bloom_a),
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

        // 2. lights + shadows
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lights"),
                color_attachments: &[color_att(&t.light, wgpu::LoadOp::Clear(clear(frame.ambient.alpha(1.0))))],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.stencil,
                    depth_ops: None,
                    stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0), store: wgpu::StoreOp::Discard }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.cam_bg, &[]);
            for (i, &(s, e)) in self.light_ranges.iter().enumerate() {
                let id = i as u32 + 1;
                pass.set_stencil_reference(id);
                if e > s {
                    pass.set_pipeline(&self.shadow_pipe);
                    pass.set_vertex_buffer(0, self.shadow_buf.buf.slice(..));
                    pass.draw(s..e, 0..1);
                }
                pass.set_pipeline(&self.light_pipe);
                pass.set_vertex_buffer(0, self.light_buf.buf.slice(..));
                pass.draw(0..6, i as u32..i as u32 + 1);
            }
        }

        // 3. composite
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite"),
                color_attachments: &[color_att(target, wgpu::LoadOp::Clear(wgpu::Color::BLACK))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.composite_pipe);
            pass.set_bind_group(0, &t.composite_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // 4. UI
        if !frame.ui.is_empty() {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui"),
                color_attachments: &[color_att(target, wgpu::LoadOp::Load)],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.ui_pipe);
            pass.set_bind_group(0, &self.ui_cam_bg, &[]);
            pass.set_bind_group(1, &self.font_bg, &[]);
            pass.set_vertex_buffer(0, self.ui_buf.buf.slice(..));
            pass.draw(0..6, 0..frame.ui.len() as u32);
        }

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

    /// (lights drawn, shadow-volume vertices) in the last rendered frame.
    pub fn last_stats(&self) -> (usize, usize) {
        (self.light_insts.len(), self.shadow_verts.len())
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

fn push_quad(out: &mut Vec<[f32; 2]>, l: Vec2, p0: Vec2, p1: Vec2, far: f32) {
    let d0 = p0 - l;
    let d1 = p1 - l;
    if d0.length_squared() < 1e-8 || d1.length_squared() < 1e-8 {
        return;
    }
    let q0 = p0 + d0.normalize() * far;
    let q1 = p1 + d1.normalize() * far;
    out.extend_from_slice(&[p0.into(), p1.into(), q1.into(), p0.into(), q1.into(), q0.into()]);
}

/// Appends shadow-volume triangles cast by `o` from light `l`.
fn extrude_occluder(out: &mut Vec<[f32; 2]>, l: &Light, o: &Occluder) {
    // far enough to cover the light's square quad (corners at r * sqrt 2)
    // and no further: longer volumes only add stencil fill
    let far = l.radius * 1.42;
    match *o {
        Occluder::Box { center, half, rot } => {
            if (center - l.pos).length() > l.radius + half.length() {
                return;
            }
            let ax = rot * half.x;
            let ay = rot.perp() * half.y;
            let c = [center - ax - ay, center + ax - ay, center + ax + ay, center - ax + ay];
            for k in 0..4 {
                let (p0, p1) = (c[k], c[(k + 1) % 4]);
                let e = p1 - p0;
                let outward = Vec2::new(e.y, -e.x); // CCW polygon
                // Only back faces cast, so the lit face of the box stays lit.
                if outward.dot((p0 + p1) * 0.5 - l.pos) > 0.0 {
                    push_quad(out, l.pos, p0, p1, far);
                }
            }
        }
        Occluder::Segment { a, b } => {
            let (dist, _) = crate::seg_dist(l.pos, a, b);
            if dist > l.radius {
                return;
            }
            push_quad(out, l.pos, a, b, far);
        }
        Occluder::Circle { center, radius } => {
            let d = center - l.pos;
            if d.length() > l.radius + radius || d.length_squared() < 1e-8 {
                return;
            }
            let side = d.normalize().perp() * radius;
            push_quad(out, l.pos, center - side, center + side, far);
        }
    }
}
