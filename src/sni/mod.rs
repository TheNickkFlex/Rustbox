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
use crate::notify::SignalEvent;

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};

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

/// A request from the WM (a tray right-click) to the SNI thread, which
/// invokes `ContextMenu` on the StatusNotifierItem.
pub struct ContextMenuRequest {
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
    /// Middle click.
    fn secondary_activate(&self, x: i32, y: i32) -> zbus::Result<()>;
    /// Right click — show the app's context menu at `x`/`y`.
    fn context_menu(&self, x: i32, y: i32) -> zbus::Result<()>;
    #[zbus(property)]
    fn icon_name(&self) -> zbus::Result<String>;
    /// `Vec` of `(width, height, ARGB32 bytes)` frames; apps usually send one.
    #[zbus(property)]
    fn icon_pixmap(&self) -> zbus::Result<Vec<(i32, i32, Vec<u8>)>>;

    /// Emitted by the app when its icon changes (badge, play/pause, etc.).
    #[zbus(signal)]
    fn new_icon(&self) -> zbus::Result<()>;
}

/// The `org.kde.StatusNotifierWatcher` service we expose on the session bus.
struct Watcher {
    cmd_tx: UnboundedSender<Command>,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &mut self,
        service: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let _ = emitter.status_notifier_item_registered(&service).await;
        // The argument is normally the item's bus name. Some clients (older
        // Qt/Chromium) send an absolute object path, or "bus_name/path"
        // concatenated. Parse both forms so those apps also show up in the tray.
        let (bus, path) = if let Some(slash) = service.find('/') {
            let (b, p) = service.split_at(slash);
            let b = if b.is_empty() {
                // Absolute path only: build a synthetic bus name.
                "org.kde.StatusNotifierItem"
            } else {
                b
            };
            (b.to_string(), p.to_string())
        } else {
            (service, "/StatusNotifierItem".into())
        };
        let _ = self.cmd_tx.unbounded_send(Command::Register { bus, path });
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
        let _ = self.cmd_tx.unbounded_send(Command::Unregister { bus: service });
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
    activate_tx: UnboundedSender<ActivateRequest>,
    context_menu_tx: UnboundedSender<ContextMenuRequest>,
    wake_r: RawFd,
    _wake_w: RawFd,
}

impl SniManager {
    /// Spawn the SNI + notifications D-Bus thread. `notify_tx` is the channel
    /// sender used by the `org.freedesktop.Notifications` handler; `signal_rx`
    /// receives signal-emission requests from the main thread.
    pub fn new(
        notify_tx: Sender<crate::notify::NotifyCommand>,
        signal_rx: futures_channel::mpsc::UnboundedReceiver<SignalEvent>,
    ) -> anyhow::Result<Self> {
        let mut fds = [0 as RawFd; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(anyhow::anyhow!("SNI: falha ao criar self-pipe"));
        }
        let (wake_r, wake_w) = (fds[0], fds[1]);
        set_nonblocking(wake_r);
        set_nonblocking(wake_w);

        let (to_wm_tx, to_wm_rx) = mpsc::channel();
        let (activate_tx, activate_rx) = futures_channel::mpsc::unbounded();
        let (context_menu_tx, context_menu_rx) = futures_channel::mpsc::unbounded();

        thread::Builder::new().name("rustbox-sni".into()).spawn(move || {
            let res = smol::block_on(run_sni(
                to_wm_tx, activate_rx, context_menu_rx, wake_w, notify_tx, signal_rx,
            ));
            if let Err(e) = res {
                log::warn!("SNI: thread encerrou (tray moderna indisponível): {e}");
            }
        })?;

        Ok(Self {
            rx: to_wm_rx,
            activate_tx,
            context_menu_tx,
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

    /// Ask the SNI thread to invoke `Activate` on `service` (a tray left-click).
    pub fn activate(&self, service: &str, x: i32, y: i32) {
        let _ = self.activate_tx.unbounded_send(ActivateRequest {
            service: service.to_string(),
            x,
            y,
        });
    }

    /// Ask the SNI thread to invoke `ContextMenu` on `service` (a tray right-click).
    pub fn context_menu(&self, service: &str, x: i32, y: i32) {
        let _ = self.context_menu_tx.unbounded_send(ContextMenuRequest {
            service: service.to_string(),
            x,
            y,
        });
    }

    /// Clone of the channel used to forward click/activate requests to the
    /// SNI thread. Stored on each screen so a tray click can reach the thread.
    pub fn activator(&self) -> UnboundedSender<ActivateRequest> {
        self.activate_tx.clone()
    }

    /// Clone of the channel used to forward right-click/context-menu requests.
    pub fn context_menu_activator(&self) -> UnboundedSender<ContextMenuRequest> {
        self.context_menu_tx.clone()
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
    mut activate_rx: UnboundedReceiver<ActivateRequest>,
    mut context_menu_rx: UnboundedReceiver<ContextMenuRequest>,
    wake_w: RawFd,
    notify_tx: Sender<crate::notify::NotifyCommand>,
    mut signal_rx: futures_channel::mpsc::UnboundedReceiver<SignalEvent>,
) -> zbus::Result<()> {
    use futures_util::select;
    use futures_util::stream::StreamExt;

    let (cmd_tx, mut cmd_rx) = futures_channel::mpsc::unbounded();

    let conn = connection::Builder::session()?
        .name("org.kde.StatusNotifierWatcher")?
        .name("org.freedesktop.Notifications")?
        .serve_at("/StatusNotifierWatcher", Watcher { cmd_tx })?
        .serve_at(
            "/org/freedesktop/Notifications",
            crate::notify::NotificationsHandler::new(notify_tx),
        )?
        .build()
        .await?;

    log::info!("SNI: StatusNotifierWatcher + Notifications registrados no barramento");
    signal_pipe(wake_w);

    loop {
        select! {
            req = activate_rx.select_next_some() => {
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
            req = context_menu_rx.select_next_some() => {
                if let Ok(proxy) = StatusNotifierItemProxy::builder(&conn)
                    .destination(req.service.as_str())?
                    .path("/StatusNotifierItem")?
                    .build()
                    .await
                {
                    if let Err(e) = proxy.context_menu(req.x, req.y).await {
                        log::debug!("SNI: ContextMenu em {} falhou: {e}", req.service);
                    }
                }
            }
            cmd = cmd_rx.select_next_some() => {
                match cmd {
                    Command::Register { bus, path } => {
                        let _ = handle_register(&conn, &to_wm_tx, &bus, &path, wake_w).await;
                    }
                    Command::Unregister { bus } => {
                        let _ = to_wm_tx.send(SniEvent::Unregistered { service: bus });
                        signal_pipe(wake_w);
                    }
                }
            }
            sig = signal_rx.select_next_some() => {
                match sig {
                    SignalEvent::NotificationClosed { id, reason } => {
                        crate::notify::emit_closed_signal(&conn, id, reason).await;
                    }
                    SignalEvent::ActionInvoked { id, action_key } => {
                        crate::notify::emit_action_signal(&conn, id, &action_key).await;
                    }
                }
            }
        }
    }
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
        .destination(bus.to_string())?
        .path(path.to_string())?
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

    // Keep the tray icon alive: some apps (Discord, media players, etc.)
    // change their icon after registration (notification badge, play/pause).
    // Watch the dedicated SNI `NewIcon` signal plus the `IconPixmap` and
    // `IconName` property changes, re-sending `SniEvent::Updated` each time.
    let proxy_clone_for_new = proxy.clone();
    let bus_for_new = bus.to_string();
    let to_wm_for_new = to_wm_tx.clone();
    smol::spawn(async move {
        use futures_util::stream::StreamExt;
        if let Ok(mut new_icon) = proxy_clone_for_new.receive_new_icon().await {
            while let Some(_sig) = new_icon.next().await {
                if let Ok(frames) = proxy_clone_for_new.icon_pixmap().await {
                    if let Some((w, h, argb)) = largest_frame(&frames) {
                        let _ = to_wm_for_new.send(SniEvent::Updated {
                            service: bus_for_new.clone(),
                            width: w as u32,
                            height: h as u32,
                            argb,
                        });
                        signal_pipe(wake_w);
                    }
                }
            }
        }
    })
    .detach();

    // `IconPixmap` property changes.
    {
        let bus = bus.to_string();
        let to_wm_tx = to_wm_tx.clone();
        let proxy_clone = proxy.clone();
        smol::spawn(async move {
            use futures_util::stream::StreamExt;
            let mut changes = proxy_clone.receive_icon_pixmap_changed().await;
            while let Some(_change) = changes.next().await {
                if let Ok(frames) = proxy_clone.icon_pixmap().await {
                    if let Some((w, h, argb)) = largest_frame(&frames) {
                        let _ = to_wm_tx.send(SniEvent::Updated {
                            service: bus.clone(),
                            width: w as u32,
                            height: h as u32,
                            argb,
                        });
                        signal_pipe(wake_w);
                    }
                }
            }
        })
        .detach();
    }

    // `IconName` property changes (re-resolve the theme icon).
    {
        let bus = bus.to_string();
        let to_wm_tx = to_wm_tx.clone();
        let proxy_clone = proxy.clone();
        smol::spawn(async move {
            use futures_util::stream::StreamExt;
            let mut changes = proxy_clone.receive_icon_name_changed().await;
            while let Some(_change) = changes.next().await {
                if let Ok(frames) = proxy_clone.icon_pixmap().await {
                    if let Some((w, h, argb)) = largest_frame(&frames) {
                        let _ = to_wm_tx.send(SniEvent::Updated {
                            service: bus.clone(),
                            width: w as u32,
                            height: h as u32,
                            argb,
                        });
                        signal_pipe(wake_w);
                    }
                }
            }
        })
        .detach();
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
