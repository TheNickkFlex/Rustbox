use std::os::unix::io::AsRawFd;
use std::os::unix::process::CommandExt;

use ab_glyph::{Font as AbFont, FontRef, PxScale, ScaleFont};
use anyhow::Result;
use terminal_emulator::ansi::{Color, NamedColor};
use terminal_emulator::term::{SizeInfo, Term};
use terminal_emulator::term::cell::Flags;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;

// ── Color palette (standard ANSI 16 + extras) ─────────────────────────

fn named_rgb(name: NamedColor) -> [u8; 3] {
    match name {
        NamedColor::Black => [0x00, 0x00, 0x00],
        NamedColor::Red => [0xAA, 0x00, 0x00],
        NamedColor::Green => [0x00, 0xAA, 0x00],
        NamedColor::Yellow => [0xAA, 0x55, 0x00],
        NamedColor::Blue => [0x00, 0x00, 0xAA],
        NamedColor::Magenta => [0xAA, 0x00, 0xAA],
        NamedColor::Cyan => [0x00, 0xAA, 0xAA],
        NamedColor::White => [0xAA, 0xAA, 0xAA],
        NamedColor::BrightBlack => [0x55, 0x55, 0x55],
        NamedColor::BrightRed => [0xFF, 0x55, 0x55],
        NamedColor::BrightGreen => [0x55, 0xFF, 0x55],
        NamedColor::BrightYellow => [0xFF, 0xFF, 0x55],
        NamedColor::BrightBlue => [0x55, 0x55, 0xFF],
        NamedColor::BrightMagenta => [0xFF, 0x55, 0xFF],
        NamedColor::BrightCyan => [0x55, 0xFF, 0xFF],
        NamedColor::BrightWhite => [0xFF, 0xFF, 0xFF],
        NamedColor::Foreground => [0xFF, 0xFF, 0xFF],
        NamedColor::Background => [0x00, 0x00, 0x00],
        NamedColor::CursorText => [0x00, 0x00, 0x00],
        NamedColor::Cursor => [0xFF, 0xFF, 0xFF],
        NamedColor::DimForeground => [0x80, 0x80, 0x80],
        _ => [0xFF, 0xFF, 0xFF],
    }
}

fn color_to_rgb(color: Color) -> [u8; 3] {
    match color {
        Color::Named(n) => named_rgb(n),
        Color::Spec(rgb) => [rgb.r, rgb.g, rgb.b],
        Color::Indexed(i) => {
            if i < 16 {
                named_rgb(match i {
                    0 => NamedColor::Black,
                    1 => NamedColor::Red,
                    2 => NamedColor::Green,
                    3 => NamedColor::Yellow,
                    4 => NamedColor::Blue,
                    5 => NamedColor::Magenta,
                    6 => NamedColor::Cyan,
                    7 => NamedColor::White,
                    8 => NamedColor::BrightBlack,
                    9 => NamedColor::BrightRed,
                    10 => NamedColor::BrightGreen,
                    11 => NamedColor::BrightYellow,
                    12 => NamedColor::BrightBlue,
                    13 => NamedColor::BrightMagenta,
                    14 => NamedColor::BrightCyan,
                    15 => NamedColor::BrightWhite,
                    _ => unreachable!(),
                })
            } else if i < 232 {
                let idx = i - 16;
                let r = (idx / 36) as u8;
                let g = ((idx % 36) / 6) as u8;
                let b = (idx % 6) as u8;
                let expand = |v: u8| -> u8 { if v == 0 { 0 } else { 55 + v * 40 } };
                [expand(r), expand(g), expand(b)]
            } else {
                let g = 8 + (i - 232) * 10;
                [g, g, g]
            }
        }
    }
}

fn lerp_u8(a: u8, b: u8, t: u8) -> u8 {
    let t = t as u16;
    let a = a as u16;
    let b = b as u16;
    ((a * (255 - t) + b * t) / 255) as u8
}

// ── System font loader ─────────────────────────────────────────────────

struct Font {
    data: Vec<u8>,
    px: f32,
    cell_w: u16,
    cell_h: u16,
    ascent: f32,
}

impl Font {
    fn new(px_size: f32) -> Option<Self> {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();

        let families = [
            fontdb::Family::Monospace,
            fontdb::Family::Name("Fira Code"),
            fontdb::Family::Name("JetBrains Mono"),
            fontdb::Family::Name("DejaVu Sans Mono"),
            fontdb::Family::Name("Liberation Mono"),
            fontdb::Family::Name("Noto Sans Mono"),
            fontdb::Family::Name("Cascadia Code"),
            fontdb::Family::Name("Source Code Pro"),
        ];

        let id = families.iter().find_map(|f| {
            let q = fontdb::Query {
                families: &[f.clone()],
                weight: fontdb::Weight::NORMAL,
                ..fontdb::Query::default()
            };
            db.query(&q)
        })?;

        let (source, _face_index) = db.face_source(id)?;
        let data = match source {
            fontdb::Source::Binary(arc) => arc.as_ref().as_ref().to_vec(),
            fontdb::Source::File(path) => std::fs::read(path).ok()?,
            fontdb::Source::SharedFile(_, arc) => arc.as_ref().as_ref().to_vec(),
        };

        let font = FontRef::try_from_slice(&data).ok()?;

        let scaled = font.as_scaled(PxScale::from(px_size));
        let advance = scaled.h_advance(scaled.glyph_id('W'));
        let cell_w = advance.ceil().max(1.0) as u16;
        let cell_h = (scaled.height().ceil().max(1.0)) as u16;
        let ascent = scaled.ascent();

        Some(Self {
            data,
            px: px_size,
            cell_w,
            cell_h,
            ascent,
        })
    }

    fn rasterize_glyph(&self, ch: char, buf: &mut [u8], buf_w: usize, x: usize, y: usize, fg: [u8; 3]) -> Option<()> {
        let font = FontRef::try_from_slice(&self.data).ok()?;
        let scaled = font.as_scaled(PxScale::from(self.px));
        let glyph = scaled.scaled_glyph(ch);
        let og = scaled.outline_glyph(glyph)?;
        let bounds = og.px_bounds();
        let ox = bounds.min.x.floor() as i32;
        let oy = bounds.min.y.floor() as i32;

        og.draw(|gx, gy, coverage| {
            if coverage == 0.0 {
                return;
            }
            let dx = x as i32 + ox + gx as i32;
            let dy = (y as i32 + oy + gy as i32) + self.ascent.ceil() as i32;
            if dx < 0 || dy < 0 || dx >= buf_w as i32 || dy >= (buf.len() / (buf_w * 4)) as i32 {
                return;
            }
            let pi = ((dy as usize) * buf_w + (dx as usize)) * 4;
            let cov = (coverage * 255.0) as u8;
            if cov >= 254 {
                buf[pi] = fg[0];
                buf[pi + 1] = fg[1];
                buf[pi + 2] = fg[2];
            } else {
                buf[pi] = lerp_u8(buf[pi], fg[0], cov);
                buf[pi + 1] = lerp_u8(buf[pi + 1], fg[1], cov);
                buf[pi + 2] = lerp_u8(buf[pi + 2], fg[2], cov);
            }
        });
        Some(())
    }

    fn rows(&self) -> u16 { 24 }
    fn cols(&self) -> u16 { 80 }
}

// ── PTY ────────────────────────────────────────────────────────────────

struct Pty {
    master: std::os::unix::io::RawFd,
}

impl Pty {
    fn new(rows: u16, cols: u16) -> Result<Self> {
        let mut master: std::os::unix::io::RawFd = 0;
        let mut win = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let pid = unsafe {
            libc::forkpty(&mut master, std::ptr::null_mut(), std::ptr::null_mut(), &mut win)
        };
        if pid == -1 {
            anyhow::bail!("forkpty failed");
        }
        if pid == 0 {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
            std::env::set_var("TERM", "xterm-256color");
            let _ = std::process::Command::new(shell).exec();
            std::process::exit(1);
        }
        unsafe { libc::signal(libc::SIGCHLD, libc::SIG_IGN) };
        Ok(Self { master })
    }

    fn write(&self, data: &[u8]) -> Result<()> {
        unsafe {
            libc::write(self.master, data.as_ptr() as *const libc::c_void, data.len());
        }
        Ok(())
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let n = unsafe {
            libc::read(
                self.master,
                buf.as_mut_ptr() as *mut libc::c_void,
                buf.len(),
            )
        };
        if n < 0 {
            anyhow::bail!("pty read error");
        }
        Ok(n as usize)
    }

    fn fd(&self) -> std::os::unix::io::RawFd {
        self.master
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        unsafe { libc::close(self.master) };
    }
}

impl std::io::Write for Pty {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = unsafe { libc::write(self.master, buf.as_ptr() as *const libc::c_void, buf.len()) };
        if n < 0 { Err(std::io::Error::last_os_error()) } else { Ok(n as usize) }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ── Renderer ──────────────────────────────────────────────────────────

struct Renderer {
    buf: Vec<u8>,
    win_w: u16,
    win_h: u16,
    cols: u16,
    rows: u16,
    cell_w: u16,
    cell_h: u16,
}

impl Renderer {
    fn new(font: &Font) -> Self {
        let cell_w = font.cell_w;
        let cell_h = font.cell_h;
        let cols = font.cols();
        let rows = font.rows();
        let win_w = cols * cell_w;
        let win_h = rows * cell_h;
        let buf = vec![0u8; win_w as usize * win_h as usize * 4];
        Self { buf, win_w, win_h, cols, rows, cell_w, cell_h }
    }

    fn render(&mut self, font: &Font, term: &Term) {
        let w = self.win_w as usize;
        let h = self.win_h as usize;
        let stride = w * 4;

        let bg = color_to_rgb(Color::Named(NamedColor::Background));
        for row in self.buf.chunks_exact_mut(stride) {
            for pixel in row.chunks_exact_mut(4) {
                pixel[0] = bg[0];
                pixel[1] = bg[1];
                pixel[2] = bg[2];
                pixel[3] = 0xFF;
            }
        }

        for cell in term.renderable_cells() {
            let cx = cell.column.0 as usize;
            let cy = cell.line.0 as usize;
            if cx >= self.cols as usize || cy >= self.rows as usize {
                continue;
            }

            let is_inverse = cell.flags.contains(Flags::INVERSE);
            let fg_rgb = if is_inverse {
                color_to_rgb(cell.bg)
            } else {
                color_to_rgb(cell.fg)
            };
            let bg_rgb = if is_inverse {
                color_to_rgb(cell.fg)
            } else {
                color_to_rgb(cell.bg)
            };

            let px = cx * self.cell_w as usize;
            let py = cy * self.cell_h as usize;

            let non_default_bg = cell.bg != Color::Named(NamedColor::Background) || is_inverse;
            if non_default_bg {
                for row in 0..self.cell_h as usize {
                    let dy = py + row;
                    if dy >= h { continue; }
                    for col in 0..self.cell_w as usize {
                        let dx = px + col;
                        if dx >= w { continue; }
                        let pi = (dy * w + dx) * 4;
                        self.buf[pi] = bg_rgb[0];
                        self.buf[pi + 1] = bg_rgb[1];
                        self.buf[pi + 2] = bg_rgb[2];
                    }
                }
            }

            let ch = cell.chars[0];
            if ch == ' ' || ch == '\t' {
                continue;
            }

            font.rasterize_glyph(ch, &mut self.buf, w, px, py, fg_rgb);
        }
    }

    fn draw(&self, conn: &impl Connection, window: u32, gc: u32, depth: u8) -> Result<()> {
        conn.put_image(
            x11rb::protocol::xproto::ImageFormat::Z_PIXMAP,
            window,
            gc,
            self.win_w,
            self.win_h,
            0,
            0,
            0,
            depth,
            &self.buf,
        )?;
        conn.flush()?;
        Ok(())
    }
}

// ── Keyboard ───────────────────────────────────────────────────────────

fn keysym_to_bytes(ks: u32, shift: bool, ctrl: bool) -> Option<Vec<u8>> {
    match ks {
        0xFF08 => Some(vec![0x7f]),
        0xFF09 => Some(vec![0x09]),
        0xFF0D => Some(vec![0x0D]),
        0xFF1B => Some(vec![0x1B]),
        0xFF50 => Some(b"\x1b[H".to_vec()),
        0xFF57 => Some(b"\x1b[F".to_vec()),
        0xFF51 => Some(b"\x1b[D".to_vec()),
        0xFF52 => Some(b"\x1b[A".to_vec()),
        0xFF53 => Some(b"\x1b[C".to_vec()),
        0xFF54 => Some(b"\x1b[B".to_vec()),
        0xFF55 => Some(b"\x1b[5~".to_vec()),
        0xFF56 => Some(b"\x1b[6~".to_vec()),
        0xFFBE..=0xFFC9 => {
            let n = (ks - 0xFFBE) + 1;
            Some(format!("\x1b[{}~", 10 + n).into_bytes())
        }
        _ => {
            if let Some(ch) = char::from_u32(ks) {
                if ctrl && ch.is_ascii_lowercase() {
                    Some(vec![(ch as u8 - b'a' + 1)])
                } else {
                    let s = if shift { ch.to_ascii_uppercase() } else { ch };
                    Some(s.to_string().into_bytes())
                }
            } else {
                None
            }
        }
    }
}

// ── Main ───────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = conn.setup().roots[screen_num as usize].clone();
    let depth = screen.root_depth;

    let font = Font::new(14.0).expect("no monospace font found");
    let renderer = Renderer::new(&font);
    let win_w = renderer.win_w;
    let win_h = renderer.win_h;
    let rows = renderer.rows;
    let cols = renderer.cols;

    let window = conn.generate_id()?;
    conn.create_window(
        depth,
        window,
        screen.root,
        0,
        0,
        win_w,
        win_h,
        0,
        x11rb::protocol::xproto::WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &x11rb::protocol::xproto::CreateWindowAux::default()
            .background_pixel(screen.black_pixel)
            .event_mask(
                x11rb::protocol::xproto::EventMask::EXPOSURE
                    | x11rb::protocol::xproto::EventMask::KEY_PRESS
                    | x11rb::protocol::xproto::EventMask::STRUCTURE_NOTIFY,
            ),
    )?;

    conn.map_window(window)?;

    let gc = conn.generate_id()?;
    conn.create_gc(gc, window, &x11rb::protocol::xproto::CreateGCAux::default())?;

    let wm_name = conn.intern_atom(false, b"WM_NAME")?.reply()?.atom;
    let wm_label = b"rustbox-terminal";
    conn.change_property::<u32, u32>(
        x11rb::protocol::xproto::PropMode::REPLACE,
        window,
        wm_name,
        x11rb::protocol::xproto::AtomEnum::STRING.into(),
        8,
        wm_label.len() as u32,
        wm_label,
    )?;
    conn.flush()?;

    let mut pty = Pty::new(rows, cols)?;

    let size = SizeInfo {
        width: win_w as f32,
        height: win_h as f32,
        cell_width: renderer.cell_w as f32,
        cell_height: renderer.cell_h as f32,
        padding_x: 0.0,
        padding_y: 0.0,
        dpr: 1.0,
    };
    let mut term = Term::new(size);
    let mut parser = terminal_emulator::ansi::Processor::new();
    let mut pty_buf = [0u8; 4096];
    let mut renderer = renderer;

    let x11_fd = conn.stream().as_raw_fd();

    let wm_protocols = conn.intern_atom(false, b"WM_PROTOCOLS")?.reply()?.atom;
    let wm_delete = conn.intern_atom(false, b"WM_DELETE_WINDOW")?.reply()?.atom;
    let mut running = true;
    let mut need_redraw = true;

    while running {
        unsafe {
            let mut readfds: libc::fd_set = std::mem::zeroed();
            libc::FD_ZERO(&mut readfds);
            libc::FD_SET(x11_fd, &mut readfds);
            libc::FD_SET(pty.fd(), &mut readfds);

            let mut tv = if need_redraw {
                libc::timeval { tv_sec: 0, tv_usec: 0 }
            } else {
                libc::timeval { tv_sec: 60, tv_usec: 0 }
            };

            let ret = libc::select(
                x11_fd.max(pty.fd()) + 1,
                &mut readfds,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut tv,
            );
            if ret < 0 {
                anyhow::bail!("select failed");
            }

            if libc::FD_ISSET(x11_fd, &mut readfds) {
                loop {
                    match conn.poll_for_event()? {
                        Some(x11rb::protocol::Event::Expose(ev)) if ev.window == window => {
                            need_redraw = true;
                        }
                        Some(x11rb::protocol::Event::KeyPress(ev)) if ev.detail != 0 => {
                            let shift = u16::from(ev.state)
                                & u16::from(x11rb::protocol::xproto::KeyButMask::SHIFT)
                                != 0;
                            let ctrl = u16::from(ev.state)
                                & u16::from(x11rb::protocol::xproto::KeyButMask::CONTROL)
                                != 0;
                            let ks = {
                                let setup = conn.setup();
                                if ev.detail < setup.min_keycode || ev.detail > setup.max_keycode { continue; }
                                let reply = conn.get_keyboard_mapping(ev.detail, 1)?.reply()?;
                                let idx = if shift { 1 } else { 0 };
                                if idx < reply.keysyms.len() && reply.keysyms[idx] != 0 {
                                    Some(reply.keysyms[idx])
                                } else if reply.keysyms.len() > 1 && reply.keysyms[1] != 0 {
                                    Some(reply.keysyms[1])
                                } else {
                                    None
                                }
                            };
                            if let Some(ks) = ks {
                                if let Some(bytes) = keysym_to_bytes(ks, shift, ctrl) {
                                    pty.write(&bytes)?;
                                }
                            }
                        }
                        Some(x11rb::protocol::Event::ClientMessage(ev)) => {
                            if ev.format == 32
                                && ev.type_ == wm_protocols
                                && ev.data.as_data32()[0] == wm_delete
                            {
                                running = false;
                            }
                        }
                        None => break,
                        _ => {}
                    }
                }
            }

            if libc::FD_ISSET(pty.fd(), &mut readfds) {
                let n = pty.read(&mut pty_buf)?;
                if n == 0 {
                    break;
                }
                for &byte in &pty_buf[..n] {
                    parser.advance(&mut term, byte, &mut pty);
                }
                need_redraw = true;
            }

            if need_redraw {
                renderer.render(&font, &term);
                renderer.draw(&conn, window, gc, depth)?;
                need_redraw = false;
            }
        }
    }

    conn.destroy_window(window)?;
    conn.flush()?;
    Ok(())
}
