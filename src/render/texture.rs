use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt as _};

use crate::core::{Color, Rectangle, TextureType};
use crate::x11::X11Connection;

#[derive(Debug, Clone)]
pub struct Texture {
    pub type_: TextureType,
    pub color: Color,
    pub color_to: Color,
    pub hi_color: Color,
    pub lo_color: Color,
    pub bevel_width: u16,
    pub interlaced: bool,
    pub border: bool,
    pub gradient: GradientType,
    pub pixmap: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientType {
    None,
    Diagonal,
    Horizontal,
    Vertical,
    CrossDiagonal,
    Pyramid,
    Rectangle,
    PipeCross,
    Elliptic,
    MirrorHorizontal,
    MirrorVertical,
    Solid,
}

impl Texture {
    pub fn new() -> Self {
        Self {
            type_: TextureType::Flat,
            color: Color::default(),
            color_to: Color::default(),
            hi_color: Color::default(),
            lo_color: Color::default(),
            bevel_width: 0,
            interlaced: false,
            border: false,
            gradient: GradientType::None,
            pixmap: None,
        }
    }

    pub fn is_gradient(&self) -> bool {
        self.gradient != GradientType::None
    }

    pub fn is_solid(&self) -> bool {
        self.type_ == TextureType::Solid || self.type_ == TextureType::Flat
    }

    pub fn is_pixmap(&self) -> bool {
        self.type_ == TextureType::Pixmap && self.pixmap.is_some()
    }
}

impl Default for Texture {
    fn default() -> Self {
        Self::new()
    }
}

pub struct TextureRender;

impl TextureRender {
    pub fn render_gradient(
        conn: &X11Connection,
        texture: &Texture,
        rect: &Rectangle,
        depth: u8,
    ) -> Result<u32, anyhow::Error> {
        let (r1, g1, b1) = (texture.color.red, texture.color.green, texture.color.blue);
        let (r2, g2, b2) = (texture.color_to.red, texture.color_to.green, texture.color_to.blue);

        let w = rect.width as usize;
        let h = rect.height as usize;

        // ZPixmap uses 4 bytes per pixel for depth >= 24 (the 4th byte is
        // ignored on 24-bit visuals), so a single contiguous BGRA buffer with
        // 4-byte scanline padding is correct for both depth 24 and 32. We
        // generate the raw image in a single pass to avoid an intermediate
        // width*height u32 allocation (which would be ~8 MB for a 1080p frame).
        let bytes_per_pixel = 4usize;
        let row_bytes = w * bytes_per_pixel;
        let pad = (row_bytes + 3) & !3;
        let mut data = vec![0u8; pad * h];

        for y in 0..h {
            let base = y * pad;
            for x in 0..w {
                let ratio = Self::gradient_ratio(x, y, w, h, &texture.gradient);
                let r = Self::lerp(r1, r2, ratio) as u8;
                let g = Self::lerp(g1, g2, ratio) as u8;
                let b = Self::lerp(b1, b2, ratio) as u8;
                let off = base + x * bytes_per_pixel;
                data[off] = b;
                data[off + 1] = g;
                data[off + 2] = r;
                data[off + 3] = 0xff;
            }
        }

        let pixmap_id = conn.conn().generate_id()?;
        conn.conn().create_pixmap(
            depth,
            pixmap_id,
            conn.root_window(),
            rect.width,
            rect.height,
        )?;

        let gc = conn.conn().generate_id()?;
        conn.conn().create_gc(gc, pixmap_id, &xproto::CreateGCAux::new())?;

        conn.conn().put_image(
            xproto::ImageFormat::Z_PIXMAP,
            pixmap_id,
            gc,
            rect.width,
            rect.height,
            0,
            0,
            0,
            depth,
            &data,
        )?;

        conn.conn().free_gc(gc)?;
        Ok(pixmap_id)
    }

    fn lerp(a: u16, b: u16, t: f64) -> u16 {
        (a as f64 + (b as f64 - a as f64) * t) as u16
    }

    fn gradient_ratio(x: usize, y: usize, width: usize, height: usize, gradient: &GradientType) -> f64 {
        match gradient {
            GradientType::Horizontal => x as f64 / width.saturating_sub(1).max(1) as f64,
            GradientType::Vertical => y as f64 / height.saturating_sub(1).max(1) as f64,
            GradientType::Diagonal => {
                let d = (x + y) as f64;
                let max_d = (width + height).saturating_sub(2).max(1) as f64;
                d / max_d
            }
            GradientType::CrossDiagonal => {
                let d = (x as i32 - y as i32).unsigned_abs() as f64;
                let max_d = width.max(height).saturating_sub(1).max(1) as f64;
                d / max_d
            }
            GradientType::Pyramid => {
                let cx = width as f64 / 2.0;
                let cy = height as f64 / 2.0;
                let dx = (x as f64 - cx).abs() / cx;
                let dy = (y as f64 - cy).abs() / cy;
                dx.max(dy).min(1.0)
            }
            GradientType::Rectangle => {
                let cx = width as f64 / 2.0;
                let cy = height as f64 / 2.0;
                let dx = (x as f64 - cx).abs() / cx;
                let dy = (y as f64 - cy).abs() / cy;
                (dx * dx + dy * dy).sqrt().min(1.0)
            }
            GradientType::PipeCross => {
                let dx = x as f64 / width.saturating_sub(1).max(1) as f64;
                let dy = y as f64 / height.saturating_sub(1).max(1) as f64;
                ((dx - 0.5).abs().max((dy - 0.5).abs()) * 2.0).min(1.0)
            }
            GradientType::Elliptic => {
                let cx = width as f64 / 2.0;
                let cy = height as f64 / 2.0;
                let dx = (x as f64 - cx) / cx;
                let dy = (y as f64 - cy) / cy;
                (dx * dx + dy * dy).sqrt().min(1.0)
            }
            GradientType::MirrorHorizontal => {
                let half = width as f64 / 2.0;
                let mut px = x as f64;
                if px >= half {
                    px = (width as f64 - 1.0) - px;
                }
                px / half.max(1.0)
            }
            GradientType::MirrorVertical => {
                let half = height as f64 / 2.0;
                let mut py = y as f64;
                if py >= half {
                    py = (height as f64 - 1.0) - py;
                }
                py / half.max(1.0)
            }
            _ => 0.0,
        }
    }

    pub fn render_bevel(
        conn: &X11Connection,
        drawable: u32,
        gc: u32,
        rect: &Rectangle,
        bevel_width: u16,
        raised: bool,
        hi_pixel: u32,
        lo_pixel: u32,
    ) -> Result<(), anyhow::Error> {
        let bw = bevel_width as i16;
        let right = rect.right() - 1;
        let bottom = rect.bottom() - 1;

        // Collect all highlight and shadow segments and emit them in exactly
        // two `poly_segment` requests (one per colour), regardless of bevel
        // width. This is a large reduction over per-edge poly_line calls that
        // each round-trip individually.
        let mut hi_segs: Vec<xproto::Segment> = Vec::with_capacity(bw as usize * 2);
        let mut lo_segs: Vec<xproto::Segment> = Vec::with_capacity(bw as usize * 2);
        for i in 0..bw {
            hi_segs.push(xproto::Segment { x1: rect.x + i, y1: rect.y + i, x2: right - i, y2: rect.y + i });
            hi_segs.push(xproto::Segment { x1: rect.x + i, y1: rect.y + i, x2: rect.x + i, y2: bottom - i });
            lo_segs.push(xproto::Segment { x1: right - i, y1: rect.y + i, x2: right - i, y2: bottom - i });
            lo_segs.push(xproto::Segment { x1: rect.x + i, y1: bottom - i, x2: right - i, y2: bottom - i });
        }

        let (hi, lo) = if raised { (hi_pixel, lo_pixel) } else { (lo_pixel, hi_pixel) };
        conn.conn().change_gc(gc, &xproto::ChangeGCAux::new().foreground(hi))?;
        conn.conn().poly_segment(drawable, gc, &hi_segs)?;
        conn.conn().change_gc(gc, &xproto::ChangeGCAux::new().foreground(lo))?;
        conn.conn().poly_segment(drawable, gc, &lo_segs)?;
        Ok(())
    }
}
