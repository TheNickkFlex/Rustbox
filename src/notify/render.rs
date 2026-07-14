//! Lightweight X11 popup rendering for notifications.
//!
//! Each notification is one override-redirect window drawn with the WM's
//! bitmap font and core X primitives. No cairo/pango dependency.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, EventMask, WindowClass, ConnectionExt as _};

use crate::core::Rectangle;
use crate::render::image::ImageControl;
use crate::render::Font;
use crate::x11::X11Connection;

use super::{Corner, RawNotification, Theme, Urgency};

const PAD: i16 = 8;
const ICON: u16 = 32;

/// Outcome of a click inside a popup.
pub enum ClickResult {
    /// An explicit action button was clicked (key is the action identifier).
    Action(String),
    /// Anywhere else on the popup was clicked => dismiss.
    Dismiss,
}

/// A single on-screen notification popup.
pub struct Popup {
    pub window: u32,
    gc: u32,
    pub notif: RawNotification,
    x: i16,
    y: i16,
    w: u16,
    h: u16,
    font: Font,
    icon_pix: Option<u32>,
    theme: Theme,
    action_rects: Vec<(Rectangle, String)>,
    action_labels: Vec<String>,
    // Cached pixels to avoid synchronous alloc_color round-trips on redraws.
    bg_pixel: u32,
    fg_pixel: u32,
    frame_pixel: u32,
    body_pixel: u32,
    urgency_pixel: u32,
}

impl Popup {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        conn: &X11Connection,
        notif: RawNotification,
        screen_w: u16,
        screen_h: u16,
        index: usize,
        _max_visible: usize,
        origin: Corner,
        margin: i16,
        gap: i16,
        width: u16,
        scale: u16,
        icon_cache: &mut ImageControl,
        theme: Theme,
    ) -> Result<Self, anyhow::Error> {
        let c = conn.conn();
        let px = (scale * 7).max(10) as u32; // old scale=2 → 14px
        let mut font = Font::new("sans-serif");
        font.set_pixel_size(px);
        let (w, h) = Self::measure(&notif, &font, conn, width);
        let (x, y) = Self::position(origin, w, h, screen_w, screen_h, margin, gap, index);

        let window = c.generate_id()?;
        c.create_window(
            0,
            window,
            conn.root_window(),
            x,
            y,
            w,
            h,
            1,
            WindowClass::INPUT_OUTPUT,
            0,
            &xproto::CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(conn.screen().black_pixel)
                .event_mask(
                    EventMask::EXPOSURE | EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                ),
        )?;

        let gc = c.generate_id()?;
        c.create_gc(
            gc,
            window,
            &xproto::CreateGCAux::new().foreground(conn.screen().white_pixel),
        )?;

        // Load the app icon: an explicit file path, in-memory `image-data`
        // bytes, or nothing (placeholder initial is drawn instead).
        let icon_pix = if !notif.app_icon.is_empty() {
            icon_cache
                .load(&notif.app_icon)
                .and_then(|img| img.scale(ICON as u32, ICON as u32))
                .and_then(|img| img.create_pixmap(c, conn.screen(), window))
                .map(Some)
                .unwrap_or_else(|e| {
                    log::warn!("Failed to load icon '{}': {}", notif.app_icon, e);
                    None
                })
        } else if let Some(data) = &notif.icon_data {
            icon_cache
                .load_memory(data)
                .and_then(|img| img.scale(ICON as u32, ICON as u32))
                .and_then(|img| img.create_pixmap(c, conn.screen(), window))
                .map(Some)
                .unwrap_or_else(|e| {
                    log::warn!("Failed to decode image-data icon: {}", e);
                    None
                })
        } else {
            None
        };

        let white = conn.screen().white_pixel;
        let black = conn.screen().black_pixel;
        let bg_pixel = color(conn, theme.bg.0, theme.bg.1, theme.bg.2, black);
        let fg_pixel = color(conn, theme.fg.0, theme.fg.1, theme.fg.2, white);
        let frame_pixel = color(conn, theme.frame.0, theme.frame.1, theme.frame.2, black);
        let body_pixel = color(conn, theme.body.0, theme.body.1, theme.body.2, white);
        let urgency_pixel = match notif.urgency {
            Urgency::Low => color(conn, theme.urgency[0].0, theme.urgency[0].1, theme.urgency[0].2, white),
            Urgency::Normal => color(conn, theme.urgency[1].0, theme.urgency[1].1, theme.urgency[1].2, white),
            Urgency::Critical => color(conn, theme.urgency[2].0, theme.urgency[2].1, theme.urgency[2].2, white),
        };

        let mut p = Self {
            window,
            gc,
            notif,
            x,
            y,
            w,
            h,
            font,
            icon_pix,
            theme,
            action_rects: Vec::new(),
            action_labels: Vec::new(),
            bg_pixel,
            fg_pixel,
            frame_pixel,
            body_pixel,
            urgency_pixel,
        };
        p.compute_layout();
        p.redraw(conn)?;
        c.map_window(window)?;
        c.flush()?;
        if let Some(cb) = crate::hooks::AFTER_NOTIFY_CREATE.get() {
            cb(conn, window, w, h);
        }
        Ok(p)
    }

    /// Recompute position (called on layout changes / screen resize).
    pub fn reposition(
        &mut self,
        conn: &X11Connection,
        screen_w: u16,
        screen_h: u16,
        index: usize,
        origin: Corner,
        margin: i16,
        gap: i16,
    ) {
        let (x, y) = Self::position(origin, self.w, self.h, screen_w, screen_h, margin, gap, index);
        self.x = x;
        self.y = y;
        let _ = conn.conn().configure_window(
            self.window,
            &xproto::ConfigureWindowAux::new()
                .x(x as i32)
                .y(y as i32)
                .stack_mode(xproto::StackMode::ABOVE),
        );
    }

    /// Width and height for the given content and fixed width.
    fn measure(notif: &RawNotification, font: &Font, conn: &X11Connection, width: u16) -> (u16, u16) {
        let fh = font.height() as i16;
        let body_w = width
            .saturating_sub((PAD * 2 + ICON as i16 + PAD) as u16)
            .max(40);

        let summary_h = fh + 4;
        let lines = font.wrap(&notif.body, body_w as u32);
        let body_h = lines.len() as i16 * (fh + 1);

        let mut h: i16 = PAD + summary_h + 4 + body_h + PAD;
        if !notif.actions.is_empty() {
            h += fh + 6 + PAD;
        }
        if notif.progress.is_some() {
            h += fh + 4;
        }
        let min_h = ICON as i16 + PAD * 2;
        h = h.max(min_h);
        (width, h as u16)
    }

    fn position(
        origin: Corner,
        w: u16,
        h: u16,
        screen_w: u16,
        screen_h: u16,
        margin: i16,
        gap: i16,
        index: usize,
    ) -> (i16, i16) {
        let off = margin + index as i16 * (h as i16 + gap);
        match origin {
            Corner::TopRight => (screen_w as i16 - margin - w as i16, off),
            Corner::TopLeft => (margin, off),
            Corner::BottomRight => {
                (screen_w as i16 - margin - w as i16, screen_h as i16 - margin - off - h as i16)
            }
            Corner::BottomLeft => (margin, screen_h as i16 - margin - off - h as i16),
        }
    }

    /// Compute action button rectangles (popup-local coordinates).
    fn compute_layout(&mut self) {
        self.action_rects.clear();
        if self.notif.actions.is_empty() {
            return;
        }
        let btn_h = self.font.height() as i16 + 6;
        let y = self.h as i16 - PAD - btn_h;
        let n = self.notif.actions.len();
        let avail = (self.w as i16) - PAD * 2;
        let gap_b = 6i16;
        let bw = (avail - gap_b * (n as i16 - 1)) / n as i16;
        let mut x = PAD;
        for (key, label) in &self.notif.actions {
            let w = bw.max(20);
            self.action_rects.push((
                Rectangle::new(x, y, w as u16, btn_h as u16),
                key.clone(),
            ));
            // keep label for drawing
            let _ = label;
            x += w + gap_b;
        }
        // store labels separately for drawing
        self.action_labels = self
            .notif
            .actions
            .iter()
            .map(|(_, l)| l.clone())
            .collect();
    }

    /// Draw the popup contents.
    pub fn redraw(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        let c = conn.conn();
        let white = conn.screen().white_pixel;

        let bg = self.bg_pixel;
        let fg = self.fg_pixel;
        let frame = self.frame_pixel;
        let body_c = self.body_pixel;
        let rc = conn.conn();
        let ucolor = self.urgency_pixel;

        // Background.
        self.set_fg(conn, bg);
        c.poly_fill_rectangle(
            self.window,
            self.gc,
            &[xproto::Rectangle {
                x: 0,
                y: 0,
                width: self.w,
                height: self.h,
            }],
        )?;

        // Border.
        self.set_fg(conn, fg);
        c.poly_rectangle(
            self.window,
            self.gc,
            &[xproto::Rectangle {
                x: 0,
                y: 0,
                width: self.w,
                height: self.h,
            }],
        )?;

        // Urgency bar (left strip).
        self.set_fg(conn, ucolor);
        c.poly_fill_rectangle(
            self.window,
            self.gc,
            &[xproto::Rectangle {
                x: 0,
                y: 0,
                width: 4,
                height: self.h,
            }],
        )?;

        // Icon: real image if available, else a placeholder with the app's initial.
        let fh = self.font.height() as i16;
        let ix = PAD;
        let iy = PAD;
        self.set_fg(conn, frame);
        c.poly_fill_rectangle(
            self.window,
            self.gc,
            &[xproto::Rectangle {
                x: ix,
                y: iy,
                width: ICON,
                height: ICON,
            }],
        )?;
        if let Some(px) = self.icon_pix {
            c.copy_area(px, self.window, self.gc, 0, 0, ix, iy, ICON, ICON)?;
        } else {
            let initial: String = self
                .notif
                .app_name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_else(|| "?".into());
            let ix2 = ix + (ICON as i16 - self.font.text_width(rc, &initial)? as i16) / 2;
            self.font.draw_text_on_bg(rc, self.window, self.gc, ix2, iy + 14, &initial, white, bg)?;
        }

        // Summary.
        let text_x = PAD + ICON as i16 + PAD;
        let summary = strip_markup(&self.notif.summary);
        self.set_fg(conn, fg);
        self.font.draw_text_on_bg(rc, self.window, self.gc, text_x, PAD + fh, &summary, fg, bg)?;

        // Body (wrapped).
        let body_w = self.w as i16 - PAD * 2 - ICON as i16 - PAD;
        let body = strip_markup(&self.notif.body);
        let lines: Vec<String> = self.font.wrap(&body, body_w as u32);
        let max_lines = if self.notif.actions.is_empty() { 4 } else { 3 };
        self.set_fg(conn, body_c);
        let mut ly = PAD + fh + 4 + fh;
        for line in lines.iter().take(max_lines) {
            self.font.draw_text_on_bg(rc, self.window, self.gc, text_x, ly, line, body_c, bg)?;
            ly += fh + 1;
        }
        if lines.len() > max_lines {
            let more = format!("+{}", lines.len() - max_lines);
            self.font.draw_text_on_bg(rc, self.window, self.gc, text_x, ly, &more, body_c, bg)?;
            ly += fh + 1;
        }

        // Progress bar.
        if let Some(p) = self.notif.progress {
            let p = p.min(100);
            let bx = text_x;
            let bw = (self.w as i16 - PAD - text_x).max(20);
            let by = ly + 2;
            // Track.
            self.set_fg(conn, frame);
            c.poly_fill_rectangle(
                self.window,
                self.gc,
                &[xproto::Rectangle {
                    x: bx,
                    y: by,
                    width: bw as u16,
                    height: 6,
                }],
            )?;
            // Fill.
            let fw = ((bw as i32 * p as i32) / 100).max(0).min(bw as i32) as u16;
            self.set_fg(conn, ucolor);
            if fw > 0 {
                c.poly_fill_rectangle(
                    self.window,
                    self.gc,
                    &[xproto::Rectangle {
                        x: bx,
                        y: by,
                        width: fw,
                        height: 6,
                    }],
                )?;
            }
            // Percentage label.
            self.set_fg(conn, fg);
            let pct = format!("{}%", p);
            let lw = self.font.text_width(rc, &pct)?;
            let lx = (bx + bw - lw as i16).max(bx);
            self.font.draw_text_on_bg(rc, self.window, self.gc, lx, by - 2, &pct, fg, bg)?;
            ly += fh + 2;
        }

        // Action buttons.
        if !self.action_rects.is_empty() {
            self.set_fg(conn, frame);
            for (r, _) in &self.action_rects {
                c.poly_fill_rectangle(self.window, self.gc, &[xproto::Rectangle {
                    x: r.x,
                    y: r.y,
                    width: r.width,
                    height: r.height,
                }])?;
            }
            self.set_fg(conn, fg);
            let btn_fh = (fh + 6) as i16;
            for (i, (r, _)) in self.action_rects.iter().enumerate() {
                let label = self.action_labels.get(i).map(|s| s.as_str()).unwrap_or("");
                let lw = self.font.text_width(rc, label)?;
                let lx = r.x + ((r.width as i16 - lw as i16) / 2).max(2);
                let ly = r.y + btn_fh / 2 + fh / 3;
                self.font.draw_text_on_bg(rc, self.window, self.gc, lx, ly, label, fg, bg)?;
            }
        }

        Ok(())
    }

    fn set_fg(&self, conn: &X11Connection, color: u32) {
        let _ = conn.conn().change_gc(
            self.gc,
            &xproto::ChangeGCAux::new().foreground(color),
        );
    }

    /// Hit-test a click in popup-local coordinates.
    pub fn hit_test(&self, x: i16, y: i16) -> Option<ClickResult> {
        for (r, key) in &self.action_rects {
            if x >= r.x && x < r.x + r.width as i16 && y >= r.y && y < r.y + r.height as i16 {
                return Some(ClickResult::Action(key.clone()));
            }
        }
        if x >= 0 && x < self.w as i16 && y >= 0 && y < self.h as i16 {
            return Some(ClickResult::Dismiss);
        }
        None
    }

    pub fn destroy(&self, conn: &X11Connection) {
        let c = conn.conn();
        if let Some(px) = self.icon_pix {
            let _ = c.free_pixmap(px);
        }
        let pixels = [
            self.bg_pixel,
            self.fg_pixel,
            self.frame_pixel,
            self.body_pixel,
            self.urgency_pixel,
        ];
        let cmap = conn.screen().default_colormap;
        let _ = c.free_colors(cmap, 0, &pixels);
        let _ = c.free_gc(self.gc);
        let _ = c.destroy_window(self.window);
    }
}

/// Allocate an RGB color on the default colormap, falling back to `fallback`.
fn color(conn: &X11Connection, r: u8, g: u8, b: u8, fallback: u32) -> u32 {
    let cmap = conn.screen().default_colormap;
    match conn
        .conn()
        .alloc_color(cmap, (r as u16) << 8, (g as u16) << 8, (b as u16) << 8)
        .ok()
        .and_then(|c| c.reply().ok())
    {
        Some(reply) => reply.pixel,
        None => fallback,
    }
}

/// Strip a minimal subset of notification markup (the spec's `<b>`, `<i>`,
/// `<u>`, `<a>`, `<img>`, ...) down to plain text and decode the common
/// XML entities. The lightweight bitmap renderer has no styled glyphs, so
/// styling is dropped and only the text is kept.
fn strip_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if in_tag => {}
            other => out.push(other),
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}
