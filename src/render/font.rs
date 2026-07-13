use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use ab_glyph::{Font as AbFont, FontRef, OutlinedGlyph, PxScale, ScaleFont};
use image::{load_from_memory, RgbaImage};
use ttf_parser::Face;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt as _, Gcontext};
use x11rb::rust_connection::RustConnection;

/// Unicode code points that should be rendered with the color-emoji font
/// instead of the regular outline font. Includes the zero-width joiner and
/// the emoji variation selector so sequences don't fall through to boxes.
pub fn is_emoji(c: char) -> bool {
    let u = c as u32;
    (0x1F300..=0x1FAFF).contains(&u)
        || (0x1F000..=0x1F2FF).contains(&u)
        || (0x2600..=0x27BF).contains(&u)
        || (0x2300..=0x23FF).contains(&u)
        || (0x2190..=0x21FF).contains(&u)
        || u == 0x200D
        || u == 0xFE0F
        || (0x2B00..=0x2BFF).contains(&u)
}

pub struct FontData {
    pub data: Vec<u8>,
    pub face_index: u32,
}

static TEXT_DB: OnceLock<fontdb::Database> = OnceLock::new();
static TEXT_FONT: OnceLock<Option<FontData>> = OnceLock::new();
static TEXT_FR: OnceLock<Option<FontRef<'static>>> = OnceLock::new();
static EMOJI_FONT: OnceLock<Option<FontData>> = OnceLock::new();

fn db() -> &'static fontdb::Database {
    TEXT_DB.get_or_init(|| {
        let mut d = fontdb::Database::new();
        d.load_system_fonts();
        d
    })
}

fn resolve(family: fontdb::Family<'_>) -> Option<FontData> {
    let d = db();
    let q = fontdb::Query {
        families: &[family],
        ..fontdb::Query::default()
    };
    let id = d.query(&q)?;
    d.with_face_data(id, |data, face_index| FontData {
        data: data.to_vec(),
        face_index,
    })
}

fn text_font_data() -> Option<&'static FontData> {
    TEXT_FONT
        .get_or_init(|| {
            resolve(fontdb::Family::SansSerif)
                .or_else(|| resolve(fontdb::Family::Name("DejaVu Sans")))
                .or_else(|| resolve(fontdb::Family::Name("Liberation Sans")))
        })
        .as_ref()
}

pub fn text_font_ref() -> Option<&'static FontRef<'static>> {
    TEXT_FR
        .get_or_init(|| text_font_data().and_then(|fd| FontRef::try_from_slice(&fd.data).ok()))
        .as_ref()
}

pub fn emoji_font_data() -> Option<&'static FontData> {
    EMOJI_FONT
        .get_or_init(|| {
            // Try fontdb first (scans system font paths).
            resolve(fontdb::Family::Name("Noto Color Emoji"))
                .or_else(|| resolve(fontdb::Family::Name("Noto Emoji")))
                .or_else(|| resolve(fontdb::Family::Name("EmojiOne Color")))
                .or_else(|| resolve(fontdb::Family::Name("OpenMoji")))
                // Fallback: common hardcoded paths for distributions that
                // install Noto Color Emoji outside fontdb's search scope.
                .or_else(|| load_emoji_from_paths())
        })
        .as_ref()
}

/// Try to load an emoji font from common hardcoded paths.
fn load_emoji_from_paths() -> Option<FontData> {
    for path in &[
        "/usr/share/fonts/noto/NotoColorEmoji.ttf",
        "/usr/share/fonts/noto/NotoColorEmoji-Regular.ttf",
        "/usr/share/fonts/emojione/EmojiOneColor.otf",
        "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
        "/usr/share/fonts/google/NotoColorEmoji.ttf",
        "/usr/share/fonts/google-noto-color-emoji-fonts/Noto-COLRv1.ttf",
        "/usr/share/fonts/opentype/noto/NotoColorEmoji.ttf",
        "/usr/share/noto/NotoColorEmoji.ttf",
    ] {
        if let Ok(data) = std::fs::read(path) {
            return Some(FontData {
                data,
                face_index: 0,
            });
        }
    }
    None
}

/// Fallback placeholder for emoji code points that cannot be rasterised via
/// the colour font (COLR/COLRv1 fonts whose base glyphs have no outline).
/// Returns a coloured circle so the emoji position is always visible.
pub fn make_emoji_placeholder(cp: u32, px: u32) -> (u32, u32, Vec<u8>) {
    let px = px.max(8);
    let r = (px as f32) / 2.0;
    let cx = r;
    let cy = r;
    // Deterministic colour from the code point (palette of 6 bright colours).
    let palette: &[(u8, u8, u8)] = &[
        (0xFF, 0xCC, 0x00), // yellow
        (0x44, 0xBB, 0x44), // green
        (0x33, 0x99, 0xFF), // blue
        (0xDD, 0x44, 0x44), // red
        (0xDD, 0x88, 0xFF), // purple
        (0xFF, 0x88, 0x44), // orange
    ];
    let idx = (cp as usize) % palette.len();
    let (cr, cg, cb) = palette[idx];
    let mut buf = vec![0u8; (px * px * 4) as usize];
    for y in 0..px {
        for x in 0..px {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let dist = (dx * dx + dy * dy).sqrt();
            let alpha = if dist <= r - 1.0 {
                255u8
            } else if dist <= r {
                ((r - dist) * 255.0) as u8
            } else {
                0u8
            };
            let i = (y * px + x) * 4;
            buf[i as usize] = cr;
            buf[i as usize + 1] = cg;
            buf[i as usize + 2] = cb;
            buf[i as usize + 3] = alpha;
        }
    }
    (px, px, buf)
}

/// Render a single emoji code point to an RGBA8 bitmap at roughly `px` pixels
/// tall. Tries, in order:
/// 1. COLRv1 vector color glyphs (real colored emojis),
/// 2. embedded PNG (CBDT/CBLC) when available,
/// 3. the monochrome outline fallback.
pub fn render_emoji(cp: u32, px: u32) -> Option<(u32, u32, Vec<u8>)> {
    if let Some(bmp) = crate::render::emoji::render_emoji_colr(cp, px) {
        return Some(bmp);
    }

    let fd = emoji_font_data()?;
    let face = Face::parse(&fd.data, fd.face_index).ok()?;
    let ch = char::from_u32(cp)?;
    let gid = face.glyph_index(ch)?;

    // Try embedded PNG first (CBDT/CBLC fonts).
    if let Some(img) = face.glyph_raster_image(gid, px as u16) {
        if let Ok(decoded) = load_from_memory(img.data) {
            let rgba = decoded.to_rgba8();
            let (w, h) = rgba.dimensions();
            return Some((w, h, rgba.into_raw()));
        }
    }

    // Fallback: render the outline glyph as a monochrome silhouette via
    // ab_glyph.  This handles COLR/COLRv1 color fonts that don't store
    // embedded PNGs.
    fallback_emoji_outline(&fd.data, fd.face_index, gid, ch, px)
}

/// Render a glyph outline to an RGBA8 bitmap using `ab_glyph` so it can be
/// composited like a colour emoji.  The glyph is filled with a warm yellow
/// (#FFCC00) so it looks vaguely like a generic emoji.
fn fallback_emoji_outline(data: &[u8], _index: u32, _gid: ttf_parser::GlyphId, ch: char, px: u32) -> Option<(u32, u32, Vec<u8>)> {
    let fr = FontRef::try_from_slice(data).ok()?;
    let scaled = fr.as_scaled(PxScale::from(px as f32));
    let sglyph = scaled.scaled_glyph(ch);
    let og = fr.outline_glyph(sglyph)?;
    let bounds = og.px_bounds();
    let bw = (bounds.max.x - bounds.min.x).max(1.0) as usize;
    let bh = (bounds.max.y - bounds.min.y).max(1.0) as usize;
    let mut cov = vec![0u8; bw * bh];
    og.draw(|gx, gy, coverage| {
        let idx = gy as usize * bw + gx as usize;
        if idx < cov.len() {
            cov[idx] = (coverage * 255.0) as u8;
        }
    });

    // Warm yellow fill (#FFCC00) with the glyph coverage as alpha.
    let mut rgba = vec![0u8; bw * bh * 4];
    for y in 0..bh {
        for x in 0..bw {
            let c = cov[y * bw + x] as f32 / 255.0;
            if c > 0.0 {
                let i = (y * bw + x) * 4;
                rgba[i] = 0xFF;
                rgba[i + 1] = 0xCC;
                rgba[i + 2] = 0x00;
                rgba[i + 3] = (c * 255.0) as u8;
            }
        }
    }
    Some((bw as u32, bh as u32, rgba))
}

#[derive(Debug, Clone)]
pub struct Font {
    name: String,
    height: u16,
    ascent: u16,
    descent: u16,
    x_id: Option<xproto::Font>,
    scale: u16,
}

impl Font {
    pub fn new(name: &str) -> Self {
        let mut s = Self {
            name: name.to_string(),
            height: 16,
            ascent: 14,
            descent: 4,
            x_id: None,
            scale: 2,
        };
        s.update_metrics();
        s
    }

    fn px_for(scale: u16) -> f32 {
        8.0 * scale as f32
    }

    fn px(&self) -> f32 {
        Self::px_for(self.scale)
    }

    fn update_metrics(&mut self) {
        if let Some(fr) = text_font_ref() {
            let scaled = fr.as_scaled(PxScale::from(self.px()));
            self.ascent = scaled.ascent().ceil() as u16;
            self.descent = (-scaled.descent()).ceil() as u16;
            self.height = self.ascent + self.descent;
        } else {
            let px = self.px();
            self.height = (px * 1.4) as u16;
            self.ascent = px as u16;
            self.descent = self.height.saturating_sub(self.ascent);
        }
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

    pub fn scale(&self) -> u16 {
        self.scale
    }

    pub fn x_id(&self) -> Option<xproto::Font> {
        self.x_id
    }

    pub fn set_scale(&mut self, scale: u16) {
        self.scale = scale.max(1);
        self.update_metrics();
    }

    /// Set the font size by approximate pixel height (cap height in px).
    pub fn set_pixel_size(&mut self, px: u32) {
        let scale = ((px as f32) / 8.0).round().max(1.0) as u16;
        self.set_scale(scale);
    }

    /// Approximate pixel width of `text` (no X11 round-trip needed).
    fn measure(&self, text: &str) -> f32 {
        if let Some(fr) = text_font_ref() {
            let px = self.px();
            let scaled = fr.as_scaled(PxScale::from(px));
            let mut w = 0.0f32;
            for ch in text.chars() {
                if is_emoji(ch) {
                    if ch == '\u{200D}' || ch == '\u{FE0F}' {
                        continue;
                    }
                    w += px;
                    continue;
                }
                w += scaled.h_advance(fr.glyph_id(ch));
            }
            w
        } else {
            crate::render::bitmap_font::text_width_scaled(text, self.scale) as f32
        }
    }

    pub fn text_width(
        &self,
        _conn: &RustConnection,
        text: &str,
    ) -> Result<u16, anyhow::Error> {
        Ok(self.measure(text).ceil() as u16)
    }

    /// Word-wrap `text` so no line exceeds `max_width` pixels (approximate,
    /// measured per fragment).
    pub fn wrap(&self, text: &str, max_width: u32) -> Vec<String> {
        let max_width = max_width as f32;
        let mut lines = Vec::new();
        let mut cur = String::new();
        for word in text.split(' ') {
            let candidate = if cur.is_empty() {
                word.to_string()
            } else {
                format!("{cur} {word}")
            };
            if cur.is_empty() || self.measure(&candidate) <= max_width {
                cur = candidate;
            } else {
                lines.push(std::mem::take(&mut cur));
                cur = word.to_string();
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        lines
    }

    /// Paints an opaque `bg` behind the glyphs (no read-back of the destination
    /// needed). Used by notifications, toolbar, menus and frame titles, which
    /// draw text over a solid background.
    pub fn draw_text_on_bg(
        &self,
        conn: &RustConnection,
        drawable: u32,
        gc: Gcontext,
        x: i16,
        y: i16,
        text: &str,
        fg: u32,
        bg: u32,
    ) -> Result<(), anyhow::Error> {
        conn.change_gc(gc, &xproto::ChangeGCAux::new().foreground(fg))?;

        let px = self.px();
        if let Some(fr) = text_font_ref() {
            return self.draw_truetype_bg(conn, drawable, gc, x, y, text, fg, bg, fr, px);
        }

        // Fallback: bitmap font over a filled background rectangle.
        let top_y = y - self.ascent as i16;
        conn.change_gc(gc, &xproto::ChangeGCAux::new().foreground(bg))?;
        conn.poly_fill_rectangle(
            drawable,
            gc,
            &[xproto::Rectangle {
                x,
                y: top_y,
                width: self.text_width(conn, text)?,
                height: self.height(),
            }],
        )?;
        conn.change_gc(gc, &xproto::ChangeGCAux::new().foreground(fg))?;
        crate::render::bitmap_font::draw_bitmap_text(conn, drawable, gc, x, top_y, text, self.scale)?;
        Ok(())
    }

    /// Collect the per-glyph draw operations and the bounding box metrics for
    /// `text` at baseline `(x, y)`.
    fn collect_ops(
        &self,
        text: &str,
        font_ref: &FontRef<'static>,
        px: f32,
        x: f32,
        y: f32,
    ) -> (Vec<GlyphOp>, f32, f32, f32) {
        let scaled = font_ref.as_scaled(PxScale::from(px));
        let ascent_px = scaled.ascent();
        let height_px = scaled.height();

        let mut ops: Vec<GlyphOp> = Vec::new();
        let mut caret = x;
        for ch in text.chars() {
            if is_emoji(ch) {
                if ch == '\u{200D}' || ch == '\u{FE0F}' {
                    continue;
                }
                let emoji_px = px.ceil() as u32;
                let (ew, eh, rgba) = render_emoji(ch as u32, emoji_px)
                    .unwrap_or_else(|| make_emoji_placeholder(ch as u32, emoji_px));
                let (sw, sh, rgba_scaled) = scale_rgba(ew, eh, rgba, emoji_px);
                ops.push(GlyphOp::Emoji {
                    x: caret,
                    y: y - px,
                    w: sw,
                    h: sh,
                    rgba: rgba_scaled,
                });
                caret += px;
                continue;
            }
            let gid = font_ref.glyph_id(ch);
            let adv = scaled.h_advance(gid);
            if let Some(og) = font_ref.outline_glyph(scaled.scaled_glyph(ch)) {
                ops.push(GlyphOp::Text {
                    og,
                    caret,
                    baseline: y,
                });
            }
            caret += adv;
        }

        let width_px = (caret - x).ceil().max(1.0);
        (ops, width_px, ascent_px, height_px)
    }

    fn draw_truetype_bg(
        &self,
        conn: &RustConnection,
        drawable: u32,
        gc: Gcontext,
        x: i16,
        y: i16,
        text: &str,
        fg: u32,
        bg: u32,
        font_ref: &FontRef<'static>,
        px: f32,
    ) -> Result<(), anyhow::Error> {
        let (ops, width_px, ascent_px, height_px) =
            self.collect_ops(text, font_ref, px, x as f32, y as f32);
        let bx = x;
        let by = (y as f32 - ascent_px).round() as i16;
        let bw = width_px as u16;
        let bh = (height_px.ceil().max(1.0)) as u16;

        let setup = conn.setup();
        let depth = setup.roots[0].root_depth;
        let stride = bw as usize * 4;
        let needed = stride * bh as usize;
        let (br, bg_, bb) = unpack_pixel(bg);
        // Grow-or-reuse pixel buffer to avoid per-frame allocation.  Capacity
        // is preserved across calls so the hot path rarely re-allocates.
        let mut buf = Vec::with_capacity(needed);
        // SAFETY: every byte is immediately written by the fill loops below.
        unsafe { buf.set_len(needed); }
        if depth == 32 {
            for chunk in buf.chunks_exact_mut(4) {
                chunk[0] = bb;
                chunk[1] = bg_;
                chunk[2] = br;
                chunk[3] = 0xFF;
            }
        } else {
            for chunk in buf.chunks_exact_mut(4) {
                chunk[0] = bb;
                chunk[1] = bg_;
                chunk[2] = br;
            }
        }

        let (fr, fg_, fb) = unpack_pixel(fg);
        composite_ops(&mut buf, &ops, bx, by, stride, fr, fg_, fb);

        conn.put_image(
            xproto::ImageFormat::Z_PIXMAP,
            drawable,
            gc,
            bw,
            bh,
            bx,
            by,
            0,
            depth,
            &buf,
        )?;
        Ok(())
    }

    pub fn free(&self, _conn: &RustConnection) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

/// Scale an RGBA8 emoji bitmap so its height equals `target_h` while
/// preserving aspect ratio. Falls back to original dimensions if scaling
/// fails (shouldn't happen in practice).
fn scale_rgba(w: u32, h: u32, rgba: Vec<u8>, target_h: u32) -> (u32, u32, Vec<u8>) {
    if h == target_h || h == 0 || w == 0 {
        return (w, h, rgba);
    }
    let ratio = target_h as f32 / h as f32;
    let tw = (w as f32 * ratio).round().max(1.0) as u32;
    let img = match image::RgbaImage::from_raw(w, h, rgba) {
        Some(img) => img,
        None => return (w, h, vec![]),
    };
    let scaled = image::imageops::resize(&img, tw, target_h, image::imageops::FilterType::Lanczos3);
    (tw, target_h, scaled.into_raw())
}

/// Clamp a rectangle to non-negative coordinates so GetImage never receives
/// a negative origin. Returns the adjusted `(x, y, width, height)`.
fn clamp_rect(x: i16, y: i16, w: u16, h: u16) -> (i16, i16, u16, u16) {
    let img_x = x.max(0);
    let img_y = y.max(0);
    let img_w = if x < 0 {
        (w as i16 + x).max(0) as u16
    } else {
        w
    };
    let img_h = if y < 0 {
        (h as i16 + y).max(0) as u16
    } else {
        h
    };
    (img_x, img_y, img_w, img_h)
}

/// Decode an X pixel value into RGB assuming a BGRX (little-endian TrueColor)
/// framebuffer, matching how `Image::create_pixmap` packs pixels.
fn unpack_pixel(px: u32) -> (u8, u8, u8) {
    let b = (px & 0xFF) as u8;
    let g = ((px >> 8) & 0xFF) as u8;
    let r = ((px >> 16) & 0xFF) as u8;
    (r, g, b)
}

/// Composite a grayscale coverage value (`cov` in 0..1) of color (r,g,b) onto
/// the BGRX destination buffer at (dx,dy).
fn blend(buf: &mut [u8], dx: i32, dy: i32, stride: usize, r: u8, g: u8, b: u8, cov: f32) {
    if dx < 0 || dy < 0 {
        return;
    }
    let di = dy as usize * stride + dx as usize * 4;
    if di + 3 >= buf.len() {
        return;
    }
    let a = cov.clamp(0.0, 1.0);
    let src = [b, g, r];
    for c in 0..3 {
        let dst = buf[di + c] as f32;
        let s = src[c] as f32;
        buf[di + c] = (s * a + dst * (1.0 - a)) as u8;
    }
}

/// Composite an RGBA8 emoji bitmap onto the BGRX destination buffer.
fn composite_rgba(
    buf: &mut [u8],
    dx0: i32,
    dy0: i32,
    stride: usize,
    rgba: &[u8],
    w: usize,
    h: usize,
) {
    for yy in 0..h {
        for xx in 0..w {
            let si = (yy * w + xx) * 4;
            let sa = rgba[si + 3] as f32 / 255.0;
            if sa <= 0.0 {
                continue;
            }
            let dx = dx0 + xx as i32;
            let dy = dy0 + yy as i32;
            if dx < 0 || dy < 0 {
                continue;
            }
            let di = dy as usize * stride + dx as usize * 4;
            if di + 3 >= buf.len() {
                continue;
            }
            let sr = rgba[si] as f32;
            let sg = rgba[si + 1] as f32;
            let sb = rgba[si + 2] as f32;
            let src = [sb, sg, sr];
            for c in 0..3 {
                let dst = buf[di + c] as f32;
                buf[di + c] = (src[c] * sa + dst * (1.0 - sa)) as u8;
            }
        }
    }
}

enum GlyphOp {
    Text {
        og: OutlinedGlyph,
        caret: f32,
        baseline: f32,
    },
    Emoji {
        x: f32,
        y: f32,
        w: u32,
        h: u32,
        rgba: Vec<u8>,
    },
}

/// Composite the collected glyph operations onto `buf` (BGRX, `stride` bytes
/// per row) at destination offset `(bx, by)`.
fn composite_ops(
    buf: &mut [u8],
    ops: &[GlyphOp],
    bx: i16,
    by: i16,
    stride: usize,
    r: u8,
    g: u8,
    b: u8,
) {
    let bx_f = bx as f32;
    let by_f = by as f32;
    for op in ops {
        match op {
            GlyphOp::Text {
                og,
                caret,
                baseline,
            } => {
                let bounds = og.px_bounds();
                let base_x = caret + bounds.min.x - bx_f;
                let base_y = baseline + bounds.min.y - by_f;
                og.draw(|gx, gy, cov| {
                    let dx = (base_x + gx as f32) as i32;
                    let dy = (base_y + gy as f32) as i32;
                    blend(buf, dx, dy, stride, r, g, b, cov);
                });
            }
            GlyphOp::Emoji { x, y, w, h, rgba } => {
                let dx0 = (x - bx_f) as i32;
                let dy0 = (y - by_f) as i32;
                composite_rgba(buf, dx0, dy0, stride, rgba, *w as usize, *h as usize);
            }
        }
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

    pub fn load_font(&mut self, _conn: &RustConnection, name: &str) -> Result<&Font, anyhow::Error> {
        if !self.fonts.contains_key(name) {
            self.fonts.insert(name.to_string(), Font::new(name));
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
