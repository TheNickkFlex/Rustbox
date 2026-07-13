use x11rb::protocol::xproto::{self, Gcontext, Font, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;
use x11rb::connection::Connection;

#[derive(Debug, Clone)]
pub struct GContext {
    gc: Gcontext,
}

impl GContext {
    pub fn create(conn: &RustConnection, drawable: u32) -> Result<Self, anyhow::Error> {
        let gc = conn.generate_id()?;
        conn.create_gc(gc, drawable, &xproto::CreateGCAux::new())?;
        Ok(Self { gc })
    }

    pub fn create_with(conn: &RustConnection, drawable: u32, aux: &xproto::CreateGCAux) -> Result<Self, anyhow::Error> {
        let gc = conn.generate_id()?;
        conn.create_gc(gc, drawable, aux)?;
        Ok(Self { gc })
    }

    pub fn id(&self) -> Gcontext {
        self.gc
    }

    pub fn set_foreground(&self, conn: &RustConnection, pixel: u32) -> Result<(), anyhow::Error> {
        conn.change_gc(self.gc, &xproto::ChangeGCAux::new().foreground(pixel))?;
        Ok(())
    }

    pub fn set_background(&self, conn: &RustConnection, pixel: u32) -> Result<(), anyhow::Error> {
        conn.change_gc(self.gc, &xproto::ChangeGCAux::new().background(pixel))?;
        Ok(())
    }

    pub fn set_font(&self, conn: &RustConnection, font: Font) -> Result<(), anyhow::Error> {
        conn.change_gc(self.gc, &xproto::ChangeGCAux::new().font(font))?;
        Ok(())
    }

    pub fn set_line_width(&self, conn: &RustConnection, width: u32) -> Result<(), anyhow::Error> {
        conn.change_gc(self.gc, &xproto::ChangeGCAux::new().line_width(width))?;
        Ok(())
    }

    pub fn set_subwindow_mode(&self, conn: &RustConnection, mode: xproto::SubwindowMode) -> Result<(), anyhow::Error> {
        conn.change_gc(self.gc, &xproto::ChangeGCAux::new().subwindow_mode(mode))?;
        Ok(())
    }

    pub fn set_gc_aux(&self, conn: &RustConnection, aux: &xproto::ChangeGCAux) -> Result<(), anyhow::Error> {
        conn.change_gc(self.gc, aux)?;
        Ok(())
    }

    pub fn free(&self, conn: &RustConnection) -> Result<(), anyhow::Error> {
        conn.free_gc(self.gc)?;
        Ok(())
    }
}

impl From<Gcontext> for GContext {
    fn from(gc: Gcontext) -> Self {
        Self { gc }
    }
}
