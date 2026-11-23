//! wgpu renderer: glyph atlas + one instance per cell + procedural background.
use std::{cell::OnceCell, sync::Arc};

use bytemuck::{Pod, Zeroable};
use fontdue::{Font, FontSettings};
use winit::window::Window;

const ATLAS: u32 = 2048;

pub const T_BLANK: u32 = 0;
const T_BEAM: u32 = 1;
const T_BAR: u32 = 2;
const T_BOX: u32 = 3;
const T_UNDER: u32 = 4;
const T_STRIKE: u32 = 5;
const T_FIRST_GLYPH: u32 = 6;
/// The tile index lives in the low 16 bits of `Cell::tf`.
const MAX_TILES: u32 = 1 << 16;

// Flags in the high 16 bits of `Cell::tf`; the shader (shader.wgsl) draws them.
pub const F_UNDER: u32 = 1 << 16;
pub const F_STRIKE: u32 = 1 << 17;
/// Cursor shape: a 2-bit field at bits 18-19.
pub const F_BEAM: u32 = 1 << 18;
pub const F_BAR: u32 = 2 << 18;
pub const F_BOX: u32 = 3 << 18;

/// One terminal cell as the GPU reads it. The cell buffer is a flat array indexed by
/// `row * cols + col`: the position is the index, so it is not stored.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Cell {
    /// Tile in the low 16 bits, `F_*` flags above.
    pub tf: u32,
    pub fg: u32,
    pub bg: u32,
}

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

pub fn pack(r: u8, g: u8, b: u8, a: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16 | (a as u32) << 24
}

// ------------------------------------------------------------------ fonts --

/// Fallback font, parsed on first use. fontdue parses every glyph eagerly, so a CJK font costs
/// ~300 MB and ~600 ms; `ranges` (inclusive code points, empty = any) keeps it from being
/// consulted, and therefore loaded, for characters it cannot help with.
struct Fallback {
    path: &'static str,
    ranges: &'static [(u32, u32)],
    font: OnceCell<Option<Font>>,
}

const CJK: &[(u32, u32)] = &[
    (0x2E80, 0x9FFF),   // radicals, CJK symbols, kana, bopomofo, enclosed forms, ideographs
    (0xAC00, 0xD7FF),   // hangul syllables
    (0xF900, 0xFAFF),   // compatibility ideographs
    (0xFE30, 0xFE4F),   // compatibility forms
    (0xFF00, 0xFFEF),   // half/full-width forms
    (0x20000, 0x3FFFF), // ideograph extensions
];

const FALLBACKS: &[(&str, &[(u32, u32)])] = &[
    ("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", &[]),
    ("/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf", &[]),
    ("/usr/share/fonts/truetype/noto/NotoSansSymbols-Regular.ttf", &[]),
    ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", CJK),
];

struct Fonts {
    primary: Font,
    fallbacks: Vec<Fallback>,
    px: f32,
    pub cell_w: u32,
    pub cell_h: u32,
    ascent: i32,
}

fn load_font(path: &str) -> Option<Font> {
    let bytes = std::fs::read(path).ok()?;
    Font::from_bytes(bytes, FontSettings::default()).ok()
}

impl Fonts {
    fn new(px: f32) -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let mut candidates: Vec<String> = Vec::new();
        if let Ok(p) = std::env::var("AXON_FONT") {
            candidates.push(p);
        }
        for f in ["CascadiaMonoNF.ttf", "JetBrainsMono[wght].ttf", "CascadiaMono.ttf"] {
            candidates.push(format!("{home}/.local/share/fonts/{f}"));
        }
        for f in [
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
            "/usr/share/fonts/truetype/ubuntu/UbuntuMono-R.ttf",
        ] {
            candidates.push(f.to_string());
        }
        let primary = candidates
            .iter()
            .find_map(|p| load_font(p))
            .expect("no monospaced font found (set AXON_FONT=/path/to/font.ttf)");

        let fallbacks = FALLBACKS
            .iter()
            .map(|&(path, ranges)| Fallback { path, ranges, font: OnceCell::new() })
            .collect();

        let mut fonts = Fonts { primary, fallbacks, px, cell_w: 1, cell_h: 1, ascent: 0 };
        fonts.set_px(px);
        fonts
    }

    /// Recomputes the cell metrics for a new size; no font file is read again.
    fn set_px(&mut self, px: f32) {
        let lm = self.primary.horizontal_line_metrics(px).expect("font has no line metrics");
        self.px = px;
        self.cell_w = self.primary.metrics('M', px).advance_width.round().max(1.0) as u32;
        self.cell_h = (lm.ascent - lm.descent).ceil().max(1.0) as u32;
        self.ascent = lm.ascent.round() as i32;
    }

    /// First fallback font that has `c`, loading fallbacks lazily and in order.
    fn fallback_for(&self, c: char) -> Option<&Font> {
        let cp = c as u32;
        self.fallbacks.iter().find_map(|f| {
            if !f.ranges.is_empty() && !f.ranges.iter().any(|&(lo, hi)| (lo..=hi).contains(&cp)) {
                return None;
            }
            f.font.get_or_init(|| load_font(f.path)).as_ref().filter(|font| font.lookup_glyph_index(c) != 0)
        })
    }

    /// Rasterizes a glyph into a `cell_w * (1|2)` x `cell_h` bitmap.
    fn raster(&self, c: char, bold: bool, wide: bool) -> Vec<u8> {
        let w = (self.cell_w * if wide { 2 } else { 1 }) as i32;
        let h = self.cell_h as i32;
        let mut buf = vec![0u8; (w * h) as usize];

        let (font, fb) = if self.primary.lookup_glyph_index(c) != 0 {
            (&self.primary, false)
        } else if let Some(f) = self.fallback_for(c) {
            (f, true)
        } else {
            (&self.primary, false)
        };
        let (m, bmp) = font.rasterize(c, self.px);
        let xoff = if fb { ((w as f32 - m.advance_width) / 2.0).round() as i32 } else { 0 };
        for r in 0..m.height as i32 {
            let y = self.ascent - m.ymin - m.height as i32 + r;
            if y < 0 || y >= h {
                continue;
            }
            for col in 0..m.width as i32 {
                let x = m.xmin + col + xoff;
                if x < 0 || x >= w {
                    continue;
                }
                buf[(y * w + x) as usize] = bmp[(r * m.width as i32 + col) as usize];
            }
        }
        if bold {
            for y in 0..h {
                for x in (1..w).rev() {
                    let i = (y * w + x) as usize;
                    buf[i] = buf[i].max(buf[i - 1]);
                }
            }
        }
        buf
    }
}

// ------------------------------------------------------------ glyph table --

/// Tile of each baked (code point, style): a two-level flat table, no hashing. Code point `cp`
/// lives in page `cp >> 8`, slot `(cp & 255) << 2 | style`. A tile of 0 means "not baked":
/// glyph tiles start at `T_FIRST_GLYPH`, so 0 is never a real glyph.
struct GlyphTable {
    dir: Vec<u16>,
    pages: Vec<[u32; 1024]>,
}

impl GlyphTable {
    fn new() -> Self {
        GlyphTable { dir: vec![0; 0x1100], pages: Vec::new() }
    }

    #[inline]
    fn get(&self, c: char, style: u8) -> u32 {
        let cp = c as usize;
        match self.dir[cp >> 8] {
            0 => 0,
            p => self.pages[p as usize - 1][(cp & 255) << 2 | style as usize],
        }
    }

    fn set(&mut self, c: char, style: u8, tile: u32) {
        let cp = c as usize;
        if self.dir[cp >> 8] == 0 {
            self.pages.push([0; 1024]);
            self.dir[cp >> 8] = self.pages.len() as u16;
        }
        let page = self.dir[cp >> 8] as usize - 1;
        self.pages[page][(cp & 255) << 2 | style as usize] = tile;
    }

    fn clear(&mut self) {
        self.dir.fill(0);
        self.pages.clear();
    }
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
    /// Scratch for the rows being written; dead once uploaded.
    stage: Vec<Cell>,
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
            stage: Vec::new(),
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
        let (w, h) = (self.fonts.cell_w as usize, self.fonts.cell_h as usize);
        let t = ((self.fonts.px / 14.0).round() as usize).max(1);
        let base = self.fonts.ascent as usize;
        let mk = |f: &dyn Fn(usize, usize) -> bool| -> Vec<u8> {
            let mut v = vec![0u8; w * h];
            for y in 0..h {
                for x in 0..w {
                    if f(x, y) {
                        v[y * w + x] = 255;
                    }
                }
            }
            v
        };
        let tiles = [
            (T_BLANK, mk(&|_, _| false)),
            (T_BEAM, mk(&|x, _| x < t.max(2))),
            (T_BAR, mk(&|_, y| y + t.max(2) >= h)),
            (T_BOX, mk(&|x, y| x < t || y < t || x + t >= w || y + t >= h)),
            (T_UNDER, mk(&|_, y| y >= (base + 1).min(h - t) && y < (base + 1).min(h - t) + t)),
            (T_STRIKE, mk(&|_, y| y >= h * 55 / 100 && y < h * 55 / 100 + t)),
        ];
        for (tile, data) in tiles {
            self.upload_tile(tile, w as u32, &data);
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
    pub fn glyph(&mut self, c: char, bold: bool, wide: bool) -> u32 {
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

    /// Starts a frame for a grid of `rows` x `cols` cells. Returns true when every row has to be
    /// written: first frame, the grid size changed, or the glyph tiles were reassigned.
    pub fn begin_frame(&mut self, rows: usize, cols: usize) -> bool {
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

    /// Writes `count` consecutive rows starting at `first`. `fill` receives `count * cols` cells
    /// (zeroed: blank) and must set every cell that is not blank.
    pub fn write_rows(&mut self, first: usize, count: usize, fill: impl FnOnce(&mut Gpu, &mut [Cell])) {
        if first + count > self.nrows {
            return;
        }
        let mut stage = std::mem::take(&mut self.stage);
        stage.clear();
        stage.resize(count * self.cols, Cell::zeroed());
        fill(self, &mut stage);
        let offset = (first * self.cols * std::mem::size_of::<Cell>()) as u64;
        self.queue.write_buffer(&self.ibuf, offset, bytemuck::cast_slice(&stage));
        self.stage = stage;
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
