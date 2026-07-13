//! StatusNotifierItem (SNI) support — the "modern" system tray protocol used
//! by apps such as Discord, Telegram and many Electron/Qt applications.
//!
//! Unlike the legacy `_NET_SYSTEM_TRAY` (XEmbed) protocol handled in
//! `crate::tray`, SNI is purely D-Bus: there is no X11 embedding. A tray app
//! announces itself by calling `RegisterStatusNotifierItem` on a session-wide
//! singleton service, `org.kde.StatusNotifierWatcher`, and exposes its icon as
//! ARGB32 pixels (the `IconPixmap` property) that the host (us) renders.
//!
//! Because minimal window managers normally do not ship a `StatusNotifierWatcher`
//! (that usually comes from KDE/GNOME), Rustbox implements the Watcher itself,
//! so apps actually have somewhere to register.
//!
//! Integration with the synchronous X11 event loop
//! ------------------------------------------------
//! zbus is async, while the WM's main loop is a blocking `poll(2)` on the X11
//! fd. The SNI connection therefore runs on its own thread (driven by the
//! `smol` executor). Every time something changes it sends an `SniEvent` over
//! an `mpsc` channel and pokes a self-pipe; the main loop adds that pipe's read
//! end to its `poll()` set and, when woken, drains the channel and updates the
//! tray — exactly like it already does for XEmbed icons.

use std::os::unix::io::RawFd;
use std::sync::mpsc::{self, RecvTimeoutError, Receiver, Sender};
use std::thread;
use std::time::Duration;

use zbus::{
    connection, fdo, interface, object_server::SignalEmitter, proxy, Connection,
};

/// Events delivered from the SNI thread to the WM main loop.
///
/// `Updated` carries the raw ARGB32 pixels of the item's icon so the main
/// thread can build an X11 pixmap without touching D-Bus itself.
#[derive(Clone)]
pub enum SniEvent {
    Registered { service: String },
    Unregistered { service: String },
    Updated {
        service: String,
        width: u32,
        height: u32,
        argb: Vec<u8>,
    },
}

/// A request from the WM (a tray click) to the SNI thread, which owns the
/// per-item D-Bus proxies and actually invokes `Activate`.
pub struct ActivateRequest {
    pub service: String,
    pub x: i32,
    pub y: i32,
}

/// Internal command from the served `Watcher` interface to the run loop.
enum Command {
    Register { bus: String, path: String },
    Unregister { bus: String },
}

/// Proxy to a remote `org.kde.StatusNotifierItem` (implemented by the app).
#[proxy(
    interface = "org.kde.StatusNotifierItem",
    default_path = "/StatusNotifierItem"
)]
trait StatusNotifierItem {
    /// Left click. `x`/`y` are the click position in root coordinates.
    fn activate(&self, x: i32, y: i32) -> zbus::Result<()>;
    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;
    /// `Vec` of `(width, height, ARGB32 bytes)` frames; apps usually send one.
    #[zbus(property)]
    fn icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;
}

/// The `org.kde.StatusNotifierWatcher` service we expose on the session bus.
struct Watcher {
    cmd_tx: Sender<Command>,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &mut self,
        service: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let _ = emitter.status_notifier_item_registered(&service).await;
        // The argument is normally the item's bus name; some clients send an
        // absolute object path instead. For v1 we only handle the common case
        // (bus name + the spec's default path) — good enough for Discord/
        // Telegram/Electron.
        let _ = self
            .cmd_tx
            .send(Command::Register {
                bus: service,
                path: "/StatusNotifierItem".into(),
            });
        Ok(())
    }

    async fn register_status_notifier_host(&mut self, _service: String) -> fdo::Result<()> {
        // We are the host; nothing to do.
        Ok(())
    }

    async fn unregister_status_notifier_item(
        &mut self,
        service: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let _ = emitter.status_notifier_item_unregistered(&service).await;
        let _ = self.cmd_tx.send(Command::Unregister { bus: service });
        Ok(())
    }

    #[zbus(property)]
    async fn registered_status_notifier_items(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    async fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    async fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;
}

/// Owns the SNI thread handle and the plumbing the main loop needs: the
/// `mpsc` receiver (thread -> WM) and the self-pipe read fd to wake `poll`.
pub struct SniManager {
    rx: Receiver<SniEvent>,
    activate_tx: Sender<ActivateRequest>,
    wake_r: RawFd,
    _wake_w: RawFd,
}

impl SniManager {
    /// Spawn the SNI D-Bus thread. Never fails fatally: if the session bus is
    /// unavailable the thread logs and exits, but the WM keeps running (it just
    /// won't show modern tray icons).
    pub fn new() -> anyhow::Result<Self> {
        let mut fds = [0 as RawFd; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(anyhow::anyhow!("SNI: falha ao criar self-pipe"));
        }
        let (wake_r, wake_w) = (fds[0], fds[1]);
        set_nonblocking(wake_r);
        set_nonblocking(wake_w);

        let (to_wm_tx, to_wm_rx) = mpsc::channel();
        let (activate_tx, activate_rx) = mpsc::channel();

        thread::Builder::new().name("rustbox-sni".into()).spawn(move || {
            let res = smol::block_on(run_sni(to_wm_tx, activate_rx, wake_w));
            if let Err(e) = res {
                log::warn!("SNI: thread encerrou (tray moderna indisponível): {e}");
            }
        })?;

        Ok(Self {
            rx: to_wm_rx,
            activate_tx,
            wake_r,
            _wake_w: wake_w,
        })
    }

    /// Read fd for the main loop's `poll()` — woken whenever the thread sends
    /// an `SniEvent`.
    pub fn wake_fd(&self) -> RawFd {
        self.wake_r
    }

    /// Non-blocking drain of one pending event (call in a loop until `None`).
    pub fn try_recv(&self) -> Option<SniEvent> {
        self.rx.try_recv().ok()
    }

    /// Ask the SNI thread to invoke `Activate` on `service` (a tray click).
    pub fn activate(&self, service: &str, x: i32, y: i32) {
        let _ = self.activate_tx.send(ActivateRequest {
            service: service.to_string(),
            x,
            y,
        });
    }

    /// Clone of the channel used to forward click/activate requests to the
    /// SNI thread. Stored on each screen so a tray click can reach the thread.
    pub fn activator(&self) -> Sender<ActivateRequest> {
        self.activate_tx.clone()
    }

    /// Drain the self-pipe so its buffer does not grow without bound (the WM
    /// polls the read end for readability but only consumes the `mpsc` events).
    pub fn drain_wake(&self) {
        let mut buf = [0u8; 64];
        loop {
            let n = unsafe {
                libc::read(
                    self.wake_r,
                    buf.as_mut_ptr() as *mut libc::c_void,
                    buf.len(),
                )
            };
            if n <= 0 {
                break;
            }
        }
    }
}

fn set_nonblocking(fd: RawFd) {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
}

fn signal_pipe(fd: RawFd) {
    let buf = [1u8];
    unsafe {
        libc::write(fd, buf.as_ptr() as *const libc::c_void, 1);
    }
}

async fn run_sni(
    to_wm_tx: Sender<SniEvent>,
    activate_rx: Receiver<ActivateRequest>,
    wake_w: RawFd,
) -> zbus::Result<()> {
    let (cmd_tx, cmd_rx) = mpsc::channel();

    let conn = connection::Builder::session()?
        .name("org.kde.StatusNotifierWatcher")?
        .serve_at("/StatusNotifierWatcher", Watcher { cmd_tx })?
        .build()
        .await?;

    log::info!("SNI: StatusNotifierWatcher registrado no barramento de sessão");
    signal_pipe(wake_w);

    loop {
        // Drain click/activate requests from the WM.
        loop {
            match activate_rx.recv_timeout(Duration::ZERO) {
                Ok(req) => {
                    if let Ok(proxy) = StatusNotifierItemProxy::builder(&conn)
                        .destination(req.service.as_str())?
                        .path("/StatusNotifierItem")?
                        .build()
                        .await
                    {
                        if let Err(e) = proxy.activate(req.x, req.y).await {
                            log::debug!("SNI: Activate em {} falhou: {e}", req.service);
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            }
        }

        // Process registration/unregistration from the Watcher interface.
        match cmd_rx.recv_timeout(Duration::ZERO) {
            Ok(Command::Register { bus, path }) => {
                let _ = handle_register(&conn, &to_wm_tx, &bus, &path, wake_w).await;
            }
            Ok(Command::Unregister { bus }) => {
                let _ = to_wm_tx.send(SniEvent::Unregistered { service: bus });
                signal_pipe(wake_w);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        smol::Timer::after(Duration::from_millis(50)).await;
    }

    Ok(())
}

async fn handle_register(
    conn: &Connection,
    to_wm_tx: &Sender<SniEvent>,
    bus: &str,
    path: &str,
    wake_w: RawFd,
) -> zbus::Result<()> {
    let _ = to_wm_tx.send(SniEvent::Registered {
        service: bus.to_string(),
    });

    let proxy = StatusNotifierItemProxy::builder(conn)
        .destination(bus)?
        .path(path)?
        .build()
        .await?;

    if let Ok(frames) = proxy.icon_pixmap().await {
        if let Some((w, h, argb)) = largest_frame(&frames) {
            let _ = to_wm_tx.send(SniEvent::Updated {
                service: bus.to_string(),
                width: w as u32,
                height: h as u32,
                argb,
            });
            signal_pipe(wake_w);
        }
    }

    Ok(())
}

/// Pick the largest frame (apps usually send a single one).
fn largest_frame(frames: &[(i32, i32, Vec<u8>)]) -> Option<(i32, i32, Vec<u8>)> {
    frames
        .iter()
        .max_by_key(|(w, h, _)| w * h)
        .map(|(w, h, d)| (*w, *h, d.clone()))
}
