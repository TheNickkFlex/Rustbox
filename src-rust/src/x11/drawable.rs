use x11rb::protocol::xproto::{self, Gcontext, ConnectionExt as _};
use x11rb::rust_connection::RustConnection;

pub trait Drawable {
    fn id(&self) -> u32;

    fn width(&self) -> u16;

    fn height(&self) -> u16;

    fn draw_rectangle(&self, conn: &RustConnection, gc: Gcontext, x: i16, y: i16, w: u16, h: u16, fill: bool) -> Result<(), anyhow::Error> {
        if fill {
            conn.poly_fill_rectangle(self.id(), gc, &[xproto::Rectangle { x, y, width: w, height: h }])?;
        } else {
            conn.poly_rectangle(self.id(), gc, &[xproto::Rectangle { x, y, width: w, height: h }])?;
        }
        Ok(())
    }

    fn clear(&self, conn: &RustConnection) -> Result<(), anyhow::Error> {
        conn.clear_area(true, self.id(), 0, 0, self.width(), self.height())?;
        Ok(())
    }

    fn copy_area(&self, conn: &RustConnection, gc: Gcontext, src_x: i16, src_y: i16, dst_x: i16, dst_y: i16, width: u16, height: u16, dst: &impl Drawable) -> Result<(), anyhow::Error> {
        conn.copy_area(self.id(), dst.id(), gc, src_x, src_y, dst_x, dst_y, width, height)?;
        Ok(())
    }
}
