use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, EventMask, WindowClass, ConnectionExt as _};

use crate::core::{Rectangle, Strut};
use crate::render::font::Font;
use crate::x11::X11Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarPlacement {
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolbarAction {
    None,
    Workspace(usize),
    Window(usize),
}

pub struct ToolbarStyle {
    pub height: u16,
    pub bevel_width: u16,
    pub border_width: u16,
    pub placement: ToolbarPlacement,
    pub font: String,
}

impl Default for ToolbarStyle {
    fn default() -> Self {
        Self {
            height: 24,
            bevel_width: 2,
            border_width: 1,
            placement: ToolbarPlacement::Bottom,
            font: "fixed".to_string(),
        }
    }
}

/// A Fluxbox-style toolbar: a docked bar with workspace buttons on the left,
/// the focused window label in the center, and a clock on the right. Owned by
/// `BScreen`; it is an `override_redirect` window so it never participates in
/// normal window management.
pub struct FbToolbar {
    window: u32,
    gc: u32,
    font: Font,
    fg_pixel: u32,
    bg_pixel: u32,
    screen_width: u16,
    screen_height: u16,
    style: ToolbarStyle,
    workspace_names: Vec<String>,
    current_workspace: u32,
    label: String,
    button_rects: Vec<(usize, Rectangle)>,
    label_rect: Rectangle,
    clock_rect: Rectangle,
    window_items: Vec<(String, bool)>,
    window_rects: Vec<Rectangle>,
}

impl FbToolbar {
    pub fn new(
        conn: &X11Connection,
        screen_width: u16,
        screen_height: u16,
        workspace_names: Vec<String>,
        placement: ToolbarPlacement,
    ) -> Result<Self, anyhow::Error> {
        let style = ToolbarStyle {
            placement,
            ..ToolbarStyle::default()
        };

        let window = conn.conn().generate_id()?;
        conn.conn().create_window(
            0,
            window,
            conn.root_window(),
            0,
            0,
            screen_width,
            style.height,
            style.border_width,
            WindowClass::INPUT_OUTPUT,
            0,
            &xproto::CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(conn.screen().white_pixel)
                .event_mask(
                    EventMask::EXPOSURE | EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                ),
        )?;

        let gc = conn.conn().generate_id()?;
        conn.conn().create_gc(
            gc,
            window,
            &xproto::CreateGCAux::new().foreground(conn.screen().black_pixel),
        )?;

        let font = Font::load_x11_font(conn.conn(), &style.font)
            .unwrap_or_else(|_| Font::new(&style.font));
        log::debug!("toolbar font x_id present: {}", font.x_id().is_some());

        let mut tb = Self {
            window,
            gc,
            font,
            fg_pixel: conn.screen().black_pixel,
            bg_pixel: conn.screen().white_pixel,
            screen_width,
            screen_height,
            style,
            workspace_names,
            current_workspace: 0,
            label: String::new(),
            button_rects: Vec::new(),
            label_rect: Rectangle::zero(),
            clock_rect: Rectangle::zero(),
            window_items: Vec::new(),
            window_rects: Vec::new(),
        };
        tb.layout();
        Ok(tb)
    }

    pub fn window_id(&self) -> u32 {
        self.window
    }

    pub fn show(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        conn.conn().map_window(self.window)?;
        conn.conn().configure_window(
            self.window,
            &xproto::ConfigureWindowAux::new().stack_mode(xproto::StackMode::ABOVE),
        )?;
        Ok(())
    }

    /// Vertical space the toolbar reserves on its edge (used to build a strut
    /// so managed clients never overlap it).
    pub fn strut(&self) -> Strut {
        let h = self.style.height + self.style.border_width * 2;
        match self.style.placement {
            ToolbarPlacement::Top => Strut::new(0, 0, h, 0),
            ToolbarPlacement::Bottom => Strut::new(0, 0, 0, h),
        }
    }

    pub fn set_current_workspace(&mut self, ws: u32) {
        self.current_workspace = ws;
    }

    pub fn set_label(&mut self, label: String) {
        self.label = label;
    }

    /// Replace the running-window list. Each entry is `(name, focused)`.
    pub fn set_window_items(&mut self, items: Vec<(String, bool)>) {
        self.window_items = items;
    }

    fn placement_y(&self) -> i16 {
        match self.style.placement {
            ToolbarPlacement::Top => 0,
            ToolbarPlacement::Bottom => {
                self.screen_height as i16 - (self.style.height + self.style.border_width * 2) as i16
            }
        }
    }

    /// Recompute the geometry and the clickable regions. Cheap (no X calls).
    /// All rectangles are in *window-local* coordinates (the window origin is
    /// its top-left border), NOT screen coordinates.
    fn layout(&mut self) {
        let h = self.style.height;
        let bw = self.style.border_width as i16;
        let y = bw;

        let btn_w: i16 = 24;
        let clock_w: i16 = 144;
        let gap: i16 = 2;

        let mut x = bw + gap;
        self.button_rects.clear();
        for (i, _name) in self.workspace_names.iter().enumerate() {
            self.button_rects.push((
                i,
                Rectangle::new(x, y, btn_w as u16, h),
            ));
            x += btn_w + gap;
        }

        let clock_x = (self.screen_width as i16).saturating_sub(clock_w + gap + bw);
        let list_x = x;
        let list_w = (clock_x - gap - list_x).max(0) as i16;

        self.window_rects.clear();
        if !self.window_items.is_empty() {
            let n = self.window_items.len() as i16;
            let sep = gap;
            let item_w = if n > 0 {
                ((list_w - sep * (n - 1)) / n).max(24)
            } else {
                0
            };
            let mut ix = list_x;
            for _ in &self.window_items {
                self.window_rects.push(Rectangle::new(ix, y, item_w.max(0) as u16, h));
                ix += item_w + sep;
            }
        }

        self.label_rect = Rectangle::new(list_x, y, list_w.max(0) as u16, h);
        self.clock_rect = Rectangle::new(clock_x, y, clock_w.max(0) as u16, h);
    }

    pub fn render(&mut self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        self.layout();

        // Keep the window glued to its edge if the screen size changed.
        conn.conn().configure_window(
            self.window,
            &xproto::ConfigureWindowAux::new()
                .x(0)
                .y(self.placement_y() as i32)
                .width(self.screen_width as u32)
                .height((self.style.height + self.style.border_width * 2) as u32),
        )?;

        let white = self.bg_pixel;
        let black = self.fg_pixel;

        // Clear the entire toolbar background so stale pixels from a previous
        // render (e.g. old clock digits, old window-list entries) don't bleed
        // through the new content.
        conn.conn().change_gc(
            self.gc,
            &xproto::ChangeGCAux::new().foreground(white),
        )?;
        let full_h = self.style.height + self.style.border_width * 2;
        conn.conn().poly_fill_rectangle(
            self.window,
            self.gc,
            &[xproto::Rectangle {
                x: 0,
                y: 0,
                width: self.screen_width,
                height: full_h,
            }],
        )?;

        // Workspace buttons: sunken (highlighted) for the active one.
        for (i, rect) in &self.button_rects {
            let current = *i as u32 == self.current_workspace;
            conn.conn().change_gc(
                self.gc,
                &xproto::ChangeGCAux::new().foreground(if current { black } else { white }),
            )?;
            conn.conn().poly_fill_rectangle(self.window, self.gc, &[xproto::Rectangle {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            }])?;
            crate::render::texture::TextureRender::render_bevel(
                conn,
                self.window,
                self.gc,
                rect,
                self.style.bevel_width,
                !current,
                white,
                black,
            )?;

            let text = format!("{}", i + 1);
            self.draw_text(conn, rect, &text, crate::core::Align::Center, if current { white } else { black })?;
        }

        // Running-window list in the centre (the "taskbar").
        if !self.window_items.is_empty() {
            for (i, (name, focused)) in self.window_items.iter().enumerate() {
                let rect = match self.window_rects.get(i) {
                    Some(r) => *r,
                    None => continue,
                };
                conn.conn().change_gc(
                    self.gc,
                    &xproto::ChangeGCAux::new()
                        .foreground(if *focused { black } else { white }),
                )?;
                conn.conn().poly_fill_rectangle(self.window, self.gc, &[xproto::Rectangle {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                }])?;
                crate::render::texture::TextureRender::render_bevel(
                    conn,
                    self.window,
                    self.gc,
                    &rect,
                    self.style.bevel_width,
                    !*focused,
                    white,
                    black,
                )?;

                // Clip the label to the button width.
                let avail = (rect.width as i16 - 6).max(4);
                let mut disp = name.clone();
                while !disp.is_empty()
                    && self.font.text_width(conn.conn(), &disp).unwrap_or(0) as i16 > avail
                {
                    disp.pop();
                }
                if disp.len() < name.len() {
                    disp.push('…');
                }
                let ty = rect.y
                    + (rect.height as i16 + self.font.height() as i16) / 2
                    - self.font.descent() as i16;
                conn.conn().change_gc(
                    self.gc,
                    &xproto::ChangeGCAux::new()
                        .foreground(if *focused { white } else { black }),
                )?;
                self.font.draw_text(conn.conn(), self.window, self.gc, rect.x + 3, ty, &disp)?;
            }
        } else if !self.label.is_empty() {
            self.draw_text(conn, &self.label_rect, &self.label, crate::core::Align::Left, black)?;
        }

        // Clock on the right.
        let clock = current_clock();
        self.draw_text(conn, &self.clock_rect, &clock, crate::core::Align::Right, black)?;

        Ok(())
    }

    pub fn handle_expose(&mut self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        self.render(conn)
    }

    pub fn handle_button_press(&self, x: i16, y: i16) -> ToolbarAction {
        for (i, rect) in &self.button_rects {
            if rect.contains(x, y) {
                return ToolbarAction::Workspace(*i);
            }
        }
        for (i, rect) in self.window_rects.iter().enumerate() {
            if rect.contains(x, y) {
                return ToolbarAction::Window(i);
            }
        }
        ToolbarAction::None
    }

    fn draw_text(
        &self,
        conn: &X11Connection,
        rect: &Rectangle,
        text: &str,
        align: crate::core::Align,
        fg: u32,
    ) -> Result<(), anyhow::Error> {
        let font = &self.font;
        let tw = font.text_width(conn.conn(), text).unwrap_or(0) as i16;
        let fh = font.height() as i16;

        let (tx, _) = match align {
            crate::core::Align::Left => (rect.x + 4, 0),
            crate::core::Align::Right => (rect.right() - tw - 4, 0),
            crate::core::Align::Center => {
                let cx = rect.x + (rect.width as i16 - tw) / 2;
                (cx, 0)
            }
            _ => (rect.x + 4, 0),
        };
        let ty = rect.y + (rect.height as i16 + fh) / 2 - font.descent() as i16;

        log::debug!("draw_text '{}' at ({},{}) fg={} x_id={:?}", text, tx, ty, fg, font.x_id());

        conn.conn().change_gc(
            self.gc,
            &xproto::ChangeGCAux::new()
                .foreground(fg),
        )?;
        font.draw_text(conn.conn(), self.window, self.gc, tx, ty, text)?;
        Ok(())
    }

    pub fn destroy(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        conn.conn().free_gc(self.gc)?;
        conn.conn().destroy_window(self.window)?;
        Ok(())
    }
}

/// Format the current UTC time as HH:MM:SS without any external dependency.
fn current_clock() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}
