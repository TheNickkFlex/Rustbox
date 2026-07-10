use std::collections::HashMap;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, Font as X11Font, Gcontext, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

#[derive(Debug, Clone)]
pub struct Font {
    name: String,
    height: u16,
    ascent: u16,
    descent: u16,
    x_id: Option<X11Font>,
    scale: u16,
}

impl Font {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            height: 8 * 2,
            ascent: 7 * 2,
            descent: 1 * 2,
            x_id: None,
            scale: 2,
        }
    }

    pub fn load_x11_font(conn: &RustConnection, name: &str) -> Result<Self, anyhow::Error> {
        let font = conn.generate_id()?;
        conn.open_font(font, name.as_bytes())?;

        let reply = conn.query_font(font)?.reply()?;
        Ok(Self {
            name: name.to_string(),
            height: (reply.font_ascent + reply.font_descent) as u16,
            ascent: reply.font_ascent as u16,
            descent: reply.font_descent as u16,
            x_id: Some(font),
            scale: 2,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    pub fn ascent(&self) -> u16 {
        self.ascent
    }

    pub fn descent(&self) -> u16 {
        self.descent
    }

    pub fn x_id(&self) -> Option<X11Font> {
        self.x_id
    }

    pub fn text_width(&self, _conn: &RustConnection, text: &str) -> Result<u16, anyhow::Error> {
        // Always use the built-in bitmap font metrics for consistency with draw_text.
        Ok(crate::render::bitmap_font::text_width_scaled(text, self.scale))
    }

    pub fn draw_text(
        &self,
        conn: &RustConnection,
        drawable: u32,
        gc: Gcontext,
        x: i16,
        y: i16,
        text: &str,
    ) -> Result<(), anyhow::Error> {
        // Always use the built-in 8x8 bitmap font. X core fonts are unreliable
        // on some X servers (e.g. termux-x11) — open_font succeeds but
        // image_text8 draws nothing. The bitmap font renders via
        // poly_fill_rectangle which works everywhere.
        //
        // The y coordinate is the baseline for X core fonts; for the
        // bitmap font we draw from the top, so shift up by ascent.
        let top_y = y - self.ascent as i16;
        crate::render::bitmap_font::draw_bitmap_text(conn, drawable, gc, x, top_y, text, self.scale)?;
        Ok(())
    }

    pub fn free(&self, conn: &RustConnection) -> Result<(), anyhow::Error> {
        if let Some(font_id) = self.x_id {
            conn.close_font(font_id)?;
        }
        Ok(())
    }
}

pub struct FontManager {
    fonts: HashMap<String, Font>,
    fallback: Font,
}

impl FontManager {
    pub fn new() -> Self {
        Self {
            fonts: HashMap::new(),
            fallback: Font::new("fixed"),
        }
    }

    pub fn load_font(&mut self, conn: &RustConnection, name: &str) -> Result<&Font, anyhow::Error> {
        if !self.fonts.contains_key(name) {
            let font = Font::load_x11_font(conn, name)
                .unwrap_or_else(|_| Font::new(name));
            self.fonts.insert(name.to_string(), font);
        }
        Ok(self.fonts.get(name).unwrap_or(&self.fallback))
    }

    pub fn get_font(&self, name: &str) -> &Font {
        self.fonts.get(name).unwrap_or(&self.fallback)
    }
}

impl Default for FontManager {
    fn default() -> Self {
        Self::new()
    }
}
