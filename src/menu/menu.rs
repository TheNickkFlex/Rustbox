use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt as _};

use crate::core::Rectangle;
use crate::render::font::Font;
use crate::x11::X11Connection;

use super::menuitem::{MenuItem, MenuItemType};

/// The height in pixels of the menu title bar.
const TITLE_HEIGHT: u16 = 22;
/// The height in pixels of each item row.
const ITEM_HEIGHT: u16 = 20;
/// Minimum menu width (includes text + bevels).
const MIN_WIDTH: u16 = 120;
/// Bevel inset around item text.
const BEVEL: i16 = 4;
/// Gap between columns.
const COL_GAP: i16 = 2;

/// A single popup menu window (override‑redirect).  One instance per visible
/// menu (including submenus).  Owns its X11 window and the off‑screen pixmap
/// used for flicker‑free highlighting.
pub struct Menu {
    /// X11 window ID (override‑redirect, save‑under).
    window: u32,
    /// Title label shown at the top.
    title: String,
    /// Items in display order.
    items: Vec<MenuItem>,
    /// For each item the ordered type‑resolved variants (separate from
    /// MenuItem so the parser can stay lightweight).
    item_types: Vec<MenuItemType>,

    // ---- geometry (in window‑local coordinates) ----
    width: u16,
    height: u16,
    /// Width of a single grid cell.
    item_w: u16,
    /// Number of display columns.
    columns: u16,
    /// Rows per column.
    rows_per_col: u16,

    // ---- interaction state ----
    active_index: Option<usize>,
    /// Which item has an open submenu (index into `items`).
    which_sub: Option<usize>,
    /// The child submenu window, if open.
    submenu: Option<Box<Menu>>,

    /// Screen position of this menu's window.
    screen_x: i16,
    screen_y: i16,

    // ---- derived for hit‑testing ----
    item_rects: Vec<Rectangle>,
    /// The title bar rectangle.
    title_rect: Rectangle,
    /// The frame (items area) rectangle.
    frame_rect: Rectangle,

    // ---- screen dimensions for clamping ----
    screen_width: u16,
    screen_height: u16,

    // ---- X resources ----
    font: Font,
    gc: u32,
    fg_pixel: u32,
    bg_pixel: u32,
    sel_pixel: u32,
    hi_pixel: u32,
}

impl Menu {
    /// Build a new menu window.  Does **not** show it (call `position_and_show`).
    pub fn new(
        conn: &X11Connection,
        title: &str,
        items: Vec<MenuItem>,
        screen_width: u16,
        screen_height: u16,
    ) -> Result<Self, anyhow::Error> {
        let screen = conn.screen();
        let fg = screen.black_pixel;
        let bg = screen.white_pixel;

        // Create override‑redirect window (initially 1×1 — sized in layout).
        let win = conn.conn().generate_id()?;
        conn.conn().create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
            conn.root_window(),
            0,
            0,
            1,
            1,
            0,
            xproto::WindowClass::INPUT_OUTPUT,
            0,
            &xproto::CreateWindowAux::new()
                .override_redirect(1u32)
                .background_pixel(bg)
                .event_mask(
                    xproto::EventMask::EXPOSURE
                        | xproto::EventMask::BUTTON_PRESS
                        | xproto::EventMask::BUTTON_RELEASE
                        | xproto::EventMask::POINTER_MOTION
                        | xproto::EventMask::ENTER_WINDOW
                        | xproto::EventMask::LEAVE_WINDOW,
                ),
        )?;

        let gc = conn.conn().generate_id()?;
        conn.conn().create_gc(gc, win, &xproto::CreateGCAux::new().foreground(fg))?;

        let font = Font::new("fixed");

        let item_types: Vec<MenuItemType> = items.iter().map(|i| i.item_type().clone()).collect();

        let mut m = Self {
            window: win,
            title: title.to_string(),
            items,
            item_types,
            width: 0,
            height: 0,
            item_w: 0,
            columns: 1,
            rows_per_col: 1,
            active_index: None,
            which_sub: None,
            submenu: None,
            screen_x: 0,
            screen_y: 0,
            item_rects: Vec::new(),
            title_rect: Rectangle::default(),
            frame_rect: Rectangle::default(),
            screen_width,
            screen_height,
            font,
            gc,
            fg_pixel: fg,
            bg_pixel: bg,
            sel_pixel: fg,             // selected/highlight background = fg
            hi_pixel: bg,              // highlight text colour = bg
        };

        m.update_layout(conn)?;
        Ok(m)
    }

    pub fn window_id(&self) -> u32 {
        self.window
    }

    pub fn screen_x(&self) -> i16 {
        self.screen_x
    }

    pub fn screen_y(&self) -> i16 {
        self.screen_y
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn items(&self) -> &[MenuItem] {
        &self.items
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active_index
    }

    /// Recursively find the (sub)menu whose window is `win`.
    pub fn find_menu_by_window(&self, win: u32) -> Option<&Menu> {
        if self.window == win {
            return Some(self);
        }
        self.submenu.as_ref().and_then(|s| s.find_menu_by_window(win))
    }

    /// Mut‑version of `find_menu_by_window`.
    pub fn find_menu_by_window_mut(&mut self, win: u32) -> Option<&mut Menu> {
        if self.window == win {
            return Some(self);
        }
        self.submenu.as_mut().and_then(|s| s.find_menu_by_window_mut(win))
    }

    // ───────────────────────────────────────────────
    //  Layout
    // ───────────────────────────────────────────────

    /// Recompute item cell geometry, window dimensions, and clickable rects.
    /// Called once at creation and on every reconfigure.
    fn update_layout(&mut self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        let n = self.items.len() as u16;

        // Measure each item's text width.
        let mut max_w: u16 = 32;
        for item in &self.items {
            let tw = self.font.text_width(conn.conn(), item.label()).unwrap_or(0);
            max_w = max_w.max(tw + (BEVEL as u16) * 2 + 8);
        }
        // Include the title in the width calculation.
        let tw = self.font.text_width(conn.conn(), &self.title).unwrap_or(0);
        max_w = max_w.max(tw + (BEVEL as u16) * 2 + 8);

        // Submenu items get extra space for the bullet arrow.
        let has_sub = self.items.iter().any(|i| matches!(i.item_type(), MenuItemType::Submenu(..)));
        if has_sub {
            max_w += 12;
        }

        self.item_w = max_w.max(MIN_WIDTH);
        self.columns = 1;
        self.rows_per_col = n;

        // Try multi‑column if the menu would exceed screen height.
        let max_visible_h = self.screen_height.saturating_sub(20);
        let total_item_h = n * ITEM_HEIGHT;
        if total_item_h > max_visible_h && TITLE_HEIGHT + ITEM_HEIGHT > 0 {
            // Compute minimal columns so height fits the screen.
            let ideal_cols = ((total_item_h as f32) / (max_visible_h.saturating_sub(TITLE_HEIGHT) as f32)).ceil() as u16;
            self.columns = ideal_cols.max(1).min(n);
            self.rows_per_col = (n + self.columns - 1) / self.columns;
        }

        self.width = self.columns as u16 * self.item_w + (self.columns.saturating_sub(1) as u16) * (COL_GAP as u16);
        self.height = TITLE_HEIGHT + self.rows_per_col * ITEM_HEIGHT;

        // Compute rectangles.
        self.title_rect = Rectangle::new(0, 0, self.width, TITLE_HEIGHT);
        self.frame_rect = Rectangle::new(0, TITLE_HEIGHT as i16, self.width, self.rows_per_col * ITEM_HEIGHT);

        self.item_rects.clear();
        for i in 0..n as usize {
            let col = i as u16 / self.rows_per_col;
            let row = i as u16 % self.rows_per_col;
            let x = col as i16 * (self.item_w as i16 + COL_GAP);
            let y = TITLE_HEIGHT as i16 + row as i16 * ITEM_HEIGHT as i16;
            self.item_rects.push(Rectangle::new(x, y, self.item_w, ITEM_HEIGHT));
        }

        // Resize the X window to match the computed geometry.
        conn.conn().configure_window(
            self.window,
            &xproto::ConfigureWindowAux::new()
                .width(self.width as u32)
                .height(self.height as u32),
        )?;

        Ok(())
    }

    // ───────────────────────────────────────────────
    //  Rendering
    // ───────────────────────────────────────────────

    /// Full redraw of the menu window.
    pub fn render(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        // Title bar background.
        conn.conn().change_gc(
            self.gc,
            &xproto::ChangeGCAux::new().foreground(self.sel_pixel),
        )?;
        conn.conn().poly_fill_rectangle(
            self.window,
            self.gc,
            &[xproto::Rectangle {
                x: self.title_rect.x,
                y: self.title_rect.y,
                width: self.title_rect.width,
                height: self.title_rect.height,
            }],
        )?;

        // Title text (white on dark background).
        let ty = (TITLE_HEIGHT as i16 + self.font.ascent() as i16 - self.font.descent() as i16) / 2;
        self.font.draw_text_on_bg(
            conn.conn(), self.window, self.gc, BEVEL, ty, &self.title,
            self.hi_pixel, self.sel_pixel,
        )?;

        // Frame background (items area).
        conn.conn().change_gc(
            self.gc,
            &xproto::ChangeGCAux::new().foreground(self.bg_pixel),
        )?;
        conn.conn().poly_fill_rectangle(
            self.window,
            self.gc,
            &[xproto::Rectangle {
                x: self.frame_rect.x,
                y: self.frame_rect.y,
                width: self.frame_rect.width,
                height: self.frame_rect.height,
            }],
        )?;

        // Draw each item.
        for i in 0..self.items.len() {
            self.draw_item(conn, i, i == self.active_index.unwrap_or(usize::MAX))?;
        }

        conn.conn().flush()?;
        Ok(())
    }

    /// Draw a single menu item at its grid cell.
    fn draw_item(&self, conn: &X11Connection, index: usize, highlight: bool) -> Result<(), anyhow::Error> {
        let rect = match self.item_rects.get(index) {
            Some(r) => *r,
            None => return Ok(()),
        };
        let item = &self.items[index];

        // Background.
        let bg = if highlight { self.sel_pixel } else { self.bg_pixel };
        conn.conn().change_gc(
            self.gc,
            &xproto::ChangeGCAux::new().foreground(bg),
        )?;
        conn.conn().poly_fill_rectangle(
            self.window,
            self.gc,
            &[xproto::Rectangle {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
            }],
        )?;

        match item.item_type() {
            MenuItemType::Separator => {
                // Draw a horizontal line through the middle of the cell.
                let mid_y = rect.y + (rect.height as i16) / 2;
                conn.conn().change_gc(
                    self.gc,
                    &xproto::ChangeGCAux::new().foreground(self.fg_pixel),
                )?;
                conn.conn().poly_segment(
                    self.window,
                    self.gc,
                    &[xproto::Segment {
                        x1: rect.x + BEVEL,
                        y1: mid_y,
                        x2: rect.right() - BEVEL,
                        y2: mid_y,
                    }],
                )?;
            }
            MenuItemType::Submenu(_id, _) => {
                // Text.
                let fg = if highlight { self.hi_pixel } else { self.fg_pixel };
                let bg = if highlight { self.sel_pixel } else { self.bg_pixel };
                let ty = rect.y + (rect.height as i16 + self.font.ascent() as i16 - self.font.descent() as i16) / 2;
                self.font.draw_text_on_bg(
                    conn.conn(), self.window, self.gc, rect.x + BEVEL, ty,
                    item.label(), fg, bg,
                )?;

                // Submenu arrow (">") on the right side.
                let cx = rect.right() - 10;
                self.font.draw_text_on_bg(
                    conn.conn(), self.window, self.gc, cx, ty, ">", fg, bg,
                )?;
            }
            _ => {
                // Regular item.
                let bg = if highlight { self.sel_pixel } else { self.bg_pixel };
                let fg = if highlight || !item.is_enabled() {
                    if highlight {
                        self.hi_pixel
                    } else {
                        // Dimmed text for disabled items.
                        self.bg_pixel
                    }
                } else {
                    self.fg_pixel
                };
                let ty = rect.y + (rect.height as i16 + self.font.ascent() as i16 - self.font.descent() as i16) / 2;
                self.font.draw_text_on_bg(
                    conn.conn(), self.window, self.gc, rect.x + BEVEL, ty,
                    item.label(), fg, bg,
                )?;
            }
        }

        Ok(())
    }

    // ───────────────────────────────────────────────
    //  Positioning & Show
    // ───────────────────────────────────────────────

    /// Show the menu at the given screen coordinates, clamped to stay on‑screen.
    pub fn position_and_show(&mut self, conn: &X11Connection, mut x: i16, mut y: i16) -> Result<(), anyhow::Error> {
        // Clamp to screen.
        let mw = self.width as i16;
        let mh = self.height as i16;
        let sw = self.screen_width as i16;
        let sh = self.screen_height as i16;

        if x + mw > sw {
            x = (sw - mw).max(0);
        }
        if y + mh > sh {
            y = (sh - mh).max(0);
        }
        if x < 0 {
            x = 0;
        }
        if y < 0 {
            y = 0;
        }

        conn.conn().configure_window(
            self.window,
            &xproto::ConfigureWindowAux::new().x(x as i32).y(y as i32),
        )?;
        self.screen_x = x;
        self.screen_y = y;
        // Map first so the window is visible, then render the content.
        // Rendering before map_window causes the drawn content to be lost on
        // servers without backing store (e.g. termux-x11).
        conn.conn().map_window(self.window)?;
        self.render(conn)?;
        conn.conn().flush()?;
        Ok(())
    }

    pub fn hide(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        conn.conn().unmap_window(self.window)?;
        conn.conn().flush()?;
        Ok(())
    }

    // ───────────────────────────────────────────────
    //  Hit‑testing
    // ───────────────────────────────────────────────

    /// Find which item (if any) is at the given **window‑local** coordinates.
    pub fn hit_test(&self, x: i16, y: i16) -> Option<usize> {
        if y < TITLE_HEIGHT as i16 {
            return None;
        }
        for (i, rect) in self.item_rects.iter().enumerate() {
            if rect.contains(x, y) {
                return Some(i);
            }
        }
        None
    }

    // ───────────────────────────────────────────────
    //  Event handlers
    // ───────────────────────────────────────────────

    pub fn handle_motion(&mut self, conn: &X11Connection, x: i16, y: i16) -> Result<(), anyhow::Error> {
        let new_active = self.hit_test(x, y);
        if new_active != self.active_index {
            if let Some(old) = self.active_index {
                self.draw_item(conn, old, false)?;
            }
            if let Some(idx) = new_active {
                self.draw_item(conn, idx, true)?;
            }
            self.active_index = new_active;
            conn.conn().flush()?;
        }
        Ok(())
    }

    /// Returns the `MenuItemType` to execute, or `None` if no action.
    pub fn handle_click(&mut self, _conn: &X11Connection, x: i16, y: i16) -> Result<Option<MenuItemType>, anyhow::Error> {
        if let Some(idx) = self.hit_test(x, y) {
            if !self.items[idx].is_enabled() {
                return Ok(None);
            }
            // For submenu items, we return the type but the caller opens the submenu.
            // For command items, we execute and close.
            let tp = self.item_types[idx].clone();
            if matches!(tp, MenuItemType::Submenu(..)) {
                // Open submenu.
                return Ok(Some(tp));
            }
            return Ok(Some(tp));
        }
        Ok(None)
    }

    /// Set an already‑resolved submenu as the currently open one.
    pub fn set_submenu(&mut self, sub: Option<Box<Menu>>) {
        self.submenu = sub;
    }

    pub fn take_submenu(&mut self) -> Option<Box<Menu>> {
        self.submenu.take()
    }

    pub fn submenu(&self) -> Option<&Menu> {
        self.submenu.as_deref()
    }

    pub fn submenu_mut(&mut self) -> Option<&mut Menu> {
        self.submenu.as_deref_mut()
    }

    /// Walk the submenu chain to find the deepest open submenu.
    pub fn deepest_submenu(&self) -> &Menu {
        let mut current = self;
        while let Some(sub) = current.submenu() {
            current = sub;
        }
        current
    }

    /// Destroy the X11 window and free resources, recursively destroying any submenus.
    pub fn destroy(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        if let Some(sub) = &self.submenu {
            sub.destroy(conn)?;
        }
        conn.conn().destroy_window(self.window)?;
        conn.conn().free_gc(self.gc)?;
        Ok(())
    }
}
