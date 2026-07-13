use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, EventMask, WindowClass, ConnectionExt as _};

use crate::x11::X11Connection;

/// System tray icon entry (legacy XEmbed / _NET_SYSTEM_TRAY client).
struct TrayIcon {
    window: u32,
}

/// A StatusNotifierItem (SNI) entry — the modern D-Bus tray protocol used by
/// Discord, Telegram, Electron, etc. Unlike `TrayIcon` these are not XEmbed
/// windows; we render their ARGB32 icon into a pixmap ourselves.
struct SniSlot {
    service: String,
    pixmap: Option<u32>,
}

/// A freedesktop.org `_NET_SYSTEM_TRAY` manager.
///
/// Creates a container window which lives inside the toolbar area, just to the
/// left of the clock, claims the `_NET_SYSTEM_TRAY_S0` selection, and embeds
/// client tray icons via `_NET_SYSTEM_TRAY_OPCODE` ClientMessages and the
/// XEMBED protocol.
///
/// To avoid a crowded taskbar the tray shows at most `MAX_VISIBLE` icons in a
/// row. When more icons dock, a chevron ("⌄") button appears as the rightmost
/// slot; clicking it opens a popup window stacked above the tray that hosts
/// the overflow icons, exactly like the Windows notification area.
pub struct FbTray {
    window: u32,
    popup_window: u32,
    gc: u32,
    fg_pixel: u32,
    bg_pixel: u32,
    screen_width: u16,
    screen_height: u16,
    icon_size: u16,
    max_visible: usize,
    /// Screen-x of the tray's right edge (left edge of the clock minus gap).
    right_anchor: i16,
    icons: Vec<TrayIcon>,
    sni_slots: Vec<SniSlot>,
    popup_open: bool,
}

impl FbTray {
    pub fn new(
        conn: &X11Connection,
        screen_width: u16,
        screen_height: u16,
    ) -> Result<Self, anyhow::Error> {
        let icon_size = 24u16;
        let max_visible = 4usize;

        let window = conn.conn().generate_id()?;
        conn.conn().create_window(
            0,
            window,
            conn.root_window(),
            0, 0, 1, 1, 0,
            WindowClass::INPUT_OUTPUT,
            0,
            &xproto::CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(conn.screen().white_pixel)
                .event_mask(
                    EventMask::EXPOSURE
                        | EventMask::SUBSTRUCTURE_NOTIFY
                        | EventMask::SUBSTRUCTURE_REDIRECT
                        | EventMask::BUTTON_PRESS,
                ),
        )?;

        // Popup window that hosts overflow icons. A separate
        // override_redirect window stacked above the tray.
        let popup_window = conn.conn().generate_id()?;
        conn.conn().create_window(
            0,
            popup_window,
            conn.root_window(),
            0, 0, 1, 1, 0,
            WindowClass::INPUT_OUTPUT,
            0,
            &xproto::CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(conn.screen().white_pixel)
                .event_mask(EventMask::EXPOSURE | EventMask::BUTTON_PRESS),
        )?;

        let gc = conn.conn().generate_id()?;
        conn.conn().create_gc(
            gc,
            window,
            &xproto::CreateGCAux::new().foreground(conn.screen().black_pixel),
        )?;

        Ok(Self {
            window,
            popup_window,
            gc,
            fg_pixel: conn.screen().black_pixel,
            bg_pixel: conn.screen().white_pixel,
            screen_width,
            screen_height,
            icon_size,
            max_visible,
            right_anchor: 0,
            icons: Vec::new(),
            sni_slots: Vec::new(),
            popup_open: false,
        })
    }

    pub fn window_id(&self) -> u32 {
        self.window
    }

    pub fn popup_window_id(&self) -> u32 {
        self.popup_window
    }

    pub fn owns_window(&self, window: u32) -> bool {
        if window == self.window || window == self.popup_window {
            return true;
        }
        self.icons.iter().any(|icon| icon.window == window)
    }

    pub fn is_empty(&self) -> bool {
        self.icons.is_empty()
    }

    pub fn current_width(&self) -> i16 {
        let slots = self.total_slots();
        if slots == 0 {
            return 0;
        }
        slots as i16 * self.icon_size as i16
    }

    pub fn popup_open(&self) -> bool {
        self.popup_open
    }

    /// Set the screen-x where the tray's right edge should land (the left edge
    /// of the clock minus a gap). `BScreen` derives this from the toolbar.
    pub fn set_anchor(&mut self, anchor: i16) {
        self.right_anchor = anchor;
    }

    /// Number of XEmbed icons shown before SNI slots and (optionally) the
    /// chevron. Only SNI slots that have a pixmap contribute to the layout.
    fn visible_xembed(&self) -> usize {
        let room = self.max_visible.saturating_sub(self.sni_visible());
        self.icons.len().min(room)
    }

    /// XEmbed icons beyond `visible_xembed` (shown in the overflow popup).
    fn overflow_xembed(&self) -> usize {
        self.icons.len().saturating_sub(self.visible_xembed())
    }

    /// SNI slots that actually have a pixmap (visible in the tray).
    fn sni_visible(&self) -> usize {
        self.sni_slots.iter().filter(|s| s.pixmap.is_some()).count()
    }

    /// Total layout slots: visible XEmbed icons + visible SNI slots + chevron
    /// (if any XEmbed icon overflows).
    fn total_slots(&self) -> usize {
        self.visible_xembed() + self.sni_visible() + if self.overflow_xembed() > 0 { 1 } else { 0 }
    }

    /// Claim the `_NET_SYSTEM_TRAY_S0` selection and send a MANAGER ClientMessage
    /// to the root window so interested clients know we are the tray owner.
    pub fn claim_selection(
        &self,
        conn: &X11Connection,
        timestamp: u32,
    ) -> Result<(), anyhow::Error> {
        let sel_atom = conn.atoms().get(crate::x11::Atom::NetSystemTrayS0);
        let manager_atom = conn.atoms().get(crate::x11::Atom::Manager);
        if sel_atom == x11rb::NONE || manager_atom == x11rb::NONE {
            return Ok(());
        }

        conn.conn().set_selection_owner(self.window, sel_atom, timestamp)?;

        let ev = xproto::ClientMessageEvent::new(
            32,
            conn.root_window(),
            manager_atom,
            xproto::ClientMessageData::from([timestamp, sel_atom, self.window, 0, 0]),
        );
        let _ = conn.conn().send_event(
            false,
            conn.root_window(),
            EventMask::STRUCTURE_NOTIFY,
            &ev,
        );
        conn.conn().flush()?;
        log::info!("Claimed _NET_SYSTEM_TRAY_S0 selection (window={:#x})", self.window);
        Ok(())
    }

    /// Handle a `_NET_SYSTEM_TRAY_OPCODE` ClientMessage.
    pub fn handle_opcode(
        &mut self,
        conn: &X11Connection,
        opcode: u32,
        client_window: u32,
        timestamp: u32,
    ) -> Result<(), anyhow::Error> {
        match opcode {
            0 => self.dock_window(conn, client_window, timestamp),
            _ => {
                log::debug!("Unknown system tray opcode {}", opcode);
                Ok(())
            }
        }
    }

    /// Dock a client tray icon via XEMBED.
    fn dock_window(
        &mut self,
        conn: &X11Connection,
        client: u32,
        _timestamp: u32,
    ) -> Result<(), anyhow::Error> {
        if self.owns_window(client) {
            return Ok(());
        }

        log::info!("Docking tray client {:#x}", client);

        let _ = conn.conn().configure_window(
            client,
            &xproto::ConfigureWindowAux::new()
                .width(self.icon_size as u32)
                .height(self.icon_size as u32),
        );

        // Reparent into the tray container; `reposition()` will place it.
        conn.conn().reparent_window(client, self.window, 0, 0)?;
        conn.conn().map_window(client)?;

        let xembed = conn.atoms().get(crate::x11::Atom::Xembed);
        if xembed != x11rb::NONE {
            let ev = xproto::ClientMessageEvent::new(
                32,
                client,
                xembed,
                xproto::ClientMessageData::from([0, self.window, 0, 0, 0]),
            );
            let _ = conn.conn().send_event(
                false, client, EventMask::NO_EVENT, &ev,
            );
        }

        self.icons.push(TrayIcon { window: client });

        self.reposition(conn)?;
        conn.conn().flush()?;
        Ok(())
    }

    /// Undock a tray icon when its window is destroyed.
    pub fn undock_window(&mut self, conn: &X11Connection, window: u32) -> Result<(), anyhow::Error> {
        if let Some(pos) = self.icons.iter().position(|icon| icon.window == window) {
            self.icons.remove(pos);
            self.reposition(conn)?;
        }
        Ok(())
    }

    /// Toggle the overflow popup open/closed.
    pub fn toggle_popup(&mut self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        self.popup_open = !self.popup_open;
        if !self.popup_open {
            conn.conn().unmap_window(self.popup_window)?;
        }
        self.reposition(conn)?;
        Ok(())
    }

    /// Close the popup if it is open and `event_window` is neither the tray
    /// container nor the popup. Used to dismiss the popup on an outside click.
    pub fn maybe_close_popup(&mut self, conn: &X11Connection, event_window: u32) -> Result<(), anyhow::Error> {
        if self.popup_open && event_window != self.window && event_window != self.popup_window {
            self.popup_open = false;
            conn.conn().unmap_window(self.popup_window)?;
            self.reposition(conn)?;
        }
        Ok(())
    }

    /// Check whether a click (in tray-local coordinates) landed on the
    /// chevron slot; if so the caller toggles the popup.
    pub fn handle_button_press(&self, x: i16, y: i16) -> bool {
        if self.overflow_xembed() == 0 {
            return false;
        }
        let slot_x = (self.visible_xembed() as i16 + self.sni_visible() as i16) * self.icon_size as i16;
        let slot_w = self.icon_size as i16;
        x >= slot_x && x < slot_x + slot_w && y >= 0 && y < self.icon_size as i16
    }

    /// Register a new StatusNotifierItem (called from `SniEvent::Registered`).
    pub fn sni_add(&mut self, conn: &X11Connection, service: &str) -> Result<(), anyhow::Error> {
        if self.sni_slots.iter().any(|s| s.service == service) {
            return Ok(());
        }
        self.sni_slots.push(SniSlot {
            service: service.to_string(),
            pixmap: None,
        });
        log::info!("SNI: slot adicionado para {}", service);
        self.reposition(conn)?;
        Ok(())
    }

    /// Remove a StatusNotifierItem (called from `SniEvent::Unregistered`).
    pub fn sni_remove(&mut self, conn: &X11Connection, service: &str) -> Result<(), anyhow::Error> {
        if let Some(pos) = self.sni_slots.iter().position(|s| s.service == service) {
            let slot = self.sni_slots.remove(pos);
            if let Some(pm) = slot.pixmap {
                let _ = conn.conn().free_pixmap(pm);
            }
            log::info!("SNI: slot removido para {}", service);
            self.reposition(conn)?;
        }
        Ok(())
    }

    /// Update a StatusNotifierItem's icon (called from `SniEvent::Updated`).
    pub fn sni_update(
        &mut self,
        conn: &X11Connection,
        service: &str,
        width: u32,
        height: u32,
        argb: &[u8],
    ) -> Result<(), anyhow::Error> {
        let img = match crate::render::image::Image::from_argb32(width, height, argb) {
            Ok(i) => i,
            Err(e) => {
                log::warn!("SNI: icon无效o de {}: {}", service, e);
                return Ok(());
            }
        };
        let img = img.scale(self.icon_size as u32, self.icon_size as u32)?;
        let pm = img.create_pixmap(conn.conn(), conn.screen(), self.window)?;
        if let Some(slot) = self.sni_slots.iter_mut().find(|s| s.service == service) {
            if let Some(old) = slot.pixmap {
                let _ = conn.conn().free_pixmap(old);
            }
            slot.pixmap = Some(pm);
        }
        self.sni_redraw(conn)?;
        Ok(())
    }

    /// Given a tray-local click position, return the SNI service that owns the
    /// slot under the cursor (if any). Only slots with a pixmap are clickable.
    pub fn sni_slot_at(&self, x: i16, y: i16) -> Option<String> {
        if y < 0 || y >= self.icon_size as i16 {
            return None;
        }
        let idx = (x / self.icon_size as i16) as usize;
        let vx = self.visible_xembed();
        let mut visual = 0;
        for slot in &self.sni_slots {
            if slot.pixmap.is_some() {
                if idx == vx + visual {
                    return Some(slot.service.clone());
                }
                visual += 1;
            }
        }
        None
    }

    /// Copy every SNI slot's pixmap onto the tray window.
    pub fn sni_redraw(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        if self.sni_slots.is_empty() {
            return Ok(());
        }
        let vx = self.visible_xembed();
        let mut visual = 0;
        for slot in &self.sni_slots {
            if let Some(pm) = slot.pixmap {
                let sx = (vx + visual) as i16 * self.icon_size as i16;
                conn.conn().copy_area(
                    pm,
                    self.window,
                    self.gc,
                    0,
                    0,
                    sx,
                    0,
                    self.icon_size,
                    self.icon_size,
                )?;
                visual += 1;
            }
        }
        Ok(())
    }

    /// Recalculate tray size and reposition everything (tray window, visible
    /// icons, SNI slots, overflow icons/popup).
    pub fn reposition(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        if self.total_slots() == 0 {
            conn.conn().unmap_window(self.window)?;
            conn.conn().unmap_window(self.popup_window)?;
            return Ok(());
        }

        let visible = self.visible_xembed();
        let sni = self.sni_visible();
        let overflow = self.overflow_xembed();
        let chevron = overflow > 0;
        let slots = visible + sni + if chevron { 1 } else { 0 };
        let tray_w = (slots as i16) * self.icon_size as i16;
        let tray_h = self.icon_size;
        let tray_x = self.right_anchor - tray_w;
        // Bottom-align with the toolbar (height 24 + 2px border). Drop 2px
        // so the tray sits snugly inside the toolbar.
        let tray_y = (self.screen_height as i16) - (self.icon_size as i16) - 2;
        log::debug!(
            "Tray: icons={} visible={} sni={} x={} y={} w={} h={} anchor={}",
            self.icons.len(),
            visible,
            sni,
            tray_x,
            tray_y,
            tray_w,
            tray_h,
            self.right_anchor,
        );

        if slots == 0 {
            conn.conn().unmap_window(self.window)?;
            conn.conn().unmap_window(self.popup_window)?;
            return Ok(());
        }

        conn.conn().configure_window(
            self.window,
            &xproto::ConfigureWindowAux::new()
                .x(tray_x as i32)
                .y(tray_y as i32)
                .width(tray_w as u32)
                .height(tray_h as u32)
                .stack_mode(xproto::StackMode::ABOVE),
        )?;
        conn.conn().map_window(self.window)?;

        // Visible XEmbed icons occupy the first `visible` slots.
        for (i, icon) in self.icons.iter().take(visible).enumerate() {
            let px = (i as i16) * self.icon_size as i16;
            let _ = conn.conn().configure_window(
                icon.window,
                &xproto::ConfigureWindowAux::new().x(px as i32).y(0),
            );
        }

        // Overflow icons: show in the popup when open, otherwise keep them
        // reparented to the container but unmapped (off-stage).
        if self.popup_open && overflow > 0 {
            let popup_w = self.icon_size as i16;
            let popup_h = (overflow as i16) * self.icon_size as i16;
            // Anchor the popup to the chevron slot: chevron slot screen-x =
            // tray_x + (visible+sni)*icon_size = right_anchor - icon_size.
            let popup_x = self.right_anchor - self.icon_size as i16;
            let popup_y = tray_y - popup_h;

            conn.conn().configure_window(
                self.popup_window,
                &xproto::ConfigureWindowAux::new()
                    .x(popup_x as i32)
                    .y(popup_y as i32)
                    .width(popup_w as u32)
                    .height(popup_h as u32)
                    .stack_mode(xproto::StackMode::ABOVE),
            )?;

            for (j, icon) in self.icons.iter().skip(visible).enumerate() {
                let py = (j as i16) * self.icon_size as i16;
                let _ = conn.conn().reparent_window(
                    icon.window,
                    self.popup_window,
                    0,
                    py as i16,
                );
                let _ = conn.conn().configure_window(
                    icon.window,
                    &xproto::ConfigureWindowAux::new().x(0).y(py as i32),
                );
                let _ = conn.conn().map_window(icon.window);
            }
            conn.conn().map_window(self.popup_window)?;
        } else {
            // Popup closed: send overflow icons back to the container,
            // parked beyond its width and unmapped so they don't show.
            conn.conn().unmap_window(self.popup_window)?;
            for (_j, icon) in self.icons.iter().skip(visible).enumerate() {
                let _ = conn.conn().reparent_window(
                    icon.window,
                    self.window,
                    (slots as i16) * self.icon_size as i16,
                    0,
                );
                let _ = conn.conn().unmap_window(icon.window);
            }
        }

        // SNI slots carry no XEmbed window — they are painted onto the tray.
        self.sni_redraw(conn)?;

        conn.conn().flush()?;
        Ok(())
    }

    /// Update the stored screen geometry and anchor, then refit.
    pub fn reconfigure(
        &mut self,
        conn: &X11Connection,
        width: u16,
        height: u16,
        anchor: i16,
    ) -> Result<(), anyhow::Error> {
        self.screen_width = width;
        self.screen_height = height;
        self.right_anchor = anchor;
        self.reposition(conn)
    }

    pub fn handle_expose(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        let visible = self.visible_xembed();
        let sni = self.sni_visible();
        let chevron = self.overflow_xembed() > 0;
        if visible == 0 && sni == 0 && !chevron {
            return Ok(());
        }
        let slots = visible + sni + if chevron { 1 } else { 0 };
        let w = (slots as i16) * self.icon_size as i16;

        conn.conn().change_gc(
            self.gc,
            &xproto::ChangeGCAux::new()
                .foreground(self.bg_pixel)
                .line_width(0),
        )?;
        conn.conn().poly_fill_rectangle(
            self.window,
            self.gc,
            &[xproto::Rectangle {
                x: 0,
                y: 0,
                width: w as u16,
                height: self.icon_size,
            }],
        )?;

        // Paint SNI slots onto the tray.
        self.sni_redraw(conn)?;

        // Draw the chevron ("⌄") in the last slot when there is overflow.
        if chevron {
            let slot_x = (visible as i16 + sni as i16) * self.icon_size as i16;
            let cx = slot_x + (self.icon_size as i16) / 2;
            let top = 4i16;
            let bottom = (self.icon_size as i16) - 4;
            conn.conn().change_gc(
                self.gc,
                &xproto::ChangeGCAux::new()
                    .foreground(self.fg_pixel)
                    .line_width(2),
            )?;
            conn.conn().poly_line(
                xproto::CoordMode::ORIGIN,
                self.window,
                self.gc,
                &[
                    xproto::Point { x: cx - 6, y: top },
                    xproto::Point { x: cx, y: bottom },
                    xproto::Point { x: cx + 6, y: top },
                ],
            )?;
            // Reset line width for subsequent draws.
            let _ = conn.conn().change_gc(
                self.gc,
                &xproto::ChangeGCAux::new().line_width(0),
            );
        }
        Ok(())
    }

    /// Paint the overflow popup background.
    pub fn handle_popup_expose(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        let ov = self.overflow_xembed();
        if ov == 0 {
            return Ok(());
        }
        let w = self.icon_size;
        let h = (ov as i16) * self.icon_size as i16;
        conn.conn().change_gc(
            self.gc,
            &xproto::ChangeGCAux::new().foreground(self.bg_pixel),
        )?;
        let _ = conn.conn().poly_fill_rectangle(
            self.popup_window,
            self.gc,
            &[xproto::Rectangle {
                x: 0,
                y: 0,
                width: w,
                height: h as u16,
            }],
        );
        Ok(())
    }

    pub fn destroy(&self, conn: &X11Connection) -> Result<(), anyhow::Error> {
        for icon in &self.icons {
            let _ = conn.conn().reparent_window(icon.window, conn.root_window(), 0, 0);
        }
        let _ = conn.conn().free_gc(self.gc);
        let _ = conn.conn().destroy_window(self.popup_window);
        // Free any SNI icon pixmaps to avoid leaking server-side VRAM.
        for slot in &self.sni_slots {
            if let Some(pm) = slot.pixmap {
                let _ = conn.conn().free_pixmap(pm);
            }
        }
        conn.conn().destroy_window(self.window)?;
        Ok(())
    }
}
