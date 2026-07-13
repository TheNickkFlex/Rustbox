use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, Window, Pixmap, Cursor, Visualid, EventMask, StackMode, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

use crate::core::Rectangle;
use crate::x11::drawable::Drawable;

pub struct FbWindow {
    id: Window,
    parent: Window,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    border_width: u16,
    depth: u8,
    class: xproto::WindowClass,
    override_redirect: bool,
}

impl FbWindow {
    pub fn create(
        conn: &RustConnection,
        parent: Window,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        border_width: u16,
        depth: u8,
        class: xproto::WindowClass,
        visual: Visualid,
        aux: &xproto::CreateWindowAux,
    ) -> Result<Self, anyhow::Error> {
        let id = conn.generate_id()?;
        conn.create_window(depth, id, parent, x, y, width, height, border_width, class, visual, aux)?;
        Ok(Self {
            id,
            parent,
            x,
            y,
            width,
            height,
            border_width,
            depth,
            class,
            override_redirect: false,
        })
    }

    pub fn id(&self) -> Window {
        self.id
    }

    pub fn parent(&self) -> Window {
        self.parent
    }

    pub fn configure(&mut self, conn: &RustConnection, aux: &xproto::ConfigureWindowAux) -> Result<(), anyhow::Error> {
        conn.configure_window(self.id, aux)?;
        if let Some(x) = aux.x {
            self.x = x as i16;
        }
        if let Some(y) = aux.y {
            self.y = y as i16;
        }
        if let Some(w) = aux.width {
            self.width = w as u16;
        }
        if let Some(h) = aux.height {
            self.height = h as u16;
        }
        if let Some(bw) = aux.border_width {
            self.border_width = bw as u16;
        }
        Ok(())
    }

    pub fn map(&self, conn: &RustConnection) -> Result<(), anyhow::Error> {
        conn.map_window(self.id)?;
        Ok(())
    }

    pub fn unmap(&self, conn: &RustConnection) -> Result<(), anyhow::Error> {
        conn.unmap_window(self.id)?;
        Ok(())
    }

    pub fn destroy(&self, conn: &RustConnection) -> Result<(), anyhow::Error> {
        conn.destroy_window(self.id)?;
        Ok(())
    }

    pub fn raise(&self, conn: &RustConnection) -> Result<(), anyhow::Error> {
        conn.configure_window(self.id, &xproto::ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
        Ok(())
    }

    pub fn lower(&self, conn: &RustConnection) -> Result<(), anyhow::Error> {
        conn.configure_window(self.id, &xproto::ConfigureWindowAux::new().stack_mode(StackMode::BELOW))?;
        Ok(())
    }

    pub fn set_event_mask(&self, conn: &RustConnection, mask: EventMask) -> Result<(), anyhow::Error> {
        conn.change_window_attributes(self.id, &xproto::ChangeWindowAttributesAux::new().event_mask(mask))?;
        Ok(())
    }

    pub fn get_geometry(&self, conn: &RustConnection) -> Result<Rectangle, anyhow::Error> {
        let geom = conn.get_geometry(self.id)?.reply()?;
        Ok(Rectangle::new(geom.x, geom.y, geom.width, geom.height))
    }

    pub fn reparent(&self, conn: &RustConnection, new_parent: Window, x: i16, y: i16) -> Result<(), anyhow::Error> {
        conn.reparent_window(self.id, new_parent, x, y)?;
        Ok(())
    }

    pub fn set_background_pixmap(&self, conn: &RustConnection, pixmap: Pixmap) -> Result<(), anyhow::Error> {
        conn.change_window_attributes(self.id, &xproto::ChangeWindowAttributesAux::new().background_pixmap(pixmap))?;
        Ok(())
    }

    pub fn set_background_color(&self, conn: &RustConnection, pixel: u32) -> Result<(), anyhow::Error> {
        conn.change_window_attributes(self.id, &xproto::ChangeWindowAttributesAux::new().background_pixel(pixel))?;
        Ok(())
    }

    pub fn set_cursor(&self, conn: &RustConnection, cursor: Cursor) -> Result<(), anyhow::Error> {
        conn.change_window_attributes(self.id, &xproto::ChangeWindowAttributesAux::new().cursor(cursor))?;
        Ok(())
    }

    pub fn set_override_redirect(&mut self, conn: &RustConnection, redirect: bool) -> Result<(), anyhow::Error> {
        conn.change_window_attributes(self.id, &xproto::ChangeWindowAttributesAux::new().override_redirect(redirect as u32))?;
        self.override_redirect = redirect;
        Ok(())
    }
}

impl Drawable for FbWindow {
    fn id(&self) -> u32 {
        self.id
    }

    fn width(&self) -> u16 {
        self.width
    }

    fn height(&self) -> u16 {
        self.height
    }
}
