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
        let pixmap = conn.generate_id()?;
        conn.create_pixmap(screen.root_depth, pixmap, drawable, self.width as u16, self.height as u16)?;

        let gc = conn.generate_id()?;
        conn.create_gc(gc, pixmap, &xproto::CreateGCAux::new())?;

        let mut x_data = Vec::with_capacity(self.data.len());
        for chunk in self.data.chunks(4) {
            let (r, g, b) = (chunk[0], chunk[1], chunk[2]);
            x_data.push(b);
            x_data.push(g);
            x_data.push(r);
            if screen.root_depth == 32 {
                x_data.push(chunk[3]);
            }
        }

        conn.put_image(
            xproto::ImageFormat::Z_PIXMAP,
            pixmap,
            gc,
            self.width as u16,
            self.height as u16,
            0, 0, 0,
            screen.root_depth,
            &x_data,
        )?;

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
        let img = DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(self.width, self.height, self.data.clone())
                .ok_or_else(|| anyhow::anyhow!("Failed to create image"))?
        );
        let scaled = img.resize_exact(width, height, image::imageops::FilterType::Lanczos3);
        let rgba = scaled.to_rgba8();
        let (w, h) = rgba.dimensions();
        Ok(Self { width: w, height: h, data: rgba.into_raw() })
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
