use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, GrabMode, ModMask, Keycode};

#[derive(Debug, Clone)]
pub enum KeyAction {
    NextWindow,
    PrevWindow,
    Close,
    Iconify,
    Maximize,
    Move,
    Resize,
    GoToWorkspace(usize),
    GoToNextWorkspace,
    GoToPrevWorkspace,
    ShowMenu,
    Exit,
    Exec(String),
}

#[derive(Debug, Clone)]
pub struct KeyBinding {
    pub modmask: ModMask,
    pub keycode: Keycode,
    pub action: KeyAction,
}

pub fn keysym_to_keycode<C: Connection>(conn: &C, keysym: u32) -> Option<u8> {
    let setup = conn.setup();
    let first = setup.min_keycode;
    let count = setup.max_keycode - first + 1;
    let reply = conn.get_keyboard_mapping(first, count).ok()?.reply().ok()?;
    let per = reply.keysyms_per_keycode as usize;
    for (i, chunk) in reply.keysyms.chunks(per).enumerate() {
        if chunk.contains(&keysym) {
            return Some(first + i as u8);
        }
    }
    None
}

mod keysyms {
    pub const TAB: u32 = 0xff09;
    pub const RETURN: u32 = 0xff0d;
    pub const ESCAPE: u32 = 0xff1b;
    pub const BACKSPACE: u32 = 0xff08;
    pub const DELETE: u32 = 0xffff;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const SPACE: u32 = 0xff20;
    pub const F1: u32 = 0xffbe;
    pub const F2: u32 = 0xffbf;
    pub const F3: u32 = 0xffc0;
    pub const F4: u32 = 0xffc1;
    pub const F5: u32 = 0xffc2;
    pub const F6: u32 = 0xffc3;
    pub const F7: u32 = 0xffc4;
    pub const F8: u32 = 0xffc5;
    pub const F9: u32 = 0xffc6;
    pub const F10: u32 = 0xffc7;
    pub const F11: u32 = 0xffc8;
    pub const F12: u32 = 0xffc9;
}

pub fn parse_key_name(name: &str) -> Option<u32> {
    use keysyms::*;
    let upper = name.to_uppercase();
    match upper.as_str() {
        "TAB" => Some(TAB),
        "RETURN" | "ENTER" => Some(RETURN),
        "ESCAPE" | "ESC" => Some(ESCAPE),
        "BACKSPACE" | "BACK_SPACE" => Some(BACKSPACE),
        "DELETE" | "DEL" => Some(DELETE),
        "LEFT" => Some(LEFT),
        "RIGHT" => Some(RIGHT),
        "UP" => Some(UP),
        "DOWN" => Some(DOWN),
        "SPACE" => Some(SPACE),
        "F1" => Some(F1),
        "F2" => Some(F2),
        "F3" => Some(F3),
        "F4" => Some(F4),
        "F5" => Some(F5),
        "F6" => Some(F6),
        "F7" => Some(F7),
        "F8" => Some(F8),
        "F9" => Some(F9),
        "F10" => Some(F10),
        "F11" => Some(F11),
        "F12" => Some(F12),
        _ => {
            let c = name.chars().next()?;
            if c.is_ascii_alphanumeric() {
                Some(c as u32)
            } else {
                None
            }
        }
    }
}

pub fn parse_action(action_str: &str) -> Option<KeyAction> {
    let upper = action_str.to_uppercase();
    let parts: Vec<&str> = upper.splitn(2, ' ').collect();
    match parts[0] {
        "NEXTWINDOW" => Some(KeyAction::NextWindow),
        "PREVWINDOW" | "PREVIOUSWINDOW" => Some(KeyAction::PrevWindow),
        "CLOSE" => Some(KeyAction::Close),
        "ICONIFY" | "MINIMIZE" => Some(KeyAction::Iconify),
        "MAXIMIZE" | "MAXIMIZEWINDOW" | "FULLSCREEN" => Some(KeyAction::Maximize),
        "MOVEWINDOW" | "MOVE" => Some(KeyAction::Move),
        "RESIZEWINDOW" | "RESIZE" => Some(KeyAction::Resize),
        "GOTOWORKSPACE" | "WORKSPACE" => {
            parts.get(1).and_then(|n| n.parse::<usize>().ok()).map(KeyAction::GoToWorkspace)
        }
        "NEXTWORKSPACE" | "GOTONEXTWORKSPACE" => Some(KeyAction::GoToNextWorkspace),
        "PREVWORKSPACE" | "GOTOPREVWORKSPACE" => Some(KeyAction::GoToPrevWorkspace),
        "SHOWMENU" | "ROOTMENU" => Some(KeyAction::ShowMenu),
        "EXIT" | "QUIT" => Some(KeyAction::Exit),
        "EXEC" | "EXECUTE" => {
            let cmd = action_str.splitn(2, ' ').nth(1).unwrap_or("").trim().to_string();
            Some(KeyAction::Exec(cmd))
        }
        _ => None,
    }
}

/// Bits of "lock" modifiers that vary independently of a shortcut itself
/// (CapsLock = 0x02, NumLock/Mod2 = 0x10).  These must be masked out when
/// comparing a key event's state, and grabbed for every combination so the
/// shortcut fires regardless of CapsLock/NumLock state.
pub const LOCK_MASK: u16 = 0x0002 | 0x0010;

/// Combine two ModMask values into one (bitwise OR).
pub fn combine_modmask(a: ModMask, b: ModMask) -> ModMask {
    ModMask::from(u16::from(a) | u16::from(b))
}

/// Load keybindings from a Fluxbox-format keys file.
pub fn load_keys_file(path: &str) -> Vec<(ModMask, String, KeyAction)> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut bindings = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        // Format: Mod1 Tab :Action
        if let Some(rest) = line.split_once(':') {
            let key_part = rest.0.trim();
            let action_part = rest.1.trim();

            let mut modmask = ModMask::from(0u16);
            let mut key_name = String::new();

            // Parse modifiers
            for part in key_part.split_whitespace() {
                match part {
                    "Mod1" | "Alt" => modmask = combine_modmask(modmask, ModMask::M1),
                    "Mod2" => modmask = combine_modmask(modmask, ModMask::M2),
                    "Mod3" => modmask = combine_modmask(modmask, ModMask::M3),
                    "Mod4" | "Super" | "Win" => modmask = combine_modmask(modmask, ModMask::M4),
                    "Mod5" => modmask = combine_modmask(modmask, ModMask::M5),
                    "Control" | "Ctrl" => modmask = combine_modmask(modmask, ModMask::CONTROL),
                    "Shift" => modmask = combine_modmask(modmask, ModMask::SHIFT),
                    _ => key_name = part.to_string(),
                }
            }

            if let Some(action) = parse_action(action_part) {
                bindings.push((modmask, key_name, action));
            }
        }
    }
    bindings
}

/// Build the default Fluxbox-like keybindings.
pub fn default_bindings() -> Vec<(ModMask, String, KeyAction)> {
    vec![
        (ModMask::M1, "Tab".to_string(), KeyAction::NextWindow),
        (combine_modmask(ModMask::M1, ModMask::SHIFT), "Tab".to_string(), KeyAction::PrevWindow),
        (ModMask::M1, "F4".to_string(), KeyAction::Close),
        (ModMask::M1, "F9".to_string(), KeyAction::Iconify),
        (ModMask::M1, "F10".to_string(), KeyAction::Maximize),
        (ModMask::M1, "Left".to_string(), KeyAction::GoToPrevWorkspace),
        (ModMask::M1, "Right".to_string(), KeyAction::GoToNextWorkspace),
        (ModMask::M1, "F1".to_string(), KeyAction::GoToWorkspace(1)),
        (ModMask::M1, "F2".to_string(), KeyAction::GoToWorkspace(2)),
        (ModMask::M1, "F3".to_string(), KeyAction::GoToWorkspace(3)),
        (ModMask::M1, "F4".to_string(), KeyAction::GoToWorkspace(4)),
        (ModMask::M1, "space".to_string(), KeyAction::ShowMenu),
    ]
}

/// Register key grabs on the root window for all bindings.
pub fn apply_bindings<C: Connection>(
    conn: &C,
    root: u32,
    bindings: &[(ModMask, String, KeyAction)],
) -> Result<Vec<KeyBinding>, anyhow::Error> {
    let mut resolved = Vec::new();
    for (modmask, key_name, action) in bindings {
        let keysym = match parse_key_name(key_name) {
            Some(s) => s,
            None => {
                log::warn!("Unknown key name: {}", key_name);
                continue;
            }
        };
        let keycode = match keysym_to_keycode(conn, keysym) {
            Some(kc) => kc,
            None => {
                log::warn!("Cannot find keycode for keysym {} ({})", keysym, key_name);
                continue;
            }
        };
        resolved.push(KeyBinding {
            modmask: *modmask,
            keycode,
            action: action.clone(),
        });

        // XGrabKey requires an exact modifier bit match — there is no
        // partial wildcard.  A binding must therefore be grabbed for every
        // combination of "lock" modifiers (CapsLock, NumLock) that might be
        // active independently of the shortcut itself.  This is what i3, dwm
        // and openbox all do.  Without these extra grabs a shortcut only
        // fires when NumLock happens to be in the exact state of the single
        // grab registered below.
        let base = u16::from(*modmask);
        for extra in [0u16, 0x0002, 0x0010, 0x0012] {
            let combined = ModMask::from(base | extra);
            let _ = conn.grab_key(
                false,
                root,
                combined,
                keycode,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            );
        }
    }
    Ok(resolved)
}

/// Resolve key names to keycodes (without registering grabs).
pub fn resolve_bindings<C: Connection>(
    conn: &C,
    bindings: &[(ModMask, String, KeyAction)],
) -> Vec<KeyBinding> {
    let mut resolved = Vec::new();
    for (modmask, key_name, action) in bindings {
        let keysym = match parse_key_name(key_name) {
            Some(s) => s,
            None => continue,
        };
        let keycode = match keysym_to_keycode(conn, keysym) {
            Some(kc) => kc,
            None => continue,
        };
        resolved.push(KeyBinding {
            modmask: *modmask,
            keycode,
            action: action.clone(),
        });
    }
    resolved
}


/// Look up a key press event against a list of bindings.
pub fn match_key<'a>(
    bindings: &'a [KeyBinding],
    state: u16,
    detail: u8,
) -> Option<&'a KeyAction> {
    // Ignore only the lock modifiers (CapsLock/NumLock) — keep Mod3/Mod4/Mod5
    // so Super/Win (Mod4) and other custom modifiers are matched correctly.
    let state_masked = state & !LOCK_MASK;
    for b in bindings {
        if b.keycode == detail && u16::from(b.modmask) & !LOCK_MASK == state_masked {
            return Some(&b.action);
        }
    }
    None
}
