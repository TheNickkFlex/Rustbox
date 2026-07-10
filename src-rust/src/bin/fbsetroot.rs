// fbsetroot - set the root window background (solid colour, gradient, or mod).

use x11rb::protocol::xproto::{ChangeWindowAttributesAux, ConnectionExt as _, Pixmap};

use fluxbox_rs::core::parse_color;
use fluxbox_rs::core::Rectangle;
use fluxbox_rs::render::texture::{GradientType, Texture, TextureRender};
use fluxbox_rs::x11::X11Connection;

fn main() -> Result<(), anyhow::Error> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let args: Vec<String> = std::env::args().collect();
    let mut texture = Texture::new();
    texture.color = parse_color("#333333").unwrap_or_default();
    texture.color_to = parse_color("#cccccc").unwrap_or_default();
    texture.gradient = GradientType::Vertical;
    texture.bevel_width = 0;

    let mut display: Option<String> = None;
    let mut socket: Option<String> = None;
    let mut i = 1;
    let mut mode_set = false;

    while i < args.len() {
        match args[i].as_str() {
            "-socket" => {
                if i + 1 < args.len() {
                    socket = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "-display" | "-d" => {
                if i + 1 < args.len() {
                    display = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "-solid" => {
                if i + 1 < args.len() {
                    if let Some(c) = parse_color(&args[i + 1]) {
                        texture.color = c.clone();
                        texture.color_to = c;
                        texture.gradient = GradientType::None;
                    }
                    mode_set = true;
                }
                i += 1;
            }
            "-gradient" => {
                if i + 1 < args.len() {
                    if let Some(g) = gradient_from_name(&args[i + 1]) {
                        texture.gradient = g;
                    }
                    mode_set = true;
                }
                i += 1;
            }
            "-from" => {
                if i + 1 < args.len() {
                    if let Some(c) = parse_color(&args[i + 1]) {
                        texture.color = c;
                    }
                }
                i += 1;
            }
            "-to" => {
                if i + 1 < args.len() {
                    if let Some(c) = parse_color(&args[i + 1]) {
                        texture.color_to = c;
                    }
                }
                i += 1;
            }
            "-mod" => {
                // Simple mod/pattern is approximated by a flat colour here.
                if i + 2 < args.len() {
                    mode_set = true;
                }
                i += 2;
            }
            "-help" | "--help" => {
                println!("fbsetroot [-solid <color>] [-gradient <type>] [-from <c>] [-to <c>] [-display <d>]");
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown option: {}", other);
            }
        }
        i += 1;
    }

    if !mode_set {
        // Default to a vertical gradient if nothing was specified.
        texture.gradient = GradientType::Vertical;
    }

    let conn = X11Connection::connect_with_opts(display.as_deref(), socket.as_deref())?;

    let screen = conn.screen();
    let root = screen.root;
    let depth = screen.root_depth;
    let (w, h) = (screen.width_in_pixels, screen.height_in_pixels);

    let pixmap = TextureRender::render_gradient(
        &conn,
        &texture,
        &Rectangle::new(0, 0, w, h),
        depth,
    )?;

    conn.conn().change_window_attributes(
        root,
        &ChangeWindowAttributesAux::new()
            .background_pixmap(Pixmap::from(pixmap)),
    )?;
    conn.conn().clear_area(true, root, 0, 0, 0, 0)?;
    conn.flush()?;

    println!("Root background updated.");
    Ok(())
}

fn gradient_from_name(name: &str) -> Option<GradientType> {
    Some(match name.to_lowercase().as_str() {
        "horizontal" => GradientType::Horizontal,
        "vertical" => GradientType::Vertical,
        "diagonal" => GradientType::Diagonal,
        "crossdiagonal" => GradientType::CrossDiagonal,
        "rectangle" => GradientType::Rectangle,
        "pyramid" => GradientType::Pyramid,
        "pipecross" => GradientType::PipeCross,
        "elliptic" => GradientType::Elliptic,
        "mirrorhorizontal" => GradientType::MirrorHorizontal,
        "mirrorvertical" => GradientType::MirrorVertical,
        _ => return None,
    })
}
