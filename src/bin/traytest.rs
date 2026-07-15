/// Minimal system tray test: docks a coloured window and keeps it alive.
/// Usage: cargo run --bin traytest
use x11rb::connection::Connection;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    use x11rb::protocol::xproto::ConnectionExt as _;

    let tray_atom = conn.intern_atom(false, b"_NET_SYSTEM_TRAY_S0")?.reply()?;
    let owner = conn.get_selection_owner(tray_atom.atom)?.reply()?.owner;
    if owner == x11rb::NONE {
        eprintln!("No system tray owner found");
        return Ok(());
    }
    eprintln!("Tray owner window: {:#x}", owner);

    let win = conn.generate_id()?;
    conn.create_window(
        0, win, screen.root,
        0, 0, 24, 24, 0,
        x11rb::protocol::xproto::WindowClass::INPUT_OUTPUT,
        0,
        &x11rb::protocol::xproto::CreateWindowAux::new()
            .background_pixel(0x0000ff)
            .override_redirect(1),
    )?;
    conn.map_window(win)?;
    conn.flush()?;

    use x11rb::protocol::xproto::{ClientMessageData, ClientMessageEvent};
    let opcode_atom = conn.intern_atom(false, b"_NET_SYSTEM_TRAY_OPCODE")?.reply()?;
    let data = ClientMessageData::from([0xffffffffu32, 0u32, win, 0, 0]);
    let ev = ClientMessageEvent::new(32, owner, opcode_atom.atom, data);
    conn.send_event(false, owner, x11rb::protocol::xproto::EventMask::NO_EVENT, &ev)?;
    conn.flush()?;
    eprintln!("Dock request sent for window {:#x}", win);

    std::thread::sleep(std::time::Duration::from_secs(10));
    Ok(())
}
