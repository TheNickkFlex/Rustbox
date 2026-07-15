use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, EventMask, WindowClass, ConnectionExt as _};

use crate::battery::{read_battery, BatteryState};
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

/// A Rustbox-style toolbar: a docked bar with workspace buttons on the left,
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
    /// Width (px) reserved on the right of the toolbar for the system tray,
    /// which sits immediately to the left of the clock. Updated by `BScreen`.
    tray_reserve: i16,
    /// Battery indicator segment (between the tray and the clock). Width is 0
    /// when no battery is present (e.g. a desktop), in which case nothing is
    /// drawn and no space is reserved.
    battery_width: i16,
    battery_rect: Rectangle,
    battery_state: Option<(u8, BatteryState)>,
    last_battery_check: std::time::Instant,
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

        let y = match placement {
            ToolbarPlacement::Top => 0,
            ToolbarPlacement::Bottom => screen_height.saturating_sub(style.height) as i16,
        };

        let window = conn.conn().generate_id()?;
        conn.conn().create_window(
            0,
            window,
            conn.root_window(),
            0,
            y,
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

        let font = Font::new(&style.font);
        log::debug!("toolbar font x_id present: {}", font.x_id().is_some());

        // Detect whether a battery exists so we only reserve space / draw the
        // indicator on laptops. On desktops `read_battery` returns `None` and we
        // simply skip it. Uses the kernel sysfs interface, so it also works on
        // Android/Termux (Bionic) where glibc-oriented crates fail.
        let init_battery = read_battery();
        let (battery_state, battery_width) = match init_battery {
            Some(s) => (Some(s), 74),
            None => (None, 0),
        };
        log::info!("Toolbar battery init: width={}, state={:?}", battery_width, battery_state);

        let toolbar_h = (style.height + style.border_width * 2) as u16;

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
            tray_reserve: 0,
            battery_width,
            battery_rect: Rectangle::zero(),
            battery_state,
            last_battery_check: std::time::Instant::now(),
        };
        tb.layout();

        if let Some(cb) = crate::hooks::AFTER_TOOLBAR_CREATE.get() {
            cb(conn, window, screen_width, toolbar_h);
        }

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

    /// Re-raise the toolbar above all other windows so it never gets covered
    /// by a maximized window.
    pub fn raise(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
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

    pub fn set_workspace_names(&mut self, names: Vec<String>) {
        self.workspace_names = names;
    }

    /// Width the system tray currently occupies on the right, next to the
    /// clock. The toolbar stops its window-list before this region so the
    /// tray icons never overlap a window button.
    pub fn set_tray_reserve(&mut self, width: i16) {
        self.tray_reserve = width.max(0);
    }

    /// Screen-x of the tray's right edge (i.e. the left edge of the clock
    /// minus a gap). The tray anchors itself so its right edge lands here.
    /// Accounts for the battery segment that sits between the tray and clock.
    pub fn tray_right_anchor(&self) -> i16 {
        let bw = self.style.border_width as i16;
        let clock_w: i16 = 144;
        let gap: i16 = 2;
        (self.screen_width as i16) - clock_w - self.battery_width - 3 * gap - bw
    }

    pub fn set_label(&mut self, label: String) {
        self.label = label;
    }

    /// Poll the battery (throttled to ~15 s) and cache `(percent, state)`.
    /// When no battery was detected at init (`battery_width == 0`) we still
    /// retry periodically — a laptop battery might appear later, or a
    /// platform-specific reader (e.g. termux-battery-status) may only be
    /// reachable from inside the event loop.
    pub fn refresh_battery(&mut self) {
        let now = std::time::Instant::now();
        if now.duration_since(self.last_battery_check).as_secs() < 15 {
            return;
        }
        self.last_battery_check = now;
        if let Some(s) = read_battery() {
            if self.battery_width == 0 {
                log::info!("Battery detected on refresh, enabling indicator");
                self.battery_width = 74;
            }
            self.battery_state = Some(s);
        }
    }

    /// Replace the running-window list. Each entry is `(name, focused)`.
    pub fn set_window_items(&mut self, items: Vec<(String, bool)>) {
        self.window_items = items;
    }

    pub fn placement_y(&self) -> i16 {
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
        // Reserve room for the tray (to the left of the clock) so window
        // buttons never grow under the tray icons.
        let list_right = (self.tray_right_anchor() - self.tray_reserve - gap).max(list_x);
        let list_w = (list_right - list_x).max(0) as i16;

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

        // Battery segment sits immediately to the left of the clock (only when
        // a battery is present).
        if self.battery_width > 0 {
            let bx = clock_x - self.battery_width - gap;
            self.battery_rect = Rectangle::new(bx, y, self.battery_width as u16, h);
        } else {
            self.battery_rect = Rectangle::zero();
        }
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

        let white = crate::hooks::or(&crate::hooks::TOOLBAR_BG, self.bg_pixel);
        let black = crate::hooks::or(&crate::hooks::TOOLBAR_FG, self.fg_pixel);

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
            let (fg, bg) = if current { (white, black) } else { (black, white) };
            self.draw_text(conn, rect, &text, crate::core::Align::Center, fg, bg)?;
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

                let avail = (rect.width as i16 - 6).max(4);
                let disp = self.font.truncate_ellipsis(name, avail);
                let ty = rect.y
                    + (rect.height as i16 + self.font.height() as i16) / 2
                    - self.font.descent() as i16;
                let (fg, bg) = if *focused { (white, black) } else { (black, white) };
                self.font.draw_text_on_bg(conn.conn(), self.window, self.gc, rect.x + 3, ty, &disp, fg, bg)?;
            }
        } else if !self.label.is_empty() {
            self.draw_text(conn, &self.label_rect, &self.label, crate::core::Align::Left, black, white)?;
        }

        // Clock on the right.
        let clock = current_clock();
        self.draw_text(conn, &self.clock_rect, &clock, crate::core::Align::Right, black, white)?;

        // Battery indicator (only when a battery is present).
        if self.battery_width > 0 {
            if let Some(state) = self.battery_state {
                self.draw_battery(conn, &self.battery_rect, state, black, white)?;
            }
        }

        Ok(())
    }

    /// Draw a small battery glyph (outline + proportional fill + nub) followed
    /// by the percentage text. `state` is `(percent, BatteryState)`.
    fn draw_battery(
        &self,
        conn: &X11Connection,
        rect: &Rectangle,
        state: (u8, BatteryState),
        fg: u32,
        bg: u32,
    ) -> Result<(), anyhow::Error> {
        let (pct, st) = state;
        let h = self.style.height;
        let ix = rect.x + 2;
        let iw: i16 = 18;
        let ih: i16 = 9;
        let iy = rect.y + ((h as i16 - ih) / 2);
        let pct = pct.min(100) as f32;

        // Outline (black border) + white interior.
        conn.conn().change_gc(self.gc, &xproto::ChangeGCAux::new().foreground(fg))?;
        conn.conn().poly_fill_rectangle(
            self.window,
            self.gc,
            &[xproto::Rectangle {
                x: ix - 1,
                y: iy - 1,
                width: (iw + 2) as u16,
                height: (ih + 2) as u16,
            }],
        )?;
        conn.conn().change_gc(self.gc, &xproto::ChangeGCAux::new().foreground(bg))?;
        conn.conn().poly_fill_rectangle(
            self.window,
            self.gc,
            &[xproto::Rectangle {
                x: ix,
                y: iy,
                width: iw as u16,
                height: ih as u16,
            }],
        )?;

        // Charge fill (black), width proportional to percentage.
        let fill_w = ((iw as f32 * pct / 100.0).round() as i16).max(if pct > 0.0 { 1 } else { 0 });
        conn.conn().change_gc(self.gc, &xproto::ChangeGCAux::new().foreground(fg))?;
        conn.conn().poly_fill_rectangle(
            self.window,
            self.gc,
            &[xproto::Rectangle {
                x: ix,
                y: iy,
                width: fill_w.max(0) as u16,
                height: ih as u16,
            }],
        )?;

        // Nub on the right.
        conn.conn().poly_fill_rectangle(
            self.window,
            self.gc,
            &[xproto::Rectangle {
                x: ix + iw,
                y: iy + 2,
                width: 2,
                height: (ih - 4).max(1) as u16,
            }],
        )?;

        // Percentage text (with a '+' when charging, "FULL" when full).
        let text = match st {
            BatteryState::Charging => format!("{}+", pct as u8),
            BatteryState::Full => "FULL".to_string(),
            _ => format!("{}%", pct as u8),
        };
        let tx = ix + iw + 4;
        let ty = rect.y + ((h as i16 + self.font.height() as i16) / 2) - self.font.descent() as i16;
        self.font.draw_text_on_bg(conn.conn(), self.window, self.gc, tx, ty, &text, fg, bg)?;

        Ok(())
    }

    /// Update the stored screen size and refit the toolbar to it. Called when
    /// the root window geometry changes (RandR resize).
    pub fn reconfigure(&mut self, conn: &X11Connection, width: u16, height: u16) -> Result<(), anyhow::Error> {
        self.screen_width = width;
        self.screen_height = height;
        self.render(conn)
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
        bg: u32,
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

        log::debug!("draw_text '{}' at ({},{}) fg={} bg={}", text, tx, ty, fg, bg);

        font.draw_text_on_bg(conn.conn(), self.window, self.gc, tx, ty, text, fg, bg)?;
        Ok(())
    }

    pub fn destroy(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        conn.conn().free_gc(self.gc)?;
        conn.conn().destroy_window(self.window)?;
        Ok(())
    }
}

/// Format the current local time as HH:MM:SS using the system timezone.
fn current_clock() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as libc::time_t;

    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}
