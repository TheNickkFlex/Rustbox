use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

use crate::x11::X11Connection;
use x11rb::protocol::xproto;

pub const UNSET: u32 = u32::MAX;

// ── Color overrides ──────────────────────────────────────────────

pub static TOOLBAR_BG: AtomicU32 = AtomicU32::new(UNSET);
pub static TOOLBAR_FG: AtomicU32 = AtomicU32::new(UNSET);
pub static FRAME_BG: AtomicU32 = AtomicU32::new(UNSET);
pub static FRAME_FG: AtomicU32 = AtomicU32::new(UNSET);
pub static FRAME_FOCUSED_BG: AtomicU32 = AtomicU32::new(UNSET);
pub static FRAME_FOCUSED_FG: AtomicU32 = AtomicU32::new(UNSET);
pub static MENU_BG: AtomicU32 = AtomicU32::new(UNSET);
pub static MENU_FG: AtomicU32 = AtomicU32::new(UNSET);
pub static MENU_HI: AtomicU32 = AtomicU32::new(UNSET);
pub static MENU_SEL: AtomicU32 = AtomicU32::new(UNSET);
pub static TRAY_BG: AtomicU32 = AtomicU32::new(UNSET);
pub static TRAY_FG: AtomicU32 = AtomicU32::new(UNSET);
pub static DIALOG_BG: AtomicU32 = AtomicU32::new(UNSET);
pub static DIALOG_FG: AtomicU32 = AtomicU32::new(UNSET);
pub static SLIT_BG: AtomicU32 = AtomicU32::new(UNSET);
pub static SLIT_FG: AtomicU32 = AtomicU32::new(UNSET);

pub fn or(at: &AtomicU32, default: u32) -> u32 {
    let v = at.load(Ordering::Relaxed);
    if v == UNSET { default } else { v }
}

// ── Creation callbacks ───────────────────────────────────────────

pub static AFTER_FRAME_CREATE: OnceLock<
    fn(conn: &X11Connection, frame_window: xproto::Window, width: u16, height: u16)
> = OnceLock::new();

pub static AFTER_FRAME_RESIZE: OnceLock<
    fn(conn: &X11Connection, frame_window: xproto::Window, width: u16, height: u16)
> = OnceLock::new();

pub static AFTER_TOOLBAR_CREATE: OnceLock<
    fn(conn: &X11Connection, window: xproto::Window, width: u16, height: u16)
> = OnceLock::new();

pub static AFTER_MENU_CREATE: OnceLock<
    fn(conn: &X11Connection, window: xproto::Window, width: u16, height: u16)
> = OnceLock::new();

pub static AFTER_TRAY_CREATE: OnceLock<
    fn(conn: &X11Connection, window: xproto::Window, width: u16, height: u16)
> = OnceLock::new();

pub static AFTER_NOTIFY_CREATE: OnceLock<
    fn(conn: &X11Connection, window: xproto::Window, width: u16, height: u16)
> = OnceLock::new();

pub static AFTER_SLIT_CREATE: OnceLock<
    fn(conn: &X11Connection, window: xproto::Window, width: u16, height: u16)
> = OnceLock::new();
