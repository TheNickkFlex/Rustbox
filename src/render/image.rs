use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;

use image::{DynamicImage, GenericImageView};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, Screen, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

pub struct Image {
    pub width: u32,
    pub height: u32,
    data: Vec<u8>,
}

impl Image {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, anyhow::Error> {
        let img = image::open(path)?;
        let (width, height) = img.dimensions();
        let rgba = img.to_rgba8();
        let data = rgba.into_raw();
        Ok(Self { width, height, data })
    }

    pub fn from_memory(data: &[u8]) -> Result<Self, anyhow::Error> {
        let img = image::load_from_memory(data)?;
        let (width, height) = img.dimensions();
        let rgba = img.to_rgba8();
        let data = rgba.into_raw();
        Ok(Self { width, height, data })
    }

    pub fn create_pixmap(&self, conn: &RustConnection, screen: &Screen, drawable: u32) -> Result<u32, anyhow::Error> {
        let depth: u8 = screen.root_depth;
        let bpp = if depth == 32 { 4 } else { 4 };
        let pixmap = conn.generate_id()?;
        conn.create_pixmap(depth, pixmap, drawable, self.width as u16, self.height as u16)?;

        let gc = conn.generate_id()?;
        conn.create_gc(gc, pixmap, &xproto::CreateGCAux::new())?;

        let stride = self.width as usize * bpp;

        // X11 caps a single PutImage at the server's max request length. The
        // on-wire length field is only 16 bits unless BIG-REQUESTS is enabled
        // and negotiated, so we hard-cap each strip to 65535 units (≈256 KB of
        // pixel data) regardless of what the server advertises. A full-screen
        // wallpaper would otherwise overflow the request and make the server
        // disconnect the client — which previously killed the WM on real
        // displays. Sending in horizontal strips works at any resolution.
        let max_req_units = conn.setup().maximum_request_length as usize;
        let max_req_units = max_req_units.min(65535);
        let max_data_units = max_req_units.saturating_sub(8).max(1);
        let max_data_bytes = max_data_units * 4;
        let mut strip_rows = max_data_bytes / stride.max(1);
        if strip_rows == 0 {
            strip_rows = 1;
        }

        let mut y = 0u16;
        while y < self.height as u16 {
            let h = std::cmp::min(strip_rows as u16, self.height as u16 - y);
            let mut strip = Vec::with_capacity(stride * h as usize);
            for row in 0..h {
                let base = ((y as usize + row as usize) * self.width as usize) * 4;
                let end = base + self.width as usize * 4;
                for chunk in self.data[base..end].chunks(4) {
                    let (r, g, b) = (chunk[0], chunk[1], chunk[2]);
                    strip.push(b);
                    strip.push(g);
                    strip.push(r);
                    if bpp == 4 {
                        strip.push(chunk[3]);
                    }
                }
            }
            conn.put_image(
                xproto::ImageFormat::Z_PIXMAP,
                pixmap,
                gc,
                self.width as u16,
                h,
                0,
                y as i16,
                0,
                depth,
                &strip,
            )?;
            y += h;
        }

        conn.free_gc(gc)?;
        Ok(pixmap)
    }

    /// Build an `Image` from a SNI `IconPixmap` buffer.
    ///
    /// The D-Bus `IconPixmap` format is ARGB32: 4 bytes per pixel in the order
    /// `A, R, G, B`. We reorder to the RGBA8 layout used everywhere else so the
    /// existing `create_pixmap`/`scale` paths work unchanged.
    pub fn from_argb32(width: u32, height: u32, argb: &[u8]) -> Result<Self, anyhow::Error> {
        let expected = width as usize * height as usize * 4;
        if argb.len() != expected {
            return Err(anyhow::anyhow!(
                "SNI: IconPixmap com tamanho inválido (esperado {expected} bytes, obteve {})",
                argb.len()
            ));
        }
        let mut data = Vec::with_capacity(expected);
        for chunk in argb.chunks(4) {
            let (a, r, g, b) = (chunk[0], chunk[1], chunk[2], chunk[3]);
            data.push(r);
            data.push(g);
            data.push(b);
            data.push(a);
        }
        Ok(Self { width, height, data })
    }

    pub fn scale(&self, width: u32, height: u32) -> Result<Self, anyhow::Error> {
        if self.width == width && self.height == height {
            return Ok(Self {
                width: self.width,
                height: self.height,
                data: self.data.clone(),
            });
        }
        let img = DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(self.width, self.height, self.data.clone())
                .ok_or_else(|| anyhow::anyhow!("Failed to create image"))?
        );
        let scaled = img.resize_exact(width, height, image::imageops::FilterType::Triangle);
        let rgba = scaled.to_rgba8();
        let (w, h) = rgba.dimensions();
        Ok(Self { width: w, height: h, data: rgba.into_raw() })
    }

    /// Composite the image's alpha channel against a solid background color,
    /// producing fully opaque RGB data. This is needed for SNI tray icons on
    /// X11 without a compositor: the server discards the alpha byte during
    /// `PutImage` (depth 24) and transparent pixels would otherwise show as
    /// black instead of blending into the toolbar background.
    pub fn composite_on_bg(mut self, bg_r: u8, bg_g: u8, bg_b: u8) -> Self {
        for pixel in self.data.chunks_mut(4) {
            let r = pixel[0];
            let g = pixel[1];
            let b = pixel[2];
            let a = pixel[3] as f32 / 255.0;
            pixel[0] = (bg_r as f32 + (r as f32 - bg_r as f32) * a).round().clamp(0.0, 255.0) as u8;
            pixel[1] = (bg_g as f32 + (g as f32 - bg_g as f32) * a).round().clamp(0.0, 255.0) as u8;
            pixel[2] = (bg_b as f32 + (b as f32 - bg_b as f32) * a).round().clamp(0.0, 255.0) as u8;
            pixel[3] = 255; // fully opaque
        }
        self
    }
}

pub struct ImageControl {
    cache: HashMap<String, Arc<Image>>,
    /// Insertion order used for FIFO/LRU eviction. The front is the oldest
    /// entry; when the cache is full we evict from the front rather than
    /// picking an arbitrary `HashMap` slot.
    order: VecDeque<String>,
    max_size: usize,
}

impl ImageControl {
    pub fn new(max_size: usize) -> Self {
        Self {
            cache: HashMap::new(),
            order: VecDeque::new(),
            max_size,
        }
    }

    pub fn load(&mut self, path: &str) -> Result<Arc<Image>, anyhow::Error> {
        if let Some(cached) = self.cache.get(path) {
            // Touch: move to the back so recently used entries survive eviction.
            if let Some(pos) = self.order.iter().position(|k| k == path) {
                let key = self.order.remove(pos).unwrap();
                self.order.push_back(key);
            }
            return Ok(cached.clone());
        }

        if self.cache.len() >= self.max_size {
            if let Some(oldest) = self.order.pop_front() {
                self.cache.remove(&oldest);
            }
        }

        let image = Arc::new(Image::from_file(path)?);
        self.cache.insert(path.to_string(), image.clone());
        self.order.push_back(path.to_string());
        Ok(image)
    }

    /// Load an image from raw in-memory bytes (e.g. the `image-data` hint).
    /// Not cached (callers pass few, unique payloads).
    pub fn load_memory(&mut self, data: &[u8]) -> Result<Arc<Image>, anyhow::Error> {
        Ok(Arc::new(Image::from_memory(data)?))
    }

    pub fn clear(&mut self) {
        self.cache.clear();
        self.order.clear();
    }

    pub fn remove(&mut self, path: &str) {
        self.cache.remove(path);
        if let Some(pos) = self.order.iter().position(|k| k == path) {
            self.order.remove(pos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_argb32_reorders_to_rgba() {
        // One pixel: ARGB = (0xAA, 0x12, 0x34, 0x56) -> RGBA = (0x12, 0x34, 0x56, 0xAA)
        let argb = [0xAA, 0x12, 0x34, 0x56];
        let img = Image::from_argb32(1, 1, &argb).unwrap();
        assert_eq!(img.data, vec![0x12, 0x34, 0x56, 0xAA]);
        assert_eq!(img.width, 1);
        assert_eq!(img.height, 1);
    }

    #[test]
    fn from_argb32_rejects_short_buffer() {
        let argb = [0u8; 7];
        assert!(Image::from_argb32(2, 1, &argb).is_err());
    }
}
