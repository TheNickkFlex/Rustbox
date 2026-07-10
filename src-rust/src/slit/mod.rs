use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, EventMask, WindowClass, ConnectionExt as _};

use crate::core::Strut;
use crate::x11::X11Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlitPlacement {
    Left,
    Right,
    Top,
    Bottom,
}

pub struct SlitStyle {
    pub placement: SlitPlacement,
    pub gap: u16,
}

impl Default for SlitStyle {
    fn default() -> Self {
        Self {
            placement: SlitPlacement::Right,
            gap: 2,
        }
    }
}

/// A Fluxbox-style slit: a dock container that reparents small "dockapp"
/// windows (e.g. wmaker/bbpager applets, typically `override_redirect` or
/// `_NET_WM_WINDOW_TYPE_DOCK`) into a strip at one edge of the screen.
pub struct FbSlit {
    window: u32,
    gc: u32,
    bg_pixel: u32,
    screen_width: u16,
    screen_height: u16,
    style: SlitStyle,
    /// Dock windows in stacking order, with their slot size.
    dock: Vec<(u32, u16, u16)>,
    thickness: u16,
    length: u16,
}

impl FbSlit {
    pub fn new(
        conn: &X11Connection,
        screen_width: u16,
        screen_height: u16,
        placement: SlitPlacement,
    ) -> Result<Self, anyhow::Error> {
        let style = SlitStyle {
            placement,
            ..SlitStyle::default()
        };

        let window = conn.conn().generate_id()?;
        conn.conn().create_window(
            0,
            window,
            conn.root_window(),
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &xproto::CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(conn.screen().white_pixel)
                .event_mask(
                    EventMask::EXPOSURE
                        | EventMask::SUBSTRUCTURE_NOTIFY
                        | EventMask::SUBSTRUCTURE_REDIRECT,
                ),
        )?;

        let gc = conn.conn().generate_id()?;
        conn.conn().create_gc(
            gc,
            window,
            &xproto::CreateGCAux::new().foreground(conn.screen().black_pixel),
        )?;

        Ok(Self {
            window,
            gc,
            bg_pixel: conn.screen().white_pixel,
            screen_width,
            screen_height,
            style,
            dock: Vec::new(),
            thickness: 0,
            length: 0,
        })
    }

    pub fn window_id(&self) -> u32 {
        self.window
    }

    /// True if `window` is the slit itself or one of the docked clients.
    pub fn owns_window(&self, window: u32) -> bool {
        if window == self.window {
            return true;
        }
        self.dock.iter().any(|(w, _, _)| *w == window)
    }

    /// Vertical/horizontal space the slit reserves on its edge.
    pub fn strut(&self) -> Strut {
        if self.thickness == 0 || self.length == 0 {
            return Strut::zero();
        }
        match self.style.placement {
            SlitPlacement::Left => Strut::new(self.thickness, 0, 0, 0),
            SlitPlacement::Right => Strut::new(0, self.thickness, 0, 0),
            SlitPlacement::Top => Strut::new(0, 0, self.thickness, 0),
            SlitPlacement::Bottom => Strut::new(0, 0, 0, self.thickness),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.dock.is_empty()
    }

    /// Reparent a dock window into the slit and lay it out.
    pub fn add_window(&mut self, conn: &X11Connection, window: u32) -> Result<(), anyhow::Error> {
        if self.owns_window(window) {
            return Ok(());
        }

        let geom = conn.conn().get_geometry(window)?.reply()?;
        let (w, h) = (geom.width, geom.height);
        let gap = self.style.gap;

        let offset = self.length;
        self.dock.push((window, w, h));
        self.thickness = self.thickness.max(w);
        self.length = self.length.saturating_add(h).saturating_add(gap);

        // Place the dock window inside the slit, then resize/reposition the slit.
        conn.conn().reparent_window(window, self.window, 0, offset as i16)?;
        self.reposition(conn)?;

        // Make sure the dockapp is visible (it may have been unmapped).
        conn.conn().map_window(window)?;
        conn.conn().flush()?;
        Ok(())
    }

    pub fn remove_window(&mut self, conn: &X11Connection, window: u32) -> Result<(), anyhow::Error> {
        if let Some(pos) = self.dock.iter().position(|(w, _, _)| *w == window) {
            self.dock.remove(pos);
            self.relayout(conn)?;
        }
        Ok(())
    }

    /// Recompute thickness/length and reposition every docked window.
    fn relayout(&mut self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        let gap = self.style.gap;
        let mut thickness: u16 = 0;
        let mut length: u16 = 0;
        for (_w, dw, dh) in &self.dock {
            thickness = thickness.max(*dw);
            length = length.saturating_add(*dh).saturating_add(gap);
        }
        self.thickness = thickness;
        self.length = length;
        self.reposition(conn)?;
        Ok(())
    }

    fn reposition(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        if self.thickness == 0 || self.length == 0 {
            conn.conn().unmap_window(self.window)?;
            return Ok(());
        }

        let (x, y, w, h) = match self.style.placement {
            SlitPlacement::Right => (
                (self.screen_width as i16 - self.thickness as i16),
                0,
                self.thickness,
                self.length,
            ),
            SlitPlacement::Left => (0, 0, self.thickness, self.length),
            SlitPlacement::Top => (0, 0, self.length, self.thickness),
            SlitPlacement::Bottom => (
                0,
                (self.screen_height as i16 - self.thickness as i16),
                self.length,
                self.thickness,
            ),
        };

        conn.conn().configure_window(
            self.window,
            &xproto::ConfigureWindowAux::new()
                .x(x as i32)
                .y(y as i32)
                .width(w as u32)
                .height(h as u32)
                .stack_mode(xproto::StackMode::ABOVE),
        )?;
        conn.conn().map_window(self.window)?;

        // Restack dock windows from the top/left of the slit inward.
        let gap = self.style.gap;
        let mut off: i16 = 0;
        for (win, _dw, dh) in &self.dock {
            conn.conn().configure_window(
                *win,
                &xproto::ConfigureWindowAux::new()
                    .x(0)
                    .y(off as i32)
                    .stack_mode(xproto::StackMode::ABOVE),
            )?;
            off += *dh as i16 + gap as i16;
        }
        conn.conn().flush()?;
        Ok(())
    }

    pub fn handle_expose(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        if self.thickness == 0 || self.length == 0 {
            return Ok(());
        }
        conn.conn().change_gc(
            self.gc,
            &xproto::ChangeGCAux::new().foreground(self.bg_pixel),
        )?;
        conn.conn().poly_fill_rectangle(
            self.window,
            self.gc,
            &[xproto::Rectangle {
                x: 0,
                y: 0,
                width: self.thickness,
                height: self.length,
            }],
        )?;
        Ok(())
    }

    pub fn destroy(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        // Reparent dock apps back to root so they survive our shutdown.
        for (win, _, _) in &self.dock {
            let _ = conn
                .conn()
                .reparent_window(*win, conn.root_window(), 0, 0);
        }
        conn.conn().free_gc(self.gc)?;
        conn.conn().destroy_window(self.window)?;
        Ok(())
    }
}
