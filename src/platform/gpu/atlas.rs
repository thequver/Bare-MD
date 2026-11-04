//! A tiny shelf-packing glyph atlas (R8 coverage) with a reserved white texel.

pub struct Atlas {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: u32,
    shelf_y: u32,
    shelf_h: u32,
    pen_x: u32,
    /// UV rect of the fully-covered white texel.
    pub white_uv: [f32; 4],
}

impl Atlas {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, size: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph-atlas"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // White texel at (0,0) so solid quads sample 1.0.
        let atlas = Atlas {
            texture,
            view,
            size,
            shelf_y: 0,
            shelf_h: 0,
            pen_x: 2,
            white_uv: [0.5 / size as f32, 0.5 / size as f32, 0.5 / size as f32, 0.5 / size as f32],
        };
        atlas.upload(queue, 0, 0, 1, 1, &[255u8]);
        atlas
    }

    pub fn upload(&self, queue: &wgpu::Queue, x: u32, y: u32, w: u32, h: u32, data: &[u8]) {
        if w == 0 || h == 0 {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    /// Allocate a slot; returns (x, y) in texels or None when the atlas is full.
    pub fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let pad = 1;
        if self.pen_x + w + pad > self.size {
            self.shelf_y += self.shelf_h + pad;
            self.shelf_h = 0;
            self.pen_x = pad;
        }
        if self.shelf_y + h + pad > self.size {
            return None;
        }
        let x = self.pen_x;
        let y = self.shelf_y;
        self.pen_x += w + pad;
        self.shelf_h = self.shelf_h.max(h);
        Some((x, y))
    }

    pub fn uv(&self, x: u32, y: u32, w: u32, h: u32) -> [f32; 4] {
        let s = self.size as f32;
        [x as f32 / s, y as f32 / s, (x + w) as f32 / s, (y + h) as f32 / s]
    }
}


/// A shelf-packed RGBA pool for bitmapped content that needs colour (photos,
/// screenshots pasted into a document). Same allocation strategy as the glyph
/// atlas, four bytes per texel.
pub struct PixPool {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: u32,
    shelf_y: u32,
    shelf_h: u32,
    pen_x: u32,
    /// Set once the pool has been asked for more room than it has.
    pub full: bool,
}

impl PixPool {
    pub fn new(device: &wgpu::Device, size: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("pixel-pool"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        PixPool { texture, view, size, shelf_y: 0, shelf_h: 0, pen_x: 0, full: false }
    }

    pub fn upload(&self, queue: &wgpu::Queue, x: u32, y: u32, w: u32, h: u32, data: &[u8]) {
        if w == 0 || h == 0 {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    pub fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w > self.size || h > self.size {
            self.full = true;
            return None;
        }
        let pad = 1;
        if self.pen_x + w + pad > self.size {
            self.shelf_y += self.shelf_h + pad;
            self.shelf_h = 0;
            self.pen_x = pad;
        }
        if self.shelf_y + h + pad > self.size {
            self.full = true;
            return None;
        }
        let x = self.pen_x;
        let y = self.shelf_y;
        self.pen_x += w + pad;
        self.shelf_h = self.shelf_h.max(h);
        Some((x, y))
    }

    pub fn uv(&self, x: u32, y: u32, w: u32, h: u32) -> [f32; 4] {
        let s = self.size as f32;
        [x as f32 / s, y as f32 / s, (x + w) as f32 / s, (y + h) as f32 / s]
    }
}
