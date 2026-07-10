// Fluxbox-rs - Rust port of the Fluxbox window manager
// Optimized for performance, safety, and modern X11 practices
//
// Key optimizations over C version:
// - x11rb protocol layer (batched requests, no Xlib overhead)
// - Damage-pass rendering (only redraws changed regions)
// - O(1) window lookups via HashMap
// - Type-safe atom system
// - Modern gradient rendering (SIMD-friendly pixel generation)
// - Zero-cost error handling via Result types
// - No global state - explicit dependency injection

use x11rb::protocol::xproto::{ChangeWindowAttributesAux, EventMask, ConnectionExt as _};

fn main() -> Result<(), anyhow::Error> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info")
    )
    .format_timestamp_secs()
    .init();

    log::info!("Fluxbox-rs 1.4.0 starting...");

    let args: Vec<String> = std::env::args().collect();
    let (display_name, config_dir, socket_path) = parse_cli_args(&args);

    log::info!("Display: {}", display_name);
    log::info!("Config dir: {}", config_dir);

    let xconn = match socket_path {
        Some(socket) => {
            log::info!("Connecting via explicit socket: {}", socket);
            fluxbox_rs::x11::X11Connection::connect_to_socket(&socket)?
        }
        None => fluxbox_rs::x11::X11Connection::connect()?,
    };
    log::info!("Connected to X11 server");

    become_wm(&xconn)?;
    log::info!("Became window manager");

    let mut fluxbox = fluxbox_rs::event::Fluxbox::new(
        xconn,
        &display_name,
        &config_dir,
    )?;
    log::info!("Fluxbox initialized");

    log::info!("Entering main event loop");
    fluxbox.event_loop()?;

    log::info!("Fluxbox shutting down");
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
                println!("Fluxbox-rs 1.4.0");
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
        format!("{}/.fluxbox", home)
    } else {
        "/etc/fluxbox".to_string()
    }
}

fn become_wm(xconn: &fluxbox_rs::x11::X11Connection) -> Result<(), anyhow::Error> {
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
    println!("Fluxbox-rs 1.4.0");
    println!("Usage: fluxbox [options]");
    println!("Options:");
    println!("  -display <display>   X11 display to manage (default: $DISPLAY)");
    println!("  -rc <file>           Alternative init file");
    println!("  -help                Display this help");
    println!("  -version             Display version");
    println!("  -info                Display build information");
}

fn print_info() {
    println!("Fluxbox-rs 1.4.0 - Rust port of Fluxbox window manager");
    println!();
    println!("Build features:");
    #[cfg(feature = "xrender")] println!("  XRender: enabled");
    #[cfg(feature = "xinerama")] println!("  Xinerama: enabled");
    #[cfg(feature = "xrandr")] println!("  XRandR: enabled");
    #[cfg(feature = "composite")] println!("  Composite: enabled");
}
