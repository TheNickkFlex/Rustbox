// Rustbox-rs - Rust port of the Rustbox window manager
// Optimized for performance, safety, and modern X11 practices
//
// Key optimizations over C version:
// - x11rb protocol layer (batched requests, no Xlib overhead)
// - Deferred rendering: X requests are coalesced and flushed once per event
//   batch instead of round-tripping on every change
// - O(1) window lookups via HashMap
// - Type-safe atom system
// - Zero-cost error handling via Result types
// - No global state - explicit dependency injection

use x11rb::protocol::xproto::{ChangeWindowAttributesAux, EventMask, ConnectionExt as _};

fn main() -> Result<(), anyhow::Error> {
    // Capture panics to a file so crashes are diagnosable on-device.
    let _ = std::panic::set_hook(Box::new(|info| {
        use std::io::Write;
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let path = format!("{}/.rustbox/panic.log", home);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(f, "PANIC: {}", info);
            if let Some(loc) = info.location() {
                let _ = writeln!(f, "  at {}:{}", loc.file(), loc.line());
            }
        }
        eprintln!("PANIC: {}", info);
    }));

    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info")
    )
    .format_timestamp_secs()
    .init();

    rustbox_rs::mods::init_all();

    log::debug!("DEBUG logging active — env_logger initialized");

    log::info!("Rustbox-rs 1.4.0 starting...");

    let args: Vec<String> = std::env::args().collect();
    let (display_name, config_dir, socket_path) = parse_cli_args(&args);

    log::info!("Display: {}", display_name);
    log::info!("Config dir: {}", config_dir);

    let xconn = match socket_path {
        Some(socket) => {
            log::info!("Connecting via explicit socket: {}", socket);
            rustbox_rs::x11::X11Connection::connect_to_socket(&socket)?
        }
        None => {
            // Ensure DISPLAY is set in environment for x11rb::connect(None).
            std::env::set_var("DISPLAY", &display_name);
            rustbox_rs::x11::X11Connection::connect()?
        }
    };
    log::info!("Connected to X11 server");

    // Make spawned applications open on the display we actually manage (the
    // socket-derived one), not the ambient DISPLAY that may point elsewhere.
    // Also drop WAYLAND_DISPLAY and XDG_SESSION_TYPE so Vulkan/GL apps (notably
    // Chromium/Brave's Ozone) don't hijack to a Wayland session and bypass this
    // X11 display.
    std::env::set_var("DISPLAY", xconn.display_name().to_string());
    std::env::remove_var("WAYLAND_DISPLAY");
    std::env::remove_var("XDG_SESSION_TYPE");

    become_wm(&xconn)?;
    log::info!("Became window manager");

    let mut rustbox = rustbox_rs::event::Rustbox::new(
        xconn,
        &display_name,
        &config_dir,
    )?;
    log::info!("Rustbox initialized");

    // Force glibc to release all freed heap pages from the entire initialization
    // phase (font database scanning, wallpaper decoding, and scaling) back to the OS.
    #[cfg(target_os = "linux")]
    unsafe {
        libc::malloc_trim(0);
    }

    log::info!("Entering main event loop");
    if let Err(e) = rustbox.event_loop() {
        log::error!("Event loop terminou com erro: {:?}", e);
        if let Ok(home) = std::env::var("HOME") {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(format!("{}/.rustbox/startup.log", home))
            {
                use std::io::Write;
                let _ = writeln!(f, "[startup] FATAL event_loop error: {:?}", e);
                let _ = f.flush();
            }
        }
        return Err(e);
    }

    log::info!("Rustbox shutting down");
    Ok(())
}

fn parse_cli_args(args: &[String]) -> (String, String, Option<String>) {
    let mut display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string());
    let mut config_dir = String::new();
    let mut socket_path = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-display" | "-d" => {
                if i + 1 < args.len() {
                    display = args[i + 1].clone();
                    i += 1;
                }
            }
            "-socket" => {
                if i + 1 < args.len() {
                    socket_path = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "-rc" => {
                if i + 1 < args.len() {
                    config_dir = args[i + 1].clone();
                    i += 1;
                }
            }
            "-help" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "-version" | "--version" => {
                println!("Rustbox-rs 1.4.0");
                std::process::exit(0);
            }
            "-info" => {
                print_info();
                std::process::exit(0);
            }
            _ => {}
        }
        i += 1;
    }

    if config_dir.is_empty() {
        config_dir = get_default_config_dir();
    }

    (display, config_dir, socket_path)
}

fn get_default_config_dir() -> String {
    if let Ok(home) = std::env::var("HOME") {
        format!("{}/.rustbox", home)
    } else {
        "/etc/rustbox".to_string()
    }
}

fn become_wm(xconn: &rustbox_rs::x11::X11Connection) -> Result<(), anyhow::Error> {
    let screen = xconn.screen();
    let root = screen.root;

    let change = ChangeWindowAttributesAux::default()
        .event_mask(EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY);

    match xconn.conn().change_window_attributes(root, &change)?.check() {
        Ok(()) => {
            xconn.conn().change_window_attributes(
                root,
                &ChangeWindowAttributesAux::default()
                    .event_mask(
                        EventMask::SUBSTRUCTURE_REDIRECT
                            | EventMask::SUBSTRUCTURE_NOTIFY
                            | EventMask::PROPERTY_CHANGE
                            | EventMask::BUTTON_PRESS
                            | EventMask::BUTTON_RELEASE
                            | EventMask::KEY_PRESS
                            | EventMask::KEY_RELEASE
                            | EventMask::POINTER_MOTION
                            | EventMask::ENTER_WINDOW
                            | EventMask::LEAVE_WINDOW,
                    ),
            )?;
            xconn.flush()?;
            Ok(())
        }
        Err(e) => {
            log::error!("Cannot become window manager: {}", e);
            log::error!("Is another window manager running?");
            std::process::exit(1);
        }
    }
}

fn print_usage() {
    println!("Rustbox-rs 1.4.0");
    println!("Usage: rustbox [options]");
    println!("Options:");
    println!("  -display <display>   X11 display to manage (default: $DISPLAY)");
    println!("  -rc <file>           Alternative init file");
    println!("  -help                Display this help");
    println!("  -version             Display version");
    println!("  -info                Display build information");
}

fn print_info() {
    println!("Rustbox-rs 1.4.0 - Rust port of Rustbox window manager");
    println!();
    println!("Build features:");
    #[cfg(feature = "xrender")] println!("  XRender: enabled");
    #[cfg(feature = "xinerama")] println!("  Xinerama: enabled");
    #[cfg(feature = "xrandr")] println!("  XRandR: enabled");
    #[cfg(feature = "composite")] println!("  Composite: enabled");
}
