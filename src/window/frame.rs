use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, EventMask, WindowClass, ConnectionExt as _};
use x11rb::protocol::xproto::Segment;

use crate::core::Rectangle;
use crate::render::font::Font;
use crate::render::texture::{Texture, TextureRender};
use crate::x11::X11Connection;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ButtonType {
    Close,
    Maximize,
    Iconify,
    Shade,
    Stick,
    Menu,
}

pub struct FrameButton {
    pub rect: Rectangle,
    pub btn_type: ButtonType,
    pub pressed: bool,
}

impl FrameButton {
    fn new(btn_type: ButtonType, x: i16, y: i16, size: u16) -> Self {
        Self { rect: Rectangle::new(x, y, size, size), btn_type, pressed: false }
    }

    fn draw_glyph(&self, conn: &X11Connection, w: u32, gc: u32) -> Result<(), anyhow::Error> {
        match self.btn_type {
            ButtonType::Close => draw_close_glyph(conn, w, gc, &self.rect),
            ButtonType::Maximize => draw_maximize_glyph(conn, w, gc, &self.rect),
            ButtonType::Iconify => draw_iconify_glyph(conn, w, gc, &self.rect),
            _ => Ok(()),
        }
    }
}

pub struct FbWinFrame {
    pub frame_window: u32,
    pub title_window: u32,
    pub handle_window: u32,
    pub client_window: u32,
    width: u16,
    height: u16,
    title_height: u16,
    border_width: u16,
    bevel_width: u16,
    title_visible: bool,
    handles_visible: bool,
    tab_width: u16,
    label: String,
    pub iconified: bool,
    pub maximized: bool,
    pub shaded: bool,
    pub focused: bool,
    pub no_maximize: bool,
    texture: Texture,
    gc: Option<xproto::Gcontext>,
    font: Font,
    buttons: Vec<FrameButton>,
    resize_cursor: Option<u32>,
}

const BTN_SIZE: u16 = 18;
const BTN_GAP: i16 = 2;

// Helper: draw a button shape using segments/rectangles instead of text.

fn draw_close_glyph(conn: &X11Connection, w: u32, gc: u32, r: &Rectangle) -> Result<(), anyhow::Error> {
    let inset = 3i16;
    let x1 = r.x + inset;
    let y1 = r.y + inset;
    let x2 = r.x + r.width as i16 - inset;
    let y2 = r.y + r.height as i16 - inset;
    let segs = &[
        Segment { x1, y1, x2, y2 },
        Segment { x1: x2, y1, x2: x1, y2 },
    ];
    conn.conn().poly_segment(w, gc, segs)?;
    Ok(())
}

fn draw_maximize_glyph(conn: &X11Connection, w: u32, gc: u32, r: &Rectangle) -> Result<(), anyhow::Error> {
    let inset = 3i16;
    let x = r.x + inset;
    let y = r.y + inset;
    let sz = r.width as i16 - inset * 2;
    let rects = &[xproto::Rectangle { x, y, width: sz as u16, height: sz as u16 }];
    conn.conn().poly_rectangle(w, gc, rects)?;
    Ok(())
}

fn draw_iconify_glyph(conn: &X11Connection, w: u32, gc: u32, r: &Rectangle) -> Result<(), anyhow::Error> {
    let inset = 4i16;
    let y = r.y + r.height as i16 / 2;
    let segs = &[
        Segment { x1: r.x + inset, y1: y, x2: r.x + r.width as i16 - inset, y2: y },
    ];
    conn.conn().poly_segment(w, gc, segs)?;
    Ok(())
}

impl FbWinFrame {
    pub fn new(
        conn: &X11Connection,
        parent: u32,
        client: u32,
        width: u16,
        height: u16,
    ) -> Result<Self, anyhow::Error> {
        let title_height = 22;
        let border_width = 2u16;
        let bevel_width = 2;

        let frame = conn.conn().generate_id()?;
        conn.conn().create_window(
            0, frame, parent,
            0, 0, width, height.saturating_add(title_height).saturating_add(border_width * 2),
            border_width,
            WindowClass::INPUT_OUTPUT,
            0,
            &xproto::CreateWindowAux::new()
                .background_pixel(conn.screen().white_pixel)
                .event_mask(
                    EventMask::EXPOSURE
                        | EventMask::BUTTON_PRESS
                        | EventMask::BUTTON_RELEASE
                        | EventMask::POINTER_MOTION
                        | EventMask::ENTER_WINDOW
                        | EventMask::LEAVE_WINDOW
                        | EventMask::SUBSTRUCTURE_REDIRECT
                        | EventMask::SUBSTRUCTURE_NOTIFY,
                ),
        )?;

        let title = conn.conn().generate_id()?;
        conn.conn().create_window(
            0, title, frame,
            border_width as i16, border_width as i16,
            width, title_height,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &xproto::CreateWindowAux::new()
                .background_pixel(conn.screen().white_pixel)
                .event_mask(
                    EventMask::EXPOSURE
                        | EventMask::BUTTON_PRESS
                        | EventMask::BUTTON_RELEASE
                        | EventMask::POINTER_MOTION,
                ),
        )?;

        let handle = conn.conn().generate_id()?;
        conn.conn().create_window(
            0, handle, frame,
            0, 0, 1, 1,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &xproto::CreateWindowAux::new()
                .background_pixel(conn.screen().white_pixel)
                .event_mask(
                    EventMask::BUTTON_PRESS
                        | EventMask::BUTTON_RELEASE
                        | EventMask::POINTER_MOTION
                        | EventMask::ENTER_WINDOW
                        | EventMask::LEAVE_WINDOW,
                ),
        )?;

        conn.conn().reparent_window(
            client, frame,
            border_width as i16,
            (border_width + title_height) as i16,
        )?;

        let gc = conn.conn().generate_id()?;
        conn.conn().create_gc(gc, frame, &xproto::CreateGCAux::new())?;

        let font = Font::new("fixed");

        // Best-effort: create a resize cursor for the handle_window.
        let resize_cursor = (|| -> Option<u32> {
            let font = conn.conn().generate_id().ok()?;
            conn.conn().open_font(font, b"cursor").ok()?;
            let cursor = conn.conn().generate_id().ok()?;
            conn.conn().create_glyph_cursor(
                cursor, font, font,
                96, 96, // XC_bottom_right_corner
                0, 0, 0, 0xffff, 0xffff, 0xffff,
            ).ok()?;
            let _ = conn.conn().close_font(font);
            Some(cursor)
        })();

        let mut frame_ = Self {
            frame_window: frame,
            title_window: title,
            handle_window: handle,
            client_window: client,
            width,
            height,
            title_height,
            border_width,
            bevel_width,
            title_visible: true,
            handles_visible: true,
            tab_width: 64,
            label: String::new(),
            iconified: false,
            maximized: false,
            shaded: false,
            focused: false,
            no_maximize: false,
            texture: Texture::new(),
            gc: Some(gc),
            font,
            buttons: Vec::new(),
            resize_cursor,
        };
        frame_.layout_buttons();
        if let Some(cb) = crate::hooks::AFTER_FRAME_CREATE.get() {
            let fh = height.saturating_add(title_height).saturating_add(border_width * 2);
            cb(conn, frame, width, fh);
        }
        Ok(frame_)
    }

    pub fn layout_buttons(&mut self) {
        self.buttons.clear();
        let count = if self.no_maximize { 2u16 } else { 3u16 };
        let x_start = (self.width.saturating_sub(count * (BTN_SIZE + BTN_GAP as u16) + 2)) as i16;
        let y = 3i16;
        self.buttons.push(FrameButton::new(ButtonType::Iconify, x_start, y, BTN_SIZE));
        if !self.no_maximize {
            self.buttons.push(FrameButton::new(
                ButtonType::Maximize,
                x_start + (BTN_SIZE as i16 + BTN_GAP),
                y,
                BTN_SIZE,
            ));
        }
        self.buttons.push(FrameButton::new(
            ButtonType::Close,
            x_start + (count - 1) as i16 * (BTN_SIZE as i16 + BTN_GAP),
            y,
            BTN_SIZE,
        ));
    }

    pub fn frame_window(&self) -> u32 {
        self.frame_window
    }

    pub fn title_window(&self) -> u32 {
        self.title_window
    }

    pub fn handle_window(&self) -> u32 {
        self.handle_window
    }

    /// Width of the client area (excludes borders, includes frame border).
    pub fn client_width(&self) -> u16 {
        self.width
    }

    pub fn client_height(&self) -> u16 {
        self.height
    }

    pub fn border_width(&self) -> u16 {
        self.border_width
    }

    pub fn title_height(&self) -> u16 {
        self.title_height
    }

    fn configure_handle(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        const HANDLE_H: u16 = 6;
        if self.shaded {
            let _ = conn.conn().unmap_window(self.handle_window);
        } else {
            let bw = self.border_width as i16;
            let handle_y = (bw + self.title_height as i16 + self.height as i16)
                .saturating_sub(HANDLE_H as i16)
                .max(bw);
            let _ = conn.conn().map_window(self.handle_window);
            let aux = xproto::ConfigureWindowAux::new()
                .x(bw as i32)
                .y(handle_y as i32)
                .width(self.width as u32)
                .height(HANDLE_H as u32);
            if let Some(cursor) = self.resize_cursor {
                let _ = conn.conn().change_window_attributes(
                    self.handle_window,
                    &xproto::ChangeWindowAttributesAux::new().cursor(cursor),
                );
            }
            conn.conn().configure_window(self.handle_window, &aux)?;
        }
        Ok(())
    }

    pub fn resize(&mut self, conn: &X11Connection, width: u16, height: u16) -> Result<(), anyhow::Error> {
        self.width = width;
        self.height = height;
        let frame_height = if self.shaded {
            self.title_height + self.border_width * 2
        } else {
            height.saturating_add(self.title_height).saturating_add(self.border_width * 2)
        };

        conn.conn().configure_window(
            self.frame_window,
            &xproto::ConfigureWindowAux::new()
                .width(width as u32)
                .height(frame_height as u32),
        )?;
        conn.conn().configure_window(
            self.title_window,
            &xproto::ConfigureWindowAux::new().width(width as u32),
        )?;

        self.configure_handle(conn)?;
        self.layout_buttons();
        if let Some(cb) = crate::hooks::AFTER_FRAME_RESIZE.get() {
            let fh = if self.shaded {
                self.title_height + self.border_width * 2
            } else {
                height.saturating_add(self.title_height).saturating_add(self.border_width * 2)
            };
            cb(conn, self.frame_window, width, fh);
        }
        Ok(())
    }

    pub fn move_resize(
        &mut self,
        conn: &X11Connection,
        x: i16, y: i16,
        w: u16, h: u16,
    ) -> Result<(), anyhow::Error> {
        self.width = w;
        self.height = h;
        let fh = if self.shaded {
            self.title_height + self.border_width * 2
        } else {
            h.saturating_add(self.title_height).saturating_add(self.border_width * 2)
        };
        conn.conn().configure_window(
            self.frame_window,
            &xproto::ConfigureWindowAux::new()
                .x(x as i32)
                .y(y as i32)
                .width(w as u32)
                .height(fh as u32),
        )?;
        conn.conn().configure_window(
            self.title_window,
            &xproto::ConfigureWindowAux::new().width(w as u32),
        )?;
        self.configure_handle(conn)?;
        self.layout_buttons();
        if let Some(cb) = crate::hooks::AFTER_FRAME_RESIZE.get() {
            cb(conn, self.frame_window, w, fh);
        }
        Ok(())
    }

    pub fn move_to(&self, conn: &X11Connection, x: i16, y: i16) -> Result<(), anyhow::Error> {
        conn.conn().configure_window(
            self.frame_window,
            &xproto::ConfigureWindowAux::new()
                .x(x as i32)
                .y(y as i32),
        )?;
        Ok(())
    }

    pub fn show(&mut self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        self.configure_handle(conn)?;
        conn.conn().map_window(self.frame_window)?;
        conn.conn().map_window(self.title_window)?;
        if !self.shaded {
            let _ = conn.conn().map_window(self.handle_window);
        }
        // Repaint the titlebar: after being iconified the decoration windows
        // were unmapped and would otherwise show their blank white background
        // when remapped. The iconified flag toggled, so draw_titlebar will not
        // early-return here.
        self.draw_titlebar(conn)?;
        Ok(())
    }

    pub fn hide(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        conn.conn().unmap_window(self.frame_window)?;
        Ok(())
    }

    pub fn raise(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        conn.conn().configure_window(
            self.frame_window,
            &xproto::ConfigureWindowAux::new()
                .stack_mode(xproto::StackMode::ABOVE),
        )?;
        Ok(())
    }

    pub fn set_focused(&mut self, focused: bool, conn: &X11Connection) -> Result<(), anyhow::Error> {
        self.focused = focused;
        if self.title_visible {
            conn.conn().clear_area(true, self.title_window, 0, 0, 0, 0)?;
            self.draw_titlebar(conn)?;
        }
        Ok(())
    }

    fn draw_button(&self, conn: &X11Connection, gc: u32, btn: &FrameButton, fg: u32, bg: u32) -> Result<(), anyhow::Error> {
        // Background
        conn.conn().change_gc(gc, &xproto::ChangeGCAux::new().foreground(bg))?;
        conn.conn().poly_fill_rectangle(
            self.title_window,
            gc,
            &[xproto::Rectangle {
                x: btn.rect.x,
                y: btn.rect.y,
                width: btn.rect.width,
                height: btn.rect.height,
            }],
        )?;
        // Bevel
        TextureRender::render_bevel(
            conn, self.title_window, gc, &btn.rect, self.bevel_width, false, fg, bg,
        )?;
        // White border
        conn.conn().change_gc(gc, &xproto::ChangeGCAux::new().foreground(fg))?;
        conn.conn().poly_rectangle(
            self.title_window,
            gc,
            &[xproto::Rectangle {
                x: btn.rect.x,
                y: btn.rect.y,
                width: btn.rect.width,
                height: btn.rect.height,
            }],
        )?;
        // Glyph shape in foreground color
        btn.draw_glyph(conn, self.title_window, gc)?;
        Ok(())
    }

    pub fn draw_titlebar(&mut self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        let gc = match self.gc {
            Some(g) => g,
            None => return Ok(()),
        };

        let default = conn.screen().white_pixel;
        let (fg, bg) = if self.focused {
            let fg = crate::hooks::or(&crate::hooks::FRAME_FOCUSED_FG, default);
            let bg = crate::hooks::or(&crate::hooks::FRAME_FOCUSED_BG, conn.screen().black_pixel);
            (fg, bg)
        } else {
            let fg = crate::hooks::or(&crate::hooks::FRAME_FG, conn.screen().black_pixel);
            let bg = crate::hooks::or(&crate::hooks::FRAME_BG, default);
            (fg, bg)
        };

        conn.conn().change_gc(gc, &xproto::ChangeGCAux::new().foreground(bg))?;
        conn.conn().poly_fill_rectangle(
            self.title_window,
            gc,
            &[xproto::Rectangle {
                x: 0, y: 0,
                width: self.width,
                height: self.title_height,
            }],
        )?;

        TextureRender::render_bevel(
            conn, self.title_window, gc,
            &Rectangle::new(0, 0, self.width, self.title_height),
            self.bevel_width, true, fg, bg,
        )?;

        let btn_right = self.buttons.last().map(|b| b.rect.right()).unwrap_or(6);
        let avail = (btn_right - 6).max(4);
        if !self.label.is_empty() {
            let disp = self.font.truncate_ellipsis(&self.label, avail);
            let ty = self.title_height as i16 / 2 + self.font.height() as i16 / 2
                - self.font.descent() as i16;
            self.font.draw_text_on_bg(conn.conn(), self.title_window, gc, 4, ty, &disp, fg, bg)?;
        }

        for btn in &self.buttons {
            self.draw_button(conn, gc, btn, fg, bg)?;
        }

        Ok(())
    }

    pub fn set_label(&mut self, label: String) {
        self.label = label;
    }

    /// Returns which button was pressed, or None.
    pub fn hit_test_button(&self, x: i16, y: i16) -> Option<ButtonType> {
        for btn in &self.buttons {
            if btn.rect.contains(x, y) {
                return Some(btn.btn_type);
            }
        }
        None
    }

    /// Returns true if the press at (x, y) is on the title bar (not on a button).
    pub fn is_titlebar_press(&self, x: i16, y: i16) -> bool {
        y >= 0 && y < self.title_height as i16
            && x >= 0 && x < self.width as i16
            && self.hit_test_button(x, y).is_none()
    }

    pub fn destroy(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        if let Some(gc) = self.gc {
            conn.conn().free_gc(gc)?;
        }
        if let Some(cursor) = self.resize_cursor {
            let _ = conn.conn().free_cursor(cursor);
        }
        conn.conn().destroy_window(self.title_window)?;
        conn.conn().destroy_window(self.handle_window)?;
        conn.conn().destroy_window(self.frame_window)?;
        Ok(())
    }
}
