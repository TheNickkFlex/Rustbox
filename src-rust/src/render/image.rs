use std::collections::HashMap;
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
    max_size: usize,
}

impl ImageControl {
    pub fn new(max_size: usize) -> Self {
        Self { cache: HashMap::new(), max_size }
    }

    pub fn load(&mut self, path: &str) -> Result<Arc<Image>, anyhow::Error> {
        if let Some(cached) = self.cache.get(path) {
            return Ok(cached.clone());
        }

        if self.cache.len() >= self.max_size {
            if let Some(key) = self.cache.keys().next().cloned() {
                self.cache.remove(&key);
            }
        }

        let image = Arc::new(Image::from_file(path)?);
        self.cache.insert(path.to_string(), image.clone());
        Ok(image)
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }

    pub fn remove(&mut self, path: &str) {
        self.cache.remove(path);
    }
}
