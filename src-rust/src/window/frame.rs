use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, EventMask, WindowClass, ConnectionExt as _};

use crate::core::Rectangle;
use crate::render::font::Font;
use crate::render::texture::{Texture, TextureRender};
use crate::x11::X11Connection;

pub struct FbWinFrame {
    frame_window: u32,
    title_window: u32,
    handle_window: u32,
    client_window: u32,
    width: u16,
    height: u16,
    title_height: u16,
    border_width: u16,
    bevel_width: u16,
    title_visible: bool,
    handles_visible: bool,
    tab_width: u16,
    label: String,
    iconified: bool,
    maximized: bool,
    shaded: bool,
    focused: bool,
    texture: Texture,
    gc: Option<xproto::Gcontext>,
    font: Font,
    close_rect: Rectangle,
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
        // override_redirect so (un)mapping the frame never generates a
        // MapRequest/ConfigureRequest back to us (avoids a manage loop).
        conn.conn().create_window(
            0, frame, parent,
            0, 0, width, height.saturating_add(title_height).saturating_add(border_width * 2),
            border_width,
            WindowClass::INPUT_OUTPUT,
            0,
            &xproto::CreateWindowAux::new()
                .override_redirect(1)
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
            &xproto::CreateWindowAux::new().background_pixel(conn.screen().white_pixel),
        )?;

        conn.conn().reparent_window(
            client, frame,
            border_width as i16,
            (border_width + title_height) as i16,
        )?;

        // One GC for the frame's lifetime; reused for every title redraw.
        let gc = conn.conn().generate_id()?;
        conn.conn().create_gc(gc, frame, &xproto::CreateGCAux::new())?;

        // Title-bar font (falls back to a no-op font if "fixed" is missing).
        let font = Font::load_x11_font(conn.conn(), "fixed")
            .unwrap_or_else(|_| Font::new("fixed"));
        log::debug!("frame font x_id present: {}", font.x_id().is_some());

        // Close button sits on the right edge of the title bar.
        let btn: u16 = 16;
        let close_rect = Rectangle::new(
            (width.saturating_sub(btn + 2)) as i16,
            3,
            btn,
            btn,
        );

        Ok(Self {
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
            texture: Texture::new(),
            gc: Some(gc),
            font,
            close_rect,
        })
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

    pub fn resize(&self, conn: &X11Connection, width: u16, height: u16) -> Result<(), anyhow::Error> {
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

    pub fn show(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        conn.conn().map_window(self.frame_window)?;
        conn.conn().map_window(self.title_window)?;
        if !self.shaded {
            conn.conn().map_window(self.handle_window)?;
        }
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

    pub fn draw_titlebar(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        let gc = match self.gc {
            Some(g) => g,
            None => return Ok(()),
        };

        let (fg, bg) = if self.focused {
            (conn.screen().white_pixel, conn.screen().black_pixel)
        } else {
            (conn.screen().black_pixel, conn.screen().white_pixel)
        };

        conn.conn().change_gc(gc, &xproto::ChangeGCAux::new().foreground(bg))?;
        conn.conn().poly_fill_rectangle(
            self.title_window,
            gc,
            &[xproto::Rectangle {
                x: 0,
                y: 0,
                width: self.width,
                height: self.title_height,
            }],
        )?;

        // Bevel highlight along the top/left of the title for a 3D look.
        TextureRender::render_bevel(
            conn,
            self.title_window,
            gc,
            &Rectangle::new(0, 0, self.width, self.title_height),
            self.bevel_width,
            true,
            fg,
            bg,
        )?;

        // Window title text, left-aligned and clipped to avoid the close button.
        if !self.label.is_empty() {
            let avail = (self.close_rect.x - 6).max(4);
            let mut disp = self.label.clone();
            while !disp.is_empty()
                && self.font.text_width(conn.conn(), &disp).unwrap_or(0) as i16 > avail
            {
                disp.pop();
            }
            if disp.len() < self.label.len() {
                disp.push('…');
            }
            let ty = self.title_height as i16 / 2 + self.font.height() as i16 / 2
                - self.font.descent() as i16;
            conn.conn()
                .change_gc(gc, &xproto::ChangeGCAux::new().foreground(fg))?;
            self.font
                .draw_text(conn.conn(), self.title_window, gc, 4, ty, &disp)?;
        }

        // Close button: a small bevelled box with an "x" glyph.
        conn.conn().change_gc(gc, &xproto::ChangeGCAux::new().foreground(bg))?;
        conn.conn().poly_fill_rectangle(
            self.title_window,
            gc,
            &[xproto::Rectangle {
                x: self.close_rect.x,
                y: self.close_rect.y,
                width: self.close_rect.width,
                height: self.close_rect.height,
            }],
        )?;
        TextureRender::render_bevel(
            conn,
            self.title_window,
            gc,
            &self.close_rect,
            self.bevel_width,
            false,
            fg,
            bg,
        )?;
        let cx = self.close_rect.x
            + (self.close_rect.width as i16 - self.font.text_width(conn.conn(), "x").unwrap_or(0) as i16)
                / 2;
        let cy = self.close_rect.y
            + (self.close_rect.height as i16 + self.font.height() as i16) / 2
            - self.font.descent() as i16;
        conn.conn()
            .change_gc(gc, &xproto::ChangeGCAux::new().foreground(fg))?;
        self.font
            .draw_text(conn.conn(), self.title_window, gc, cx, cy, "x")?;

        Ok(())
    }

    /// Update the title-bar label (caller is responsible for redrawing).
    pub fn set_label(&mut self, label: String) {
        self.label = label;
    }

    /// True if the press at (x, y) in title-window coordinates hit the close button.
    pub fn is_close_press(&self, x: i16, y: i16) -> bool {
        self.close_rect.contains(x, y)
    }

    /// Free the frame's resources. Safe to call once when the window is
    /// unmanaged; the client itself is reparented back to root by the caller.
    pub fn destroy(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        if let Some(gc) = self.gc {
            conn.conn().free_gc(gc)?;
        }
        conn.conn().destroy_window(self.title_window)?;
        conn.conn().destroy_window(self.handle_window)?;
        conn.conn().destroy_window(self.frame_window)?;
        Ok(())
    }
}
