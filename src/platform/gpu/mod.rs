//! From-scratch wgpu renderer.
//!
//! Everything the editor draws — panels, buttons, carets, selection, minimap bars,
//! borders and text glyphs — is a textured quad. All quads for a frame are packed
//! into one instance buffer and issued with a single draw call, with clipping done
//! in the vertex shader (no extra render passes or scissor state changes).

pub mod atlas;

/// A hardware adapter for `surface`, if there is one.
///
/// Split out so the window path can try again with a software rasteriser rather
/// than dying: `request_adapter` returns an error on a machine with no Vulkan
/// driver installed, which is a normal state for a fresh Linux install.
/// The graphics backends to try: everything the platform offers, unless
/// `BAREMD_GPU_BACKEND` names one.
///
/// Device creation is the one part of a launch that is entirely the driver's, and it
/// is where a slow start-up lives on a machine whose default backend is slow (a software
/// rasteriser, a driver that only speaks OpenGL badly, a hybrid-GPU laptop switching
/// adapters). Being able to say `BAREMD_GPU_BACKEND=vulkan` - or `dx12`, `gl`, `metal` -
/// turns "it takes a second" into a five-second experiment instead of a guess.
fn backends() -> wgpu::Backends {
    match std::env::var("BAREMD_GPU_BACKEND").ok().as_deref().map(str::trim) {
        Some(v) if v.eq_ignore_ascii_case("vulkan") => wgpu::Backends::VULKAN,
        Some(v) if v.eq_ignore_ascii_case("dx12") => wgpu::Backends::DX12,
        Some(v) if v.eq_ignore_ascii_case("metal") => wgpu::Backends::METAL,
        Some(v) if v.eq_ignore_ascii_case("gl") || v.eq_ignore_ascii_case("opengl") => {
            wgpu::Backends::GL
        }
        _ => wgpu::Backends::all(),
    }
}

fn new_instance() -> wgpu::Instance {
    let backends = backends();
    // `new_without_display_handle_from_env` still honours the usual `WGPU_BACKEND`
    // variable; this only narrows the list further.
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    desc.backends = backends;
    wgpu::Instance::new(desc)
}

fn request_adapter(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'static>>,
) -> Option<wgpu::Adapter> {
    pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: surface,
        force_fallback_adapter: false,
        ..Default::default()
    }))
    .ok()
}

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use winit::window::Window;

use crate::geom::{Color, Rect};
use crate::text::{Face, TextEngine};
use atlas::{Atlas, PixPool};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Instance {
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub color: [f32; 4],
    pub clip: [f32; 4],
    pub params: [f32; 4],
    pub border_color: [f32; 4],
}

/// A rasterised icon, cached in the atlas.
#[derive(Clone, Copy)]
pub struct IconTex {
    pub uv: [f32; 4],
    pub w: u32,
    pub h: u32,
}

/// An uploaded image, cached in the pixel pool.
#[derive(Clone, Copy)]
pub struct ImageTex {
    pub uv: [f32; 4],
    pub w: u32,
    pub h: u32,
    /// Display size chosen for the document, in pixels of the source image.
    pub src: (u32, u32),
}

#[derive(Clone, Copy)]
struct GlyphTex {
    uv: [f32; 4],
    w: u32,
    h: u32,
    left: i32,
    top: i32,
}

#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub instances: usize,
    pub glyph_quads: usize,
    pub rasterized: usize,
    pub cached_glyphs: usize,
}

enum Target {
    Surface(wgpu::Surface<'static>),
    Offscreen(wgpu::Texture),
}

pub struct Renderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    target: Target,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    globals: wgpu::Buffer,
    instances: Vec<Instance>,
    instance_buf: wgpu::Buffer,
    instance_cap: usize,
    atlas: Atlas,
    /// RGBA pool for images; the same bind group samples it.
    pix: PixPool,
    icons: HashMap<(String, u32), Option<IconTex>>,
    images: HashMap<PathBuf, Option<ImageTex>>,
    glyphs: HashMap<(u8, u16, u32), GlyphTex>,
    white: [f32; 4],
    pub text: TextEngine,
    /// Physical size of the swapchain.
    pub size: (f32, f32),
    pub scale: f32,
    clip: Rect,
    clip_stack: Vec<Rect>,
    clear: Color,
    pub stats: Stats,
    pub screenshot: Option<PathBuf>,
    pub screenshot_frames: u32,
    /// When set, the next `end()` reads the finished frame back into
    /// [`Renderer::last_frame`] instead of writing it to a file. Tests ask the
    /// pixels what was painted; everything else about the frame is unchanged.
    pub capture: bool,
    /// The pixels of the last captured frame, RGBA8, row-major.
    pub last_frame: Option<(u32, u32, Vec<u8>)>,
    pub frame: u64,
    pub exit_after_screenshot: bool,
}

impl Renderer {
    /// Renders into a texture instead of a window (tests, thumbnails, CI).
    pub fn offscreen(width: u32, height: u32, scale: f32) -> Self {
        Self::try_offscreen(width, height, scale).expect("no suitable GPU adapter (Vulkan or OpenGL)")
    }

    /// [`Renderer::offscreen`], but reporting a machine with no graphics device
    /// instead of panicking - the headless `--render` path prints a message.
    pub fn try_offscreen(width: u32, height: u32, scale: f32) -> Option<Self> {
        let instance = new_instance();
        let adapter = request_adapter(&instance, None)
            .or_else(|| {
                // The fallback used to be asked for twice, identically, which doubled
                // the wait on a machine that has no hardware adapter at all.
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    compatible_surface: None,
                    force_fallback_adapter: true,
                    ..Default::default()
                }))
                .ok()
            })?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        let format = wgpu::TextureFormat::Bgra8Unorm;
        let (target, config) = Self::offscreen_target(&device, format, width, height);
        Some(Self::build(device, queue, target, config, scale, None, false))
    }

    fn offscreen_target(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (Target, wgpu::SurfaceConfiguration) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen-target"),
            size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Srgb,
        };
        (Target::Offscreen(texture), config)
    }

    /// Build a renderer for a window.
    ///
    /// `None` means this machine cannot draw at all - no Vulkan, no OpenGL, no
    /// usable driver. The caller says so and exits; a panic here would leave the
    /// user with a backtrace instead of a sentence telling them what to install.
    pub fn new(
        window: Arc<Window>,
        screenshot: Option<PathBuf>,
        exit_after_screenshot: bool,
    ) -> Option<Self> {
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let instance = new_instance();
        let surface = match instance.create_surface(window.clone()) {
            Ok(s) => s,
            Err(e) => {
                log::error!("cannot create a drawing surface for the window: {e}");
                return None;
            }
        };
        // A normal adapter first, then any adapter at all: the second pass is what
        // finds a software rasteriser (lavapipe) or an OpenGL-only driver on a
        // machine with no Vulkan - which is exactly a freshly installed Linux box.
        let adapter = request_adapter(&instance, Some(&surface))
            .or_else(|| {
                log::warn!("no hardware adapter; trying any adapter, software included");
                pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: true,
                    ..Default::default()
                }))
                .ok()
            });
        let Some(adapter) = adapter else {
            log::error!("no usable graphics device (Vulkan or OpenGL) was found");
            return None;
        };

        let mut limits = wgpu::Limits::default();
        limits.max_texture_dimension_2d = limits.max_texture_dimension_2d.max(4096);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("bare-md"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            ..Default::default()
        }))
        .expect("device");

        let caps = surface.get_capabilities(&adapter);
        // Prefer a non-sRGB swapchain so hex colours land on screen verbatim.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Srgb,
        };
        surface.configure(&device, &config);
        Some(Self::build(
            device,
            queue,
            Target::Surface(surface),
            config,
            scale,
            screenshot,
            exit_after_screenshot,
        ))
    }

    /// Shared constructor tail for both window and offscreen renderers.
    fn build(
        device: wgpu::Device,
        queue: wgpu::Queue,
        target: Target,
        config: wgpu::SurfaceConfiguration,
        scale: f32,
        screenshot: Option<PathBuf>,
        exit_after_screenshot: bool,
    ) -> Self {
        let format = config.format;
        let size = (config.width, config.height);
        // Both caches start small and grow on demand (see `grow_atlas` /
        // `grow_pix`). Reserving a 4096² atlas and a 2048² RGBA pool up front
        // cost 32 MB of video memory before a single character was drawn, which
        // is most of what "325 MB with no document open" was made of.
        let atlas_size = 2048u32.min(device.limits().max_texture_dimension_2d);
        let atlas = Atlas::new(&device, &queue, atlas_size);
        let white = atlas.white_uv;
        // Images never need to be as large as the glyph atlas: 1024² of RGBA is
        // 4 MB and holds a few full-width screenshots at once.
        let pix = PixPool::new(&device, 1024);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quad"),
            source: wgpu::ShaderSource::Wgsl(include_str!("quad.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("quad-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
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
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quad-bg"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&atlas.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&pix.view) },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quad-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let attrs = [
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 16, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 32, shader_location: 2 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 48, shader_location: 3 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 64, shader_location: 4 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 80, shader_location: 5 },
        ];

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quad-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attrs,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let instance_cap = 4096;
        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: (instance_cap * size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Renderer {
            device,
            queue,
            target,
            config,
            pipeline,
            bind_group,
            globals,
            instances: Vec::with_capacity(instance_cap),
            instance_buf,
            instance_cap,
            atlas,
            pix,
            icons: HashMap::new(),
            images: HashMap::new(),
            glyphs: HashMap::new(),
            white,
            text: TextEngine::new(),
            size: (size.0.max(1) as f32, size.1.max(1) as f32),
            scale,
            clip: Rect::new(0.0, 0.0, size.0 as f32 / scale, size.1 as f32 / scale),
            clip_stack: Vec::new(),
            clear: Color::hex("#14161a"),
            stats: Stats::default(),
            screenshot,
            screenshot_frames: 1,
            capture: false,
            last_frame: None,
            frame: 0,
            exit_after_screenshot,
        }
    }

    pub fn logical_size(&self) -> (f32, f32) {
        (self.size.0 / self.scale, self.size.1 / self.scale)
    }

    pub fn resize(&mut self, w: u32, h: u32, scale: f32) {
        if w == 0 || h == 0 {
            return;
        }
        self.scale = scale;
        self.size = (w as f32, h as f32);
        self.config.width = w;
        self.config.height = h;
        match &self.target {
            Target::Surface(surface) => surface.configure(&self.device, &self.config),
            Target::Offscreen(_) => {
                let (target, _) = Self::offscreen_target(&self.device, self.config.format, w, h);
                self.target = target;
            }
        }
    }

    pub fn begin(&mut self, clear: Color) {
        self.clear = clear;
        self.instances.clear();
        self.clip = Rect::new(0.0, 0.0, self.size.0 / self.scale, self.size.1 / self.scale);
        self.clip_stack.clear();
        self.stats = Stats { cached_glyphs: self.glyphs.len(), ..Default::default() };
        self.text.cache_hits = 0;
        self.text.cache_misses = 0;
    }

    pub fn push_clip(&mut self, r: Rect) {
        self.clip_stack.push(self.clip);
        self.clip = self.clip.intersect(r);
    }

    pub fn pop_clip(&mut self) {
        if let Some(c) = self.clip_stack.pop() {
            self.clip = c;
        }
    }

    /// Push a textured quad (logical pixels).
    ///
    /// `solid` says the colour in this quad *is* the result - a plain fill, a border, a
    /// background - so the fragment shader can skip its atlas fetch. Text, icons and
    /// images leave it false and read their coverage from a texture.
    #[allow(clippy::too_many_arguments)]
    fn push_quad(
        &mut self,
        rect: Rect,
        uv: [f32; 4],
        color: Color,
        radius: f32,
        border: f32,
        border_color: Color,
        tex: f32,
        solid: bool,
    ) {
        // An outline carries its colour in `border_color` and leaves the fill
        // transparent, so "invisible fill" must not drop the quad.
        let outlined = border > 0.0 && border_color.a > 0.0;
        if (color.a <= 0.0 && !outlined && tex <= 0.0) || rect.is_empty() {
            return;
        }
        let c = rect.intersect(self.clip);
        if c.is_empty() {
            return;
        }
        let s = self.scale;
        let clip = self.clip;
        self.instances.push(Instance {
            rect: [rect.x * s, rect.y * s, rect.right() * s, rect.bottom() * s],
            uv,
            color: [color.r, color.g, color.b, color.a],
            clip: [clip.x * s, clip.y * s, clip.right() * s, clip.bottom() * s],
            params: [radius * s, border * s, tex, if solid { 1.0 } else { 0.0 }],
            border_color: [border_color.r, border_color.g, border_color.b, border_color.a],
        });
    }

    pub fn rect(&mut self, r: Rect, color: Color) {
        let uv = self.white;
        self.push_quad(r, uv, color, 0.0, 0.0, Color::TRANSPARENT, 0.0, true);
    }

    pub fn round_rect(&mut self, r: Rect, radius: f32, color: Color) {
        let uv = self.white;
        self.push_quad(r, uv, color, radius, 0.0, Color::TRANSPARENT, 0.0, true);
    }

    pub fn stroke_rect(&mut self, r: Rect, radius: f32, width: f32, color: Color) {
        let uv = self.white;
        self.push_quad(r, uv, Color::TRANSPARENT, radius, width, color, 0.0, true);
    }

    /// Filled rect with a border in **one** quad.
    ///
    /// Use this instead of a fill followed by a stroke: two overlapping quads over the
    /// same area cost two full passes of fragment work, which on a software rasteriser
    /// is the difference between a dialog that scrolls smoothly and one that does not.
    pub fn rect_bordered(&mut self, r: Rect, radius: f32, fill: Color, width: f32, border: Color) {
        let uv = self.white;
        self.push_quad(r, uv, fill, radius, width, border, 0.0, true);
    }

    /// Horizontal 1px-ish line helper (drawn as a thin rect).
    pub fn hline(&mut self, x: f32, y: f32, w: f32, thickness: f32, color: Color) {
        self.rect(Rect::new(x, y, w, thickness), color);
    }

    pub fn vline(&mut self, x: f32, y: f32, h: f32, thickness: f32, color: Color) {
        self.rect(Rect::new(x, y, thickness, h), color);
    }

    /// Rasterise (once) and cache an icon from `assets/icons/<name>.svg`.
    ///
    /// Icons live in the R8 atlas, exactly like glyphs, and are tinted at draw
    /// time — so the same file works on both themes.
    pub fn icon(&mut self, name: &str, px: u32) -> Option<IconTex> {
        let px = px.clamp(8, 128);
        let key = (name.to_string(), px);
        if let Some(hit) = self.icons.get(&key) {
            return *hit;
        }
        let tex = (|| {
            let src = crate::icons::source(name)?;
            let svg = crate::svg::parse(src)?;
            let (w, h, mut coverage) = crate::svg::rasterize(&svg, px);
            // Keep the cache small: nothing is ever drawn larger than a button.
            if w == 0 || h == 0 {
                return None;
            }
            let (x, y) = match self.atlas.alloc(w, h) {
                Some(pos) => pos,
                None => {
                    self.grow_atlas();
                    self.atlas.alloc(w, h)?
                }
            };
            self.atlas.upload(&self.queue, x, y, w, h, &coverage);
            coverage.clear();
            Some(IconTex { uv: self.atlas.uv(x, y, w, h), w, h })
        })();
        self.icons.insert(key, tex);
        tex
    }

    /// Push an icon quad (logical pixels), tinted with `color`.
    ///
    /// An icon is a bitmap with one texel per physical pixel (`Painter::icon`
    /// rasterises it at exactly the size it is drawn), so the quad is snapped to
    /// the physical pixel grid for the same reason a glyph is: half a pixel of
    /// offset turns every 1-pixel stroke into two grey ones.
    pub fn draw_icon(&mut self, rect: Rect, icon: IconTex, color: Color) {
        let s = self.scale;
        let r = Rect::new(
            (rect.x * s).round() / s,
            (rect.y * s).round() / s,
            ((rect.w * s).round() / s).max(1.0 / s),
            ((rect.h * s).round() / s).max(1.0 / s),
        );
        self.push_quad(r, icon.uv, color, 0.0, 0.0, Color::TRANSPARENT, 0.0, false);
        self.stats.glyph_quads += 1;
    }

    /// Upload an image (once) and cache it by path.
    pub fn image(&mut self, path: &std::path::Path, max_px: u32) -> Option<ImageTex> {
        let key = path.to_path_buf();
        if let Some(hit) = self.images.get(&key) {
            return *hit;
        }
        let tex = (|| {
            let img = crate::media::load(path)?;
            let (w, h) = (img.w, img.h);
            let (tw, th, rgba) = crate::media::fit(&img, max_px);
            let (x, y) = match self.pix.alloc(tw, th) {
                Some(pos) => pos,
                None => {
                    self.grow_pix();
                    self.pix.alloc(tw, th)?
                }
            };
            self.pix.upload(&self.queue, x, y, tw, th, &rgba);
            Some(ImageTex { uv: self.pix.uv(x, y, tw, th), w: tw, h: th, src: (w, h) })
        })();
        self.images.insert(key, tex);
        tex
    }

    /// Push an image quad: colour texture, optional rounded corners.
    pub fn draw_image(&mut self, rect: Rect, tex: ImageTex, radius: f32, tint: Color) {
        self.push_quad(rect, tex.uv, tint, radius, 0.0, Color::TRANSPARENT, 1.0, false);
    }

    /// Make sure every glyph of `text` is shaped, rasterised and in the atlas.
    ///
    /// This is what a frame does lazily, one glyph at a time, the first time a
    /// character is drawn: fontdue rasterisation plus a `write_texture` per glyph. For
    /// a page of text appearing at once - the About box's first frame - that is a
    /// visible hitch, so callers that know a page is coming warm it here, while the
    /// window is idle.
    pub fn warm_text(&mut self, face: Face, px: f32, text: &str) {
        let shaped = self.text.shape_dir(face, text, false);
        let px_phys = px * self.scale;
        for g in shaped.iter() {
            let _ = self.glyph(g.face, g.gid, px_phys);
        }
    }

    fn glyph(&mut self, face: Face, gid: u16, px_phys: f32) -> Option<GlyphTex> {
        let q = (px_phys * 4.0).round().max(1.0) as u32;
        let key = (face as u8, gid, q);
        if let Some(g) = self.glyphs.get(&key) {
            return Some(*g);
        }
        let px = q as f32 / 4.0;
        let (bitmap, w, h, left, top) = self.text.rasterize(face, gid, px)?;
        self.stats.rasterized += 1;
        let (x, y) = match self.atlas.alloc(w, h) {
            Some(p) => p,
            None => {
                // Atlas full: double it (up to the driver's limit) and start over.
                self.grow_atlas();
                self.atlas.alloc(w, h)?
            }
        };
        self.atlas.upload(&self.queue, x, y, w, h, &bitmap);
        let g = GlyphTex { uv: self.atlas.uv(x, y, w, h), w, h, left, top };
        self.glyphs.insert(key, g);
        self.stats.cached_glyphs = self.glyphs.len();
        Some(g)
    }

    /// The largest texture this device can give us (capped at 4096: beyond that
    /// the atlas is bigger than any screen and the extra memory buys nothing).
    fn max_texture(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d.min(4096)
    }

    /// Double the glyph atlas (or reset it when it is already at the limit) and
    /// throw away the caches whose uv coordinates point into the old texture.
    fn grow_atlas(&mut self) {
        let next = (self.atlas.size * 2).min(self.max_texture());
        log::debug!("atlas full: {} -> {}", self.atlas.size, next);
        self.atlas = Atlas::new(&self.device, &self.queue, next);
        self.glyphs.clear();
        self.icons.clear();
        self.stats.cached_glyphs = 0;
        self.rebuild_bind_group();
    }

    /// Same for the image pool. Every cached image has to be uploaded again, so
    /// the whole map goes; the next frame asks for them once more.
    fn grow_pix(&mut self) {
        let next = (self.pix.size * 2).min(self.max_texture());
        log::debug!("image pool full: {} -> {}", self.pix.size, next);
        self.pix = PixPool::new(&self.device, next);
        self.images.clear();
        self.rebuild_bind_group();
    }

    fn rebuild_bind_group(&mut self) {
        // The bind group layout is not retained, so re-create it from the pipeline.
        let layout = self.pipeline.get_bind_group_layout(0);
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        self.bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("quad-bg"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.atlas.view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.pix.view) },
            ],
        });
    }

    /// Draw a single line of text at `x` with the given baseline; returns the advance.
    pub fn text_run(&mut self, x: f32, baseline: f32, text: &str, face: Face, px: f32, color: Color) -> f32 {
        self.text_run_weighted(x, baseline, text, face, px, color, 0.0)
    }

    /// Like [`Renderer::text_run`], but each glyph is stamped `weight` logical
    /// pixels to the right as well, which thickens the stems.
    ///
    /// This is how emphasis is shown where the font has no heavier weight to
    /// offer: `**strong**` inside a heading (already bold) or inside a table's
    /// header row. The pen advance is the plain one - only the ink grows - so
    /// layout, caret and hit-testing are unaffected.
    #[allow(clippy::too_many_arguments)]
    pub fn text_run_weighted(
        &mut self,
        x: f32,
        baseline: f32,
        text: &str,
        face: Face,
        px: f32,
        color: Color,
        weight: f32,
    ) -> f32 {
        self.text_run_weighted_dir(x, baseline, text, face, px, color, weight, false)
    }

    /// Text with extra stem thickness, shaped and painted in its own direction.
    ///
    /// `rtl` only changes the *shaping*: rustybuzz hands back right-to-left glyphs
    /// already in visual order, so the pen here still runs left to right and an
    /// Arabic run comes out in reading order - with the joined forms it only gets
    /// when the shaper knows the text runs right to left.
    #[allow(clippy::too_many_arguments)]
    pub fn text_run_weighted_dir(
        &mut self,
        x: f32,
        baseline: f32,
        text: &str,
        face: Face,
        px: f32,
        color: Color,
        weight: f32,
        rtl: bool,
    ) -> f32 {
        if text.is_empty() || color.a <= 0.0 {
            return 0.0;
        }
        let shaped = self.text.shape_dir(face, text, rtl);
        let mut pen = x;
        let s = self.scale;
        for g in shaped.iter() {
            let adv = g.advance * px;
            if g.gid != 0 {
                if let Some(gt) = self.glyph(g.face, g.gid, px * s) {
                    // **Snap the bitmap to whole physical pixels.** The pen moves
                    // by fractional advances (that is what keeps the spacing
                    // right), but the bitmap itself is an integer grid of pixels:
                    // drawn at a fractional offset the sampler interpolates
                    // between texels and every stem gets a grey fringe on both
                    // sides - the "everything looks blurry" report. Rounding the
                    // *bitmap's* top-left (and nothing else) keeps the coverage
                    // exactly the pixels that were rasterised, so glyphs and icons
                    // land 1:1 on the framebuffer.
                    let gx = ((pen + g.x_offset * px) * s + gt.left as f32).round() / s;
                    let gy = ((baseline - g.y_offset * px) * s - gt.top as f32).round() / s;
                    let w = gt.w as f32 / s;
                    let h = gt.h as f32 / s;
                    self.push_quad(Rect::new(gx, gy, w, h), gt.uv, color, 0.0, 0.0, Color::TRANSPARENT, 0.0, false);
                    self.stats.glyph_quads += 1;
                    if weight > 0.0 {
                        // Two extra stamps, half and all of the way across: one
                        // alone leaves a hard edge, three read as a heavier weight
                        // at the sizes a heading is drawn at.
                        for k in 1..=2 {
                            let dx = weight * k as f32 * 0.5;
                            let gx2 = ((pen + g.x_offset * px + dx) * s + gt.left as f32).round() / s;
                            self.push_quad(
                                Rect::new(gx2, gy, w, h),
                                gt.uv,
                                color,
                                0.0,
                                0.0,
                                Color::TRANSPARENT,
                                0.0,
                                false,
                            );
                            self.stats.glyph_quads += 1;
                        }
                    }
                }
            }
            pen += adv;
        }
        pen - x
    }

    /// Immutable-ish access to the shaping engine for measuring inside draw code.
    pub fn text_ref(&mut self) -> &mut TextEngine {
        &mut self.text
    }

    pub fn text_width(&mut self, text: &str, face: Face, px: f32) -> f32 {
        self.text.width(face, text, px)
    }

    /// Ascent/descent in logical pixels.
    pub fn metrics(&self, face: Face, px: f32) -> (f32, f32) {
        self.text.vertical_metrics(face, px)
    }

    pub fn line_height(&self, face: Face, px: f32) -> f32 {
        self.text.line_height(face, px)
    }

    /// Finish the frame: upload instances, draw, present (or dump a screenshot).
    /// Returns true when a screenshot run asked the caller to exit.
    pub fn end(&mut self) -> bool {
        self.stats.instances = self.instances.len();
        // Acquire the frame: a swapchain texture, or our own offscreen texture.
        let (texture, presentable) = match &self.target {
            Target::Surface(surface) => match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                    (t.texture.clone(), Some(t))
                }
                wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                    surface.configure(&self.device, &self.config);
                    return false;
                }
                _ => return false,
            },
            Target::Offscreen(texture) => (texture.clone(), None),
        };
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        if self.instances.len() > self.instance_cap {
            self.instance_cap = (self.instances.len() * 2).next_power_of_two();
            self.instance_buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("instances"),
                size: (self.instance_cap * size_of::<Instance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !self.instances.is_empty() {
            self.queue.write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(&self.instances));
        }
        self.queue.write_buffer(
            &self.globals,
            0,
            bytemuck::cast_slice(&[self.size.0, self.size.1, self.scale, 0.0f32]),
        );

        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("frame"),
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: self.clear.r as f64,
                            g: self.clear.g as f64,
                            b: self.clear.b as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !self.instances.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, self.instance_buf.slice(..));
                pass.draw(0..6, 0..self.instances.len() as u32);
            }
        }

        let mut screenshot: Option<(wgpu::Buffer, u32, u32, PathBuf)> = None;
        let mut capture: Option<(wgpu::Buffer, u32, u32)> = None;
        if let Some(path) = self.screenshot.clone() {
            if self.frame + 1 >= self.screenshot_frames as u64 {
                screenshot = Some(self.record_readback(&mut enc, &texture, path));
            }
        } else if self.capture {
            let (w, h) = (self.config.width, self.config.height);
            let bpr = ((w * 4).div_ceil(256)) * 256;
            let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("capture"),
                size: (bpr as u64) * (h as u64),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buf,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(bpr),
                        rows_per_image: Some(h),
                    },
                },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            capture = Some((buf, w, h));
        }
        self.queue.submit([enc.finish()]);
        self.frame += 1;

        let mut exit = false;
        let dumping = screenshot.is_some() || capture.is_some();
        if let Some(frame) = presentable {
            if dumping {
                // Headless/software WSI can block in present; drop instead.
                drop(frame);
            } else {
                self.queue.present(frame);
            }
        }
        if let Some((buf, w, h)) = capture {
            let pix = self.frame_rgba(buf, w, h);
            self.last_frame = Some((w, h, pix));
            self.capture = false;
        }
        if let Some((buf, w, h, path)) = screenshot {
            self.write_png(buf, w, h, &path);
            log::info!("screenshot written to {}", path.display());
            if self.exit_after_screenshot {
                exit = true;
            } else {
                self.screenshot = None;
            }
        }
        exit
    }

    fn record_readback(
        &self,
        enc: &mut wgpu::CommandEncoder,
        tex: &wgpu::Texture,
        path: PathBuf,
    ) -> (wgpu::Buffer, u32, u32, PathBuf) {
        let w = self.config.width;
        let h = self.config.height;
        let bpr = ((w * 4).div_ceil(256)) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (bpr as u64) * (h as u64),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bpr),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        (buf, w, h, path)
    }

    /// The pixels of a readback buffer, as RGBA8, row-major, top row first.
    fn frame_rgba(&self, buf: wgpu::Buffer, w: u32, h: u32) -> Vec<u8> {
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let data = buf.slice(..).get_mapped_range().unwrap();
        let bpr = ((w * 4).div_ceil(256)) * 256;
        let bgra = matches!(
            self.config.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let mut pix = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let row = &data[(y * bpr) as usize..(y * bpr + w * 4) as usize];
            for x in 0..w {
                let i = (x * 4) as usize;
                if bgra {
                    pix.extend_from_slice(&[row[i + 2], row[i + 1], row[i], 255]);
                } else {
                    pix.extend_from_slice(&[row[i], row[i + 1], row[i + 2], 255]);
                }
            }
        }
        pix
    }

    fn write_png(&self, buf: wgpu::Buffer, w: u32, h: u32, path: &PathBuf) {
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let data = buf.slice(..).get_mapped_range().unwrap();
        let bpr = ((w * 4).div_ceil(256)) * 256;
        let bgra = matches!(
            self.config.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let mut pix = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            let row = &data[(y * bpr) as usize..(y * bpr + w * 4) as usize];
            for x in 0..w {
                let i = (x * 4) as usize;
                if bgra {
                    pix.extend_from_slice(&[row[i + 2], row[i + 1], row[i], 255]);
                } else {
                    pix.extend_from_slice(&[row[i], row[i + 1], row[i + 2], 255]);
                }
            }
        }
        if let Ok(file) = std::fs::File::create(path) {
            let mut e = png::Encoder::new(std::io::BufWriter::new(file), w, h);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            if let Ok(mut wr) = e.write_header() {
                let _ = wr.write_image_data(&pix);
            }
        }
    }
}

#[cfg(test)]
mod sharp_tests {
    use super::*;

    /// Glyph *and* icon quads are placed on the physical pixel grid.
    ///
    /// The pen moves by fractional advances - that is what keeps the spacing
    /// right - but the bitmap the sampler reads is an integer grid: half a pixel
    /// of offset resamples it and every stem grows a grey fringe. This is the
    /// check that says the text (and not just the layout) is crisp, at 1x and at
    /// scaled desktops. One renderer for all of it: a device per size would make
    /// the suite run out of memory.
    #[test]
    fn glyph_and_icon_quads_land_on_whole_physical_pixels() {
        let mut r = Renderer::offscreen(600, 200, 1.0);
        for scale in [1.0f32, 1.25, 1.5, 2.0] {
            r.resize((600.0 * scale) as u32, (200.0 * scale) as u32, scale);
            r.begin(Color::rgba(0.0, 0.0, 0.0, 1.0));
            r.text_run(10.37, 40.61, "Hamburgefonstiv", Face::Regular, 17.0, Color::WHITE);
            assert!(!r.instances.is_empty(), "no glyphs were queued at {scale}x");
            for i in &r.instances {
                for v in i.rect {
                    assert!(
                        (v - v.round()).abs() < 1e-3,
                        "at {scale}x a glyph quad edge is at {v}, not on a pixel: {:?}",
                        i.rect
                    );
                }
            }
            // …and the same for an icon, which is drawn a texel per pixel.
            if let Some(tex) = r.icon("bold", 20) {
                let before = r.instances.len();
                let rect = Rect::new(3.3, 7.7, tex.w as f32 / scale, tex.h as f32 / scale);
                r.draw_icon(rect, tex, Color::WHITE);
                assert_eq!(r.instances.len(), before + 1);
                for v in r.instances.last().unwrap().rect {
                    assert!(
                        (v - v.round()).abs() < 1e-3,
                        "at {scale}x an icon quad is not on pixels: {:?}",
                        r.instances.last().unwrap().rect
                    );
                }
            }
        }
    }
}
