use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Pixmap as X11Pixmap, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

use crate::x11::drawable::Drawable;

pub struct Pixmap {
    id: X11Pixmap,
    width: u16,
    height: u16,
}

impl Pixmap {
    pub fn create(conn: &RustConnection, drawable: u32, width: u16, height: u16, depth: u8) -> Result<Self, anyhow::Error> {
        let id = conn.generate_id()?;
        conn.create_pixmap(depth, id, drawable, width, height)?;
        Ok(Self { id, width, height })
    }

    pub fn id(&self) -> X11Pixmap {
        self.id
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    pub fn free(&self, conn: &RustConnection) -> Result<(), anyhow::Error> {
        conn.free_pixmap(self.id)?;
        Ok(())
    }
}

impl Drawable for Pixmap {
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
