use std::sync::Arc;

use x11rb::connection::Connection;
use x11rb::rust_connection::{DefaultStream, RustConnection};

use crate::x11::atoms::AtomCache;

#[cfg(unix)]
use std::os::unix::net::UnixStream;

#[derive(Clone)]
pub struct X11Connection {
    conn: Arc<RustConnection>,
    screen_num: usize,
    atoms: AtomCache,
    display_name: String,
}

impl X11Connection {
    pub fn connect() -> Result<Self, anyhow::Error> {
        let display_name = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string());
        let (conn, screen_num) = x11rb::connect(None)?;
        let conn = Arc::new(conn);

        let mut atoms = AtomCache::new();
        atoms.init(&*conn)?;

        Ok(Self { conn, screen_num, atoms, display_name })
    }

    /// Connect to an explicit Unix socket path. Needed on systems (e.g. Termux)
    /// where the X socket does not live under the conventional `/tmp/.X11-unix`.
    #[cfg(unix)]
    pub fn connect_to_socket(path: &str) -> Result<Self, anyhow::Error> {
        let display_name = display_from_socket(path);
        let stream = UnixStream::connect(path)
            .map_err(|e| anyhow::anyhow!("Failed to connect to X socket {}: {}", path, e))?;
        let (default_stream, _peer) = DefaultStream::from_unix_stream(stream)?;
        let conn = RustConnection::connect_to_stream(default_stream, 0)?;
        let conn = Arc::new(conn);
        let screen_num = 0usize;

        let mut atoms = AtomCache::new();
        atoms.init(&*conn)?;

        Ok(Self { conn, screen_num, atoms, display_name })
    }

    /// Connect honouring optional explicit display/socket overrides. The
    /// `-socket` path takes priority (Termux-style sockets), then `-display`,
    /// then the ambient `DISPLAY` environment.
    pub fn connect_with_opts(
        display: Option<&str>,
        socket: Option<&str>,
    ) -> Result<Self, anyhow::Error> {
        if let Some(s) = socket {
            Self::connect_to_socket(s)
        } else if let Some(d) = display {
            std::env::set_var("DISPLAY", d);
            Self::connect()
        } else {
            Self::connect()
        }
    }

    pub fn conn(&self) -> &RustConnection {
        &self.conn
    }

    pub fn screen_num(&self) -> usize {
        self.screen_num
    }

    pub fn screen(&self) -> &x11rb::protocol::xproto::Screen {
        &self.conn.setup().roots[self.screen_num]
    }

    pub fn root_window(&self) -> u32 {
        self.screen().root
    }

    /// The X display this connection is attached to (e.g. `:1`). Used when
    /// spawning external applications so they open on the same display the WM
    /// manages rather than falling back to the ambient `DISPLAY`.
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn atoms(&self) -> &AtomCache {
        &self.atoms
    }

    pub fn flush(&self) -> Result<(), anyhow::Error> {
        self.conn.flush()?;
        Ok(())
    }
}

/// Derive an X display string (e.g. `:1`) from a Unix socket path such as
/// `/tmp/.X11-unix/X1` or `/data/.../usr/tmp/.X11-unix/X1`.
fn display_from_socket(path: &str) -> String {
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("X0");
    if let Some(rest) = name.strip_prefix('X') {
        format!(":{}", rest)
    } else {
        ":0".to_string()
    }
}
