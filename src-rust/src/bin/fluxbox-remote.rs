// fluxbox-remote - send a command to a running Fluxbox-rs instance.
//
// Sends the command to the root window as one or more ClientMessage events of
// type "Fluxbox/remote" (format 8, up to 20 bytes per message), terminated by
// a NUL byte. The WM accumulates the payload and executes it on the terminator.

use x11rb::protocol::xproto::{ConnectionExt as _, PropMode};

use fluxbox_rs::x11::{Atom, X11Connection};

fn main() -> Result<(), anyhow::Error> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let args: Vec<String> = std::env::args().collect();
    let mut display: Option<String> = None;
    let mut socket: Option<String> = None;
    let mut rest: Vec<String> = Vec::new();
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
            other => rest.push(other.to_string()),
        }
        i += 1;
    }

    if rest.is_empty() {
        eprintln!("Usage: fluxbox-remote [-socket <path>] [-display <d>] <command>");
        eprintln!("Commands: restart, quit, reconfig, setworkspace <n>");
        std::process::exit(1);
    }
    let command = rest.join(" ");

    let conn = X11Connection::connect_with_opts(display.as_deref(), socket.as_deref())?;
    let root = conn.root_window();
    let cmd_atom = conn.atoms().get(Atom::FluxboxRemoteCmd);
    if cmd_atom == x11rb::NONE {
        anyhow::bail!("FLUXBOX_REMOTE_CMD atom not available");
    }
    let utf8 = conn.atoms().get(Atom::Utf8String);
    if utf8 == x11rb::NONE {
        anyhow::bail!("UTF8_STRING atom not available");
    }

    // Write the command into a root-window property. The WM selects
    // PropertyChange on the root, so it receives a PropertyNotify and reads
    // the command back. (More reliable than a synthetic ClientMessage on
    // servers such as termux-x11 that don't route ClientMessages to root.)
    let payload = command.clone();
    conn.conn()
        .change_property(
            PropMode::REPLACE,
            root,
            cmd_atom,
            utf8,
            8,
            payload.as_bytes().len() as u32,
            payload.as_bytes(),
        )?;
    conn.flush()?;
    println!("Sent: {}", command);
    Ok(())
}
