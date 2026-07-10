// fbrun - a small "run" dialog. Type a command and press Enter to execute it;
// Escape (or a click) dismisses it.
//
// Implements a minimal X11 text-entry widget: it captures KeyPress events,
// maps keycodes to keysyms via the server's keyboard mapping, renders the
// typed text with a core X11 font, and spawns `sh -c <input>` on Enter.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    self, ConnectionExt as _, CreateWindowAux, EventMask, InputFocus, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;

use fluxbox_rs::render::font::Font;
use fluxbox_rs::x11::X11Connection;

fn main() -> Result<(), anyhow::Error> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let args: Vec<String> = std::env::args().collect();
    let mut display: Option<String> = None;
    let mut socket: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-socket" => {
                if i + 1 < args.len() {
                    socket = Some(args[i + 1].clone());
                    i += 2;
                    continue;
                }
            }
            "-display" | "-d" => {
                if i + 1 < args.len() {
                    display = Some(args[i + 1].clone());
                    i += 2;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let conn = X11Connection::connect_with_opts(display.as_deref(), socket.as_deref())?;
    let screen = conn.screen();
    let root = screen.root;
    let w: u16 = 400;
    let h: u16 = 24;
    let x = ((screen.width_in_pixels as i32 - w as i32) / 2) as i16;
    let y: i16 = 24;

    let win = conn.conn().generate_id()?;
    conn.conn().create_window(
        0,
        win,
        root,
        x,
        y,
        w,
        h,
        1,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new()
            .override_redirect(1)
            .background_pixel(screen.white_pixel)
            .event_mask(EventMask::EXPOSURE | EventMask::KEY_PRESS | EventMask::BUTTON_PRESS),
    )?;
    conn.conn().map_window(win)?;

    let font = Font::load_x11_font(conn.conn(), "fixed").unwrap_or_else(|_| Font::new("fixed"));
    let gc = conn.conn().generate_id()?;
    conn.conn().create_gc(
        gc,
        win,
        &xproto::CreateGCAux::new()
            .foreground(screen.black_pixel)
            .background(screen.white_pixel),
    )?;

    // Resolve keycode -> keysym once via the server keyboard mapping.
    let setup = conn.conn().setup();
    let min_kc = setup.min_keycode;
    let max_kc = setup.max_keycode;
    let km = conn
        .conn()
        .get_keyboard_mapping(min_kc, (u16::from(max_kc) - u16::from(min_kc) + 1) as u8)?
        .reply()?;
    let per = km.keysyms_per_keycode as usize;
    let lookup = |kc: u8| -> u32 {
        let idx = (kc as usize - min_kc as usize) * per;
        km.keysyms.get(idx).copied().unwrap_or(0)
    };

    conn.conn().set_input_focus(InputFocus::NONE, win, CURRENT_TIME)?;

    let prompt = "Run:";
    let mut input = String::new();
    draw(&conn, win, gc, &font, screen.white_pixel, screen.black_pixel, h, &input, prompt)?;

    loop {
        let ev = conn.conn().wait_for_event()?;
        match ev {
            Event::Expose(_) => {
                draw(&conn, win, gc, &font, screen.white_pixel, screen.black_pixel, h, &input, prompt)?;
            }
            Event::KeyPress(kp) => {
                let ks = lookup(kp.detail);
                match ks {
                    0xFF0D => {
                        // Return: execute and exit.
                        run_command(&input);
                        break;
                    }
                    0xFF1B => break, // Escape
                    0xFF08 => {
                        input.pop();
                    } // BackSpace
                    0x20..=0x7e => input.push(ks as u8 as char),
                    _ => {}
                }
                draw(&conn, win, gc, &font, screen.white_pixel, screen.black_pixel, h, &input, prompt)?;
            }
            Event::ButtonPress(_) => break,
            _ => {}
        }
    }

    conn.conn().destroy_window(win)?;
    conn.flush()?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw(
    conn: &X11Connection,
    win: u32,
    gc: u32,
    font: &Font,
    _bg: u32,
    fg: u32,
    h: u16,
    input: &str,
    prompt: &str,
) -> Result<(), anyhow::Error> {
    let rust: &RustConnection = conn.conn();
    rust.clear_area(true, win, 0, 0, 0, 0)?;
    rust.change_gc(gc, &xproto::ChangeGCAux::new().foreground(fg).font(font.x_id()))?;

    let baseline = (h as i16 + font.height() as i16) / 2 - font.descent() as i16;
    font.draw_text(rust, win, gc, 4, baseline, prompt)?;
    let px = 4 + font.text_width(rust, prompt)? as i16 + 6;
    font.draw_text(rust, win, gc, px, baseline, input)?;
    rust.flush()?;
    Ok(())
}

fn run_command(cmd: &str) {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return;
    }
    match std::process::Command::new("sh").arg("-c").arg(cmd).spawn() {
        Ok(_) => {}
        Err(e) => eprintln!("Failed to run '{}': {}", cmd, e),
    }
}
