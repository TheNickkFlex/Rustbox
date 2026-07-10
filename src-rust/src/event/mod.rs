use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;

use crate::command::CommandRegistry;
use crate::screen::BScreen;
use crate::x11::{Atom, Event, X11Connection};

pub struct Fluxbox {
    conn: X11Connection,
    screens: Vec<BScreen>,
    running: Arc<AtomicBool>,
    command_registry: CommandRegistry,
    display_name: String,
    config_dir: String,
    restart: bool,
    exit_code: i32,
    remote_buffer: String,
}

impl Fluxbox {
    pub fn new(conn: X11Connection, display_name: &str, config_dir: &str) -> Result<Self, anyhow::Error> {
        let mut fluxbox = Self {
            conn,
            screens: Vec::new(),
            running: Arc::new(AtomicBool::new(true)),
            command_registry: CommandRegistry::new(),
            display_name: display_name.to_string(),
            config_dir: config_dir.to_string(),
            restart: false,
            exit_code: 0,
            remote_buffer: String::new(),
        };

        fluxbox.init_screens()?;
        Ok(fluxbox)
    }

    fn init_screens(&mut self) -> Result<(), anyhow::Error> {
        use x11rb::protocol::xproto::{ChangeWindowAttributesAux, EventMask};

        let bscreen = BScreen::new(0, self.conn.clone(), "default")?;

        self.conn.conn().change_window_attributes(
            self.conn.root_window(),
            &ChangeWindowAttributesAux::new()
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

        self.screens.push(bscreen);
        self.conn.flush()?;
        Ok(())
    }

    pub fn conn(&self) -> &X11Connection {
        &self.conn
    }

    pub fn screen(&self, num: usize) -> Option<&BScreen> {
        self.screens.get(num)
    }

    pub fn screen_mut(&mut self, num: usize) -> Option<&mut BScreen> {
        self.screens.get_mut(num)
    }

    pub fn current_screen(&self) -> &BScreen {
        &self.screens[0]
    }

    pub fn current_screen_mut(&mut self) -> &mut BScreen {
        &mut self.screens[0]
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn command_registry(&self) -> &CommandRegistry {
        &self.command_registry
    }

    pub fn should_restart(&self) -> bool {
        self.restart
    }

    pub fn set_restart(&mut self, restart: bool) {
        self.restart = restart;
    }

    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }

    pub fn shutdown(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }

    pub fn handle_event(&mut self, event: &Event) -> Result<(), anyhow::Error> {
        // Remote commands (fluxbox-remote) are addressed to the root window
        // and handled globally rather than per-screen.
        if let Event::ClientMessage(e) = event {
            let remote = self.conn.atoms().get(Atom::FluxboxRemote);
            if e.type_ == remote {
                let data = e.data.as_data8();
                for &b in &data {
                    if b == 0 {
                        break;
                    }
                    self.remote_buffer.push(b as char);
                }
                // A NUL byte terminates the command; flush and execute.
                if data.contains(&0) || self.remote_buffer.len() >= 1024 {
                    let cmd = std::mem::take(&mut self.remote_buffer);
                    self.handle_remote_command(cmd.trim());
                }
            }
        }

        // Property-based remote command (reliable on servers that do not
        // deliver synthetic ClientMessages to the root window, e.g. termux-x11).
        if let Event::PropertyNotify(e) = event {
            let cmd_atom = self.conn.atoms().get(Atom::FluxboxRemoteCmd);
            if cmd_atom != x11rb::NONE
                && e.window == self.conn.root_window()
                && e.atom == cmd_atom
            {
                if let Some(cmd) = self.read_remote_property() {
                    let cmd = cmd.trim().to_string();
                    if !cmd.is_empty() {
                        self.handle_remote_command(&cmd);
                    }
                }
            }
        }

        for screen in &mut self.screens {
            screen.handle_event(event)?;
        }
        Ok(())
    }

    /// Read the `FLUXBOX_REMOTE_CMD` property from the root window (set by
    /// `fluxbox-remote`) and return its text payload.
    fn read_remote_property(&self) -> Option<String> {
        let cmd_atom = self.conn.atoms().get(Atom::FluxboxRemoteCmd);
        if cmd_atom == x11rb::NONE {
            return None;
        }
        let cookie = self
            .conn
            .conn()
            .get_property(false, self.conn.root_window(), cmd_atom, x11rb::NONE, 0, 4096)
            .ok()?;
        let reply = cookie.reply().ok()?;
        String::from_utf8(reply.value).ok()
    }

    /// Process a command received from `fluxbox-remote`. Only a fixed set of
    /// known commands is honoured (matching upstream behaviour).
    pub fn handle_remote_command(&mut self, cmd: &str) {
        log::info!("Remote command: {}", cmd);
        match cmd {
            "restart" | "restartwm" => {
                self.set_restart(true);
                self.shutdown();
            }
            "quit" | "exit" => {
                self.shutdown();
            }
            "reconfig" | "reconfigure" => {
                if let Err(e) = self.reconfigure() {
                    log::warn!("Reconfigure failed: {}", e);
                }
            }
            _ if cmd.starts_with("setworkspace") => {
                if let Some(arg) = cmd.split_whitespace().nth(1) {
                    if let Ok(n) = arg.parse::<u32>() {
                        if let Some(screen) = self.screen_mut(0) {
                            screen.set_current_workspace(n.saturating_sub(1));
                        }
                    }
                }
            }
            _ => {
                log::warn!("Unknown remote command: {}", cmd);
            }
        }
    }

    pub fn event_loop(&mut self) -> Result<(), anyhow::Error> {
        self.conn.flush()?;

        let mut last_clock_tick = std::time::Instant::now();
        const CLOCK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

        while self.is_running() {
            // Drain every already-queued event first so we never block while
            // work is pending. Handlers issue X requests of their own; we flush
            // once after the whole batch to coalesce them into a single write.
            while let Some(raw) = self.conn.conn().poll_for_event()? {
                if let Some(e) = Event::from_x11(raw) {
                    self.handle_event(&e)?;
                }
            }
            self.conn.flush()?;

            // Periodic clock redraw — the toolbar clock only updates on render,
            // and no X event arrives just because the second changed.
            if last_clock_tick.elapsed() >= CLOCK_INTERVAL {
                for screen in &mut self.screens {
                    let _ = screen.toolbar_render(&self.conn);
                }
                self.conn.flush()?;
                last_clock_tick = std::time::Instant::now();
            }

            // Block for the next event only when the queue is empty.
            // Use a short sleep loop so we wake up periodically for the clock
            // tick even when no X events arrive (wait_for_event would block
            // indefinitely).
            loop {
                if let Some(raw) = self.conn.conn().poll_for_event()? {
                    if let Some(e) = Event::from_x11(raw) {
                        self.handle_event(&e)?;
                    }
                    self.conn.flush()?;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
                if last_clock_tick.elapsed() >= CLOCK_INTERVAL {
                    for screen in &mut self.screens {
                        let _ = screen.toolbar_render(&self.conn);
                    }
                    self.conn.flush()?;
                    last_clock_tick = std::time::Instant::now();
                }
                if !self.is_running() {
                    break;
                }
            }
        }

        // Clean up docked UI before exiting.
        for screen in &mut self.screens {
            if let Err(e) = screen.destroy() {
                log::warn!("Error destroying screen resources: {}", e);
            }
        }

        Ok(())
    }

    pub fn reconfigure(&mut self) -> Result<(), anyhow::Error> {
        log::info!("Reconfiguring Fluxbox...");
        for screen in &mut self.screens {
            screen.reconfigure(&self.conn)?;
        }
        Ok(())
    }
}
