//! Liquid-glass pass.
//!
//! When a scene contains glass surfaces the frame is drawn into an offscreen texture. Before each
//! batch of glass, that texture is copied to a backdrop texture, which the glass shader blurs and
//! refracts. The finished offscreen texture is blitted to the real frame at the end.

use bytemuck::{Pod, Zeroable};
use gpui::PaintSurface;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlassUniforms {
    rect: [f32; 4],
    misc: [f32; 4],
    shape: [f32; 4],
    tint: [f32; 4],
}

pub(crate) struct GlassResources {
    glass_pipeline: wgpu::RenderPipeline,
    blit_pipeline: wgpu::RenderPipeline,
    glass_layout: wgpu::BindGroupLayout,
    blit_layout: wgpu::BindGroupLayout,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    size: (u32, u32),
    format: wgpu::TextureFormat,
    scene_texture: wgpu::Texture,
    pub(crate) scene_view: wgpu::TextureView,
    backdrop_texture: wgpu::Texture,
    backdrop_view: wgpu::TextureView,
}

impl GlassResources {
    pub(crate) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        let glass_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glass_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("glass.wgsl").into()),
        });
        let blit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("glass_blit_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("glass_blit.wgsl").into()),
        });

        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };

        let glass_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glass_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(1),
                sampler_entry(2),
            ],
        });
        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glass_blit_layout"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });

        let make_pipeline = |name: &str,
                             module: &wgpu::ShaderModule,
                             layout: &wgpu::BindGroupLayout,
                             blend: Option<wgpu::BlendState>| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(name),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vs"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("fs"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let premultiplied = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let glass_pipeline = make_pipeline("glass", &glass_module, &glass_layout, Some(premultiplied));
        // The blit draws a fullscreen triangle with TriangleStrip semantics: 3 vertices cover the screen.
        let blit_pipeline = make_pipeline("glass_blit", &blit_module, &blit_layout, None);

        let sampler = |filter| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: filter,
                min_filter: filter,
                ..Default::default()
            })
        };
        let linear = sampler(wgpu::FilterMode::Linear);
        let nearest = sampler(wgpu::FilterMode::Nearest);

        let (scene_texture, scene_view, backdrop_texture, backdrop_view) =
            Self::create_textures(device, format, width, height);

        Self {
            glass_pipeline,
            blit_pipeline,
            glass_layout,
            blit_layout,
            linear,
            nearest,
            size: (width, height),
            format,
            scene_texture,
            scene_view,
            backdrop_texture,
            backdrop_view,
        }
    }

    fn create_textures(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView, wgpu::Texture, wgpu::TextureView) {
        let make = |label: &str, usage: wgpu::TextureUsages| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: width.max(1),
                    height: height.max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        };
        let (scene_texture, scene_view) = make(
            "glass_scene",
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let (backdrop_texture, backdrop_view) = make(
            "glass_backdrop",
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        (scene_texture, scene_view, backdrop_texture, backdrop_view)
    }

    pub(crate) fn ensure_size(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.size == (width, height) {
            return;
        }
        let (a, b, c, d) = Self::create_textures(device, self.format, width, height);
        self.scene_texture = a;
        self.scene_view = b;
        self.backdrop_texture = c;
        self.backdrop_view = d;
        self.size = (width, height);
    }

    pub(crate) fn matches_format(&self, format: wgpu::TextureFormat) -> bool {
        self.format == format
    }

    /// Snapshots the offscreen scene so the glass can sample what is beneath it.
    pub(crate) fn copy_backdrop(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_texture_to_texture(
            self.scene_texture.as_image_copy(),
            self.backdrop_texture.as_image_copy(),
            wgpu::Extent3d {
                width: self.size.0.max(1),
                height: self.size.1.max(1),
                depth_or_array_layers: 1,
            },
        );
    }

    pub(crate) fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        surface: &PaintSurface,
    ) {
        let b = &surface.bounds;
        let g = &surface.glass;
        let uniforms = GlassUniforms {
            rect: [
                b.origin.x.0,
                b.origin.y.0,
                b.size.width.0,
                b.size.height.0,
            ],
            misc: [
                self.size.0 as f32,
                self.size.1 as f32,
                g.corner_radius,
                g.blur,
            ],
            shape: [g.bezel, g.refraction, g.fade, g.light],
            tint: g.tint,
        };
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("glass_uniforms"),
            size: std::mem::size_of::<GlassUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&uniforms));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glass_bind_group"),
            layout: &self.glass_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.backdrop_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.linear),
                },
            ],
        });
        pass.set_pipeline(&self.glass_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..4, 0..1);
    }

    /// Copies the finished offscreen scene onto the real frame.
    pub(crate) fn blit(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        frame_view: &wgpu::TextureView,
        clear: wgpu::Color,
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("glass_blit_bind_group"),
            layout: &self.blit_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.scene_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.nearest),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("glass_blit_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        pass.set_pipeline(&self.blit_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
