//! COLRv1 color-emoji rasterization.
//!
//! The system emoji font (`Noto-COLRv1.ttf`) stores color glyphs as vector
//! layers (COLRv1), not embedded bitmaps.
//! `glyph_raster_image` returns `None`. This module walks the COLRv1 paint
//! graph with [`skrifa`] and rasterizes it to an RGBA bitmap via
//! [`tiny_skia`], producing real colored emojis instead of the yellow
//! outline/placeholder fallback.

use read_fonts::tables::colr::{CompositeMode, Extend};
use read_fonts::types::{BoundingBox, Point};
use skrifa::color::{Brush, ColorPainter, ColorStop, Transform as ColrTransform};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::prelude::{LocationRef, MetadataProvider, Size};
use skrifa::{FontRef, GlyphId};

use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, GradientStop, LinearGradient, Mask, Paint, Path,
    PathBuilder, Pixmap, PixmapPaint, Point as TsPoint, RadialGradient, Rect, Shader, SpreadMode,
    Transform,
};

/// Re-exported from `skrifa::color::Color` (an alias of `cpal::ColorRecord`):
/// straight `u8` RGBA palette entries.
use skrifa::color::Color as ColorRecord;

/// Render a single emoji code point to an RGBA8 bitmap at `px` pixels tall
/// using the COLRv1 color font. Returns `None` when no color glyph is
/// available, so callers can fall back to the bitmap/PNG or placeholder path.
pub fn render_emoji_colr(cp: u32, px: u32) -> Option<(u32, u32, Vec<u8>)> {
    let fd = crate::render::font::emoji_font_data()?;
    let font = FontRef::new(&fd.data).ok()?;
    let ch = char::from_u32(cp)?;
    let gid = font.charmap().map(ch)?;
    let cg = font.color_glyphs().get(gid)?;
    let upem = font
        .metrics(Size::new(1.0), LocationRef::default())
        .units_per_em as f32;
    let palette = font
        .color_palettes()
        .get(0)
        .map(|p| p.colors().iter().copied().collect())
        .unwrap_or_default();

    let size = px.max(1);
    let mut painter = ColrPainter::new(&font, palette, size, upem);
    cg.paint(LocationRef::default(), &mut painter).ok()?;
    let buf = painter.into_rgba();
    Some((size, size, buf))
}

/// A [`ColorPainter`] that rasterizes a COLRv1 glyph into a tiny-skia pixmap.
///
/// All drawing happens in font units; the base transform maps font units to
/// pixels (scaled by `px / upem` and with the Y axis flipped, since font
/// space is Y-up while the pixmap is Y-down).
struct ColrPainter<'a> {
    face: &'a FontRef<'static>,
    palette: Vec<ColorRecord>,
    upem: f32,
    w: u32,
    h: u32,
    pixmap: Pixmap,
    transform: Transform,
    transform_stack: Vec<Transform>,
    clip: Option<Mask>,
    clip_stack: Vec<Option<Mask>>,
    layer_stack: Vec<(Pixmap, CompositeMode)>,
}

impl<'a> ColrPainter<'a> {
    fn new(face: &'a FontRef<'static>, palette: Vec<ColorRecord>, px: u32, upem: f32) -> Self {
        let s = px as f32 / upem;
        // Map font units (x, y) -> pixels (x', y') with Y flipped:
        //   x' =  s * x
        //   y' = -s * y + upem * s
        let transform = Transform::from_row(s, 0.0, 0.0, -s, 0.0, upem * s);
        let pixmap = Pixmap::new(px, px).expect("pixmap allocation");
        Self {
            face,
            palette,
            upem,
            w: px,
            h: px,
            pixmap,
            transform,
            transform_stack: Vec::new(),
            clip: None,
            clip_stack: Vec::new(),
            layer_stack: Vec::new(),
        }
    }

    /// Build a tiny-skia path for the given glyph outline, in font units.
    fn glyph_path(&self, gid: GlyphId) -> Option<Path> {
        let mut builder = Pen(PathBuilder::new());
        if let Some(g) = self.face.outline_glyphs().get(gid) {
            let _ = g.draw(
                DrawSettings::unhinted(Size::new(self.upem), LocationRef::default()),
                &mut builder,
            );
        }
        builder.0.finish()
    }

    /// Push a clip region: intersect the optional glyph/path coverage with the
    /// current clip (if any). `None` clips everything away.
    fn push_clip(&mut self, path: Option<Path>) {
        let base = self.clip.take();
        let new = match path {
            None => Mask::new(self.w, self.h).expect("mask allocation"),
            Some(p) => match &base {
                None => {
                    let mut m = Mask::new(self.w, self.h).expect("mask allocation");
                    m.fill_path(&p, FillRule::Winding, true, self.transform);
                    m
                }
                Some(b) => {
                    let mut m = b.clone();
                    m.intersect_path(&p, FillRule::Winding, true, self.transform);
                    m
                }
            },
        };
        self.clip_stack.push(base);
        self.clip = Some(new);
    }

    fn make_paint(&self, brush: &Brush<'_>) -> Option<Paint<'static>> {
        let mut paint = Paint::default();
        paint.anti_alias = true;
        match brush {
            Brush::Solid {
                palette_index,
                alpha,
            } => {
                let c = self.palette.get(*palette_index as usize)?;
                let a = (c.alpha as f32 / 255.0) * alpha;
                paint.shader = Shader::SolidColor(color_u8(c.red, c.green, c.blue, (a * 255.0) as u8));
            }
            Brush::LinearGradient {
                p0,
                p1,
                color_stops,
                extend,
            } => {
                let stops = self.gradient_stops(color_stops);
                let mode = spread(*extend);
                let sh =
                    LinearGradient::new(point(p0), point(p1), stops, mode, Transform::identity())?;
                paint.shader = sh;
            }
            Brush::RadialGradient {
                c0,
                r0: _r0,
                c1,
                r1,
                color_stops,
                extend,
            } => {
                let stops = self.gradient_stops(color_stops);
                let mode = spread(*extend);
                // COLRv1 radial: inner circle (c0, r0) -> outer circle (c1, r1).
                // tiny-skia two-point conical: start = outer center, end = focal,
                // radius = outer radius. We approximate by ignoring the inner
                // radius (r0) since COLRv1 inner-radius gradients are rare.
                let sh = RadialGradient::new(
                    point(c0),
                    point(c1),
                    *r1,
                    stops,
                    mode,
                    Transform::identity(),
                )?;
                paint.shader = sh;
            }
            Brush::SweepGradient {
                c0: _c0,
                color_stops,
                extend: _extend,
                ..
            } => {
                // tiny-skia has no native sweep/conic gradient; approximate with
                // the color at the mid offset so the shape still shows color.
                let approx = mid_color(color_stops, &self.palette);
                paint.shader = Shader::SolidColor(approx);
            }
        }
        Some(paint)
    }

    fn gradient_stops(&self, stops: &[ColorStop]) -> Vec<GradientStop> {
        stops
            .iter()
            .map(|s| {
                let c = self
                    .palette
                    .get(s.palette_index as usize)
                    .copied()
                    .unwrap_or(ColorRecord {
                        red: 0,
                        green: 0,
                        blue: 0,
                        alpha: 255,
                    });
                let a = (c.alpha as f32 / 255.0) * s.alpha;
                GradientStop::new(s.offset, color_u8(c.red, c.green, c.blue, (a * 255.0) as u8))
            })
            .collect()
    }

    /// Convert the premultiplied pixmap into a straight-alpha RGBA buffer.
    fn into_rgba(self) -> Vec<u8> {
        let data = self.pixmap.take();
        let mut out = Vec::with_capacity(data.len());
        for chunk in data.chunks_exact(4) {
            let (r, g, b, a) = (chunk[0], chunk[1], chunk[2], chunk[3]);
            if a == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let rf = (r as f32 / a as f32) * 255.0;
                let gf = (g as f32 / a as f32) * 255.0;
                let bf = (b as f32 / a as f32) * 255.0;
                out.push(rf.min(255.0) as u8);
                out.push(gf.min(255.0) as u8);
                out.push(bf.min(255.0) as u8);
                out.push(a);
            }
        }
        out
    }
}

fn color_u8(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

fn point(p: &Point<f32>) -> TsPoint {
    TsPoint::from_xy(p.x, p.y)
}

fn spread(e: Extend) -> SpreadMode {
    match e {
        Extend::Pad => SpreadMode::Pad,
        Extend::Repeat => SpreadMode::Repeat,
        Extend::Reflect => SpreadMode::Reflect,
        _ => SpreadMode::Pad,
    }
}

fn mid_color(stops: &[ColorStop], palette: &[ColorRecord]) -> Color {
    let stop = stops
        .iter()
        .min_by(|a, b| {
            (a.offset - 0.5)
                .abs()
                .partial_cmp(&(b.offset - 0.5).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .or_else(|| stops.first());
    match stop {
        Some(s) => {
            let c = palette
                .get(s.palette_index as usize)
                .copied()
                .unwrap_or(ColorRecord {
                    red: 0,
                    green: 0,
                    blue: 0,
                    alpha: 255,
                });
            let a = (c.alpha as f32 / 255.0) * s.alpha;
            color_u8(c.red, c.green, c.blue, (a * 255.0) as u8)
        }
        None => Color::from_rgba8(0, 0, 0, 0),
    }
}

fn map_blend(mode: CompositeMode) -> BlendMode {
    match mode {
        CompositeMode::Clear => BlendMode::Clear,
        CompositeMode::Src => BlendMode::Source,
        CompositeMode::Dest => BlendMode::Destination,
        CompositeMode::SrcOver => BlendMode::SourceOver,
        CompositeMode::DestOver => BlendMode::DestinationOver,
        CompositeMode::SrcIn => BlendMode::SourceIn,
        CompositeMode::DestIn => BlendMode::DestinationIn,
        CompositeMode::SrcOut => BlendMode::SourceOut,
        CompositeMode::DestOut => BlendMode::DestinationOut,
        CompositeMode::SrcAtop => BlendMode::SourceAtop,
        CompositeMode::DestAtop => BlendMode::DestinationAtop,
        CompositeMode::Xor => BlendMode::Xor,
        CompositeMode::Plus => BlendMode::Plus,
        CompositeMode::Screen => BlendMode::Screen,
        CompositeMode::Overlay => BlendMode::Overlay,
        CompositeMode::Darken => BlendMode::Darken,
        CompositeMode::Lighten => BlendMode::Lighten,
        CompositeMode::ColorDodge => BlendMode::ColorDodge,
        CompositeMode::ColorBurn => BlendMode::ColorBurn,
        CompositeMode::HardLight => BlendMode::HardLight,
        CompositeMode::SoftLight => BlendMode::SoftLight,
        CompositeMode::Difference => BlendMode::Difference,
        CompositeMode::Exclusion => BlendMode::Exclusion,
        CompositeMode::Multiply => BlendMode::Multiply,
        CompositeMode::HslHue => BlendMode::Hue,
        CompositeMode::HslSaturation => BlendMode::Saturation,
        CompositeMode::HslColor => BlendMode::Color,
        CompositeMode::HslLuminosity => BlendMode::Luminosity,
        CompositeMode::Unknown => BlendMode::SourceOver,
    }
}

/// Newtype wrapper so we can implement the external `OutlinePen` trait for the
/// external `PathBuilder` (orphan rule).
struct Pen(PathBuilder);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(cx0, cy0, x, y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.cubic_to(cx0, cy0, cx1, cy1, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

impl ColorPainter for ColrPainter<'_> {
    fn push_transform(&mut self, transform: ColrTransform) {
        // skrifa transform: (x, y) -> (xx*x + yx*y + dx, xy*x + yy*y + dy)
        let ts = Transform::from_row(
            transform.xx,
            transform.xy,
            transform.yx,
            transform.yy,
            transform.dx,
            transform.dy,
        );
        self.transform_stack.push(self.transform);
        self.transform = self.transform.pre_concat(ts);
    }

    fn pop_transform(&mut self) {
        if let Some(t) = self.transform_stack.pop() {
            self.transform = t;
        }
    }

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        self.push_clip(self.glyph_path(glyph_id));
    }

    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        let w = (clip_box.x_max - clip_box.x_min).max(0.0);
        let h = (clip_box.y_max - clip_box.y_min).max(0.0);
        if w <= 0.0 || h <= 0.0 {
            self.push_clip(None);
            return;
        }
        let rect = match Rect::from_xywh(clip_box.x_min, clip_box.y_min, w, h) {
            Some(r) => r,
            None => {
                self.push_clip(None);
                return;
            }
        };
        self.push_clip(Some(PathBuilder::from_rect(rect)));
    }

    fn pop_clip(&mut self) {
        if let Some(old) = self.clip_stack.pop() {
            self.clip = old;
        }
    }

    fn fill(&mut self, brush: Brush<'_>) {
        let paint = match self.make_paint(&brush) {
            Some(p) => p,
            None => return,
        };
        // Fill the whole font-unit area; the clip mask restricts visibility.
        let rect = Rect::from_xywh(-self.upem, -self.upem, self.upem * 2.0, self.upem * 2.0);
        if let Some(rect) = rect {
            self.pixmap
                .fill_rect(rect, &paint, self.transform, self.clip.as_ref());
        }
    }

    fn push_layer(&mut self, mode: CompositeMode) {
        let backdrop = self.pixmap.clone();
        self.layer_stack.push((backdrop, mode));
        self.pixmap = Pixmap::new(self.w, self.h).expect("pixmap allocation");
    }

    fn pop_layer_with_mode(&mut self, mode: CompositeMode) {
        if let Some((backdrop, _)) = self.layer_stack.pop() {
            let layer = std::mem::replace(&mut self.pixmap, backdrop);
            let pp = PixmapPaint {
                opacity: 1.0,
                blend_mode: map_blend(mode),
                quality: FilterQuality::Nearest,
            };
            self.pixmap
                .draw_pixmap(0, 0, layer.as_ref(), &pp, Transform::identity(), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colr_emoji_renders_color() {
        // Grinning face: should produce a non-trivial, multi-colored bitmap.
        let Some((w, h, buf)) = render_emoji_colr(0x1F600, 32) else {
            // No color emoji font on this system; skip.
            return;
        };
        assert_eq!(w, 32);
        assert_eq!(h, 32);
        assert_eq!(buf.len(), (w * h * 4) as usize);
        let mut opaque = 0usize;
        let mut colors = std::collections::HashSet::new();
        for chunk in buf.chunks_exact(4) {
            if chunk[3] > 0 {
                opaque += 1;
                colors.insert((chunk[0] / 32, chunk[1] / 32, chunk[2] / 32));
            }
        }
        assert!(opaque > 50, "emoji should have substantial coverage");
        assert!(colors.len() > 2, "emoji should show multiple colors");
    }
}
