//! wgpu renderer: glyph atlas + one instance per cell + procedural background.
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use winit::window::Window;

use crate::cell::{Cell, Glyphs, MAX_TILES, T_BLANK, T_FIRST_GLYPH, pack};
use crate::font::{Fonts, GlyphTable, special_tiles};
use crate::view::Backend;

const ATLAS: u32 = 2048;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    viewport: [f32; 2],
    cell: [f32; 2],
    pad: [f32; 2],
    opacity: f32,
    tiles_per_row: u32,
    cols: u32,
    cursor: u32,
    _pad: [u32; 2],
}

// --------------------------------------------------------------- renderer --

pub struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    cell_pipe: wgpu::RenderPipeline,
    bg_pipe: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    bind: wgpu::BindGroup,
    ubuf: wgpu::Buffer,
    atlas: wgpu::Texture,
    /// Cell buffer on the GPU: `nrows * cols` cells, row-major. Written per damaged row.
    ibuf: wgpu::Buffer,
    /// Capacity of `ibuf`, in cells.
    icap: usize,
    fonts: Fonts,
    glyphs: GlyphTable,
    next_tile: u32,
    tiles_per_row: u32,
    opacity: f32,
    pub pad: f32,
    /// Packed RGBA of the cursor overlay.
    pub cursor_color: u32,
    nrows: usize,
    cols: usize,
    /// The cell buffer no longer matches the glyph tiles: every row must be written.
    stale: bool,
}

impl Gpu {
    pub fn new(window: Arc<Window>, display: winit::event_loop::OwnedDisplayHandle, font_px: f32, opacity: f32) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(Box::new(display)));
        let surface = instance.create_surface(window.clone()).expect("surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("device");

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
            wgpu::CompositeAlphaMode::PreMultiplied
        } else {
            wgpu::CompositeAlphaMode::Opaque
        };
        let opacity = if alpha_mode == wgpu::CompositeAlphaMode::Opaque { 1.0 } else { opacity };
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 1,
            alpha_mode,
            color_space: wgpu::SurfaceColorSpace::Auto,
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let blend = Some(wgpu::BlendState {
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
        });
        let target = [Some(wgpu::ColorTargetState {
            format,
            blend,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let attrs = wgpu::vertex_attr_array![0 => Uint32, 1 => Uint32, 2 => Uint32];
        let cell_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cell"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_cell"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Cell>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_cell"),
                compilation_options: Default::default(),
                targets: &target,
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let bg_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bg"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_bg"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_bg"),
                compilation_options: Default::default(),
                targets: &target,
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let icap = 1 << 13;
        let ibuf = Self::make_ibuf(&device, icap);
        let fonts = Fonts::new(font_px);
        let (atlas, bind) = Self::make_atlas(&device, &layout, &ubuf);

        let mut g = Gpu {
            surface,
            device,
            queue,
            config,
            cell_pipe,
            bg_pipe,
            layout,
            bind,
            ubuf,
            atlas,
            ibuf,
            icap,
            tiles_per_row: ATLAS / fonts.cell_w,
            fonts,
            glyphs: GlyphTable::new(),
            next_tile: T_FIRST_GLYPH,
            opacity,
            pad: (font_px * 0.9).round(),
            cursor_color: pack(255, 255, 255, 255),
            nrows: 0,
            cols: 0,
            stale: true,
        };
        g.bake_specials();
        g
    }

    fn make_ibuf(device: &wgpu::Device, cap: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (cap * std::mem::size_of::<Cell>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn make_atlas(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        ubuf: &wgpu::Buffer,
    ) -> (wgpu::Texture, wgpu::BindGroup) {
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d { width: ATLAS, height: ATLAS, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = atlas.create_view(&Default::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
            ],
        });
        (atlas, bind)
    }

    fn upload_tile(&self, tile: u32, w: u32, data: &[u8]) {
        let cw = self.fonts.cell_w;
        let ch = self.fonts.cell_h;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.atlas,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: (tile % self.tiles_per_row) * cw,
                    y: (tile / self.tiles_per_row) * ch,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
                },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(ch) },
            wgpu::Extent3d { width: w, height: ch, depth_or_array_layers: 1 },
        );
    }

    fn bake_specials(&mut self) {
        let (w, h) = (self.fonts.cell_w, self.fonts.cell_h);
        for (tile, data) in special_tiles(w, h, self.fonts.px, self.fonts.ascent).iter().enumerate() {
            self.upload_tile(tile as u32, w, data);
        }
    }

    pub fn cell(&self) -> (u32, u32) {
        (self.fonts.cell_w, self.fonts.cell_h)
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.config.width = w.max(1);
        self.config.height = h.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    /// Rebuilds fonts and atlas (scale / font size change).
    pub fn set_font_px(&mut self, px: f32) {
        self.fonts.set_px(px);
        self.tiles_per_row = ATLAS / self.fonts.cell_w;
        self.glyphs.clear();
        self.next_tile = T_FIRST_GLYPH;
        self.pad = (px * 0.9).round();
        self.stale = true;
        let (atlas, bind) = Self::make_atlas(&self.device, &self.layout, &self.ubuf);
        self.atlas = atlas;
        self.bind = bind;
        self.bake_specials();
    }

    /// Glyph tile (for double-width glyphs returns the left one; the right one is +1).
    fn bake(&mut self, c: char, bold: bool, wide: bool) -> u32 {
        if c == ' ' || c == '\0' {
            return T_BLANK;
        }
        let style = bold as u8 | (wide as u8) << 1;
        let baked = self.glyphs.get(c, style);
        if baked != 0 {
            return baked;
        }
        let need = if wide { 2 } else { 1 };
        let cap = (self.tiles_per_row * (ATLAS / self.fonts.cell_h)).min(MAX_TILES);
        if self.next_tile + need > cap {
            return T_BLANK; // atlas full
        }
        let tile = self.next_tile;
        self.next_tile += need;
        let bmp = self.fonts.raster(c, bold, wide);
        let cw = self.fonts.cell_w as usize;
        let ch = self.fonts.cell_h as usize;
        if wide {
            for half in 0..2usize {
                let mut part = vec![0u8; cw * ch];
                for y in 0..ch {
                    part[y * cw..(y + 1) * cw]
                        .copy_from_slice(&bmp[y * cw * 2 + half * cw..y * cw * 2 + (half + 1) * cw]);
                }
                self.upload_tile(tile + half as u32, cw as u32, &part);
            }
        } else {
            self.upload_tile(tile, cw as u32, &bmp);
        }
        self.glyphs.set(c, style, tile);
        tile
    }

    pub fn render(&mut self, clear_alpha: bool) {
        let _ = clear_alpha;
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };
        let view = frame.texture.create_view(&Default::default());

        let u = Uniforms {
            viewport: [self.config.width as f32, self.config.height as f32],
            cell: [self.fonts.cell_w as f32, self.fonts.cell_h as f32],
            pad: [self.pad, self.pad],
            opacity: self.opacity,
            tiles_per_row: self.tiles_per_row,
            cols: self.cols as u32,
            cursor: self.cursor_color,
            _pad: [0; 2],
        };
        self.queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));

        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.bind, &[]);
            pass.set_pipeline(&self.bg_pipe);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&self.cell_pipe);
            pass.set_vertex_buffer(0, self.ibuf.slice(..));
            pass.draw(0..6, 0..(self.nrows * self.cols) as u32);
        }
        self.queue.submit(Some(enc.finish()));
        self.queue.present(frame);
    }
}

impl Glyphs for Gpu {
    fn glyph(&mut self, c: char, bold: bool, wide: bool) -> u32 {
        self.bake(c, bold, wide)
    }
}

impl Backend for Gpu {
    fn begin_frame(&mut self, rows: usize, cols: usize) -> bool {
        let full = self.stale || self.nrows != rows || self.cols != cols;
        if full {
            self.stale = false;
            self.nrows = rows;
            self.cols = cols;
            if rows * cols > self.icap {
                self.icap = (rows * cols).next_power_of_two();
                self.ibuf = Self::make_ibuf(&self.device, self.icap);
            }
        }
        full
    }

    fn put_rows(&mut self, first: usize, count: usize, cells: &[Cell]) {
        if first + count > self.nrows {
            return;
        }
        let offset = (first * self.cols * std::mem::size_of::<Cell>()) as u64;
        self.queue.write_buffer(&self.ibuf, offset, bytemuck::cast_slice(cells));
    }
}
