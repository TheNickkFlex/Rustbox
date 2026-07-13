use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, EventMask, WindowClass, CreateWindowAux, ClientMessageEvent, ClientMessageData};
use std::{thread, time::Duration};
fn main() {
    let (conn, _) = x11rb::connect(Some(":1")).unwrap();
    let root = conn.setup().roots[0].root;
    let op = conn.intern_atom(false, b"_NET_SYSTEM_TRAY_OPCODE").unwrap().reply().unwrap().atom;
    let icon = conn.generate_id().unwrap();
    conn.create_window(0, icon, root, 0,0, 24,24, 0, WindowClass::INPUT_OUTPUT, 0,
        &CreateWindowAux::new().background_pixel(0xFF0000)).unwrap();
    conn.map_window(icon).unwrap();
    conn.flush().unwrap();
    let ev = ClientMessageEvent::new(32, root, op, ClientMessageData::from([0u32,0u32,icon,0,0]));
    conn.send_event(false, root, EventMask::SUBSTRUCTURE_NOTIFY, &ev).unwrap();
    conn.flush().unwrap();
    eprintln!("DOCKED icon=0x{:x}", icon);
    thread::sleep(Duration::from_secs(30));
}
