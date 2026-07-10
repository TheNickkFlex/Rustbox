use std::collections::HashMap;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::Atom as X11Atom;
use x11rb::protocol::xproto::ConnectionExt as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Atom {
    // Core protocol
    WmName,
    WmIconName,
    WmHints,
    WmNormalHints,
    WmClass,
    WmTransientFor,
    WmProtocols,
    WmDeleteWindow,
    WmTakeFocus,
    WmChangeState,
    WmState,
    WmClientMachine,
    WmCommand,

    // EWMH atoms
    NetSupported,
    NetClientList,
    NetClientListStacking,
    NetNumberOfDesktops,
    NetDesktopViewport,
    NetCurrentDesktop,
    NetDesktopNames,
    NetActiveWindow,
    NetWorkarea,
    NetSupportingWmCheck,
    NetVirtualRoots,
    NetCloseWindow,
    NetWmName,
    NetWmVisibleName,
    NetWmIconName,
    NetWmVisibleIconName,
    NetWmDesktop,
    NetWmState,
    NetWmStateSticky,
    NetWmStateMaximizedVert,
    NetWmStateMaximizedHorz,
    NetWmStateShaded,
    NetWmStateSkipTaskbar,
    NetWmStateSkipPager,
    NetWmStateHidden,
    NetWmStateFullscreen,
    NetWmStateAbove,
    NetWmStateBelow,
    NetWmStateDemandsAttention,
    NetWmAllowedActions,
    NetWmActionMove,
    NetWmActionResize,
    NetWmActionMinimize,
    NetWmActionShade,
    NetWmActionStick,
    NetWmActionMaximizeHorz,
    NetWmActionMaximizeVert,
    NetWmActionFullscreen,
    NetWmActionChangeDesktop,
    NetWmActionClose,
    NetWmActionAbove,
    NetWmActionBelow,
    NetWmWindowType,
    NetWmWindowTypeDesktop,
    NetWmWindowTypeDock,
    NetWmWindowTypeToolbar,
    NetWmWindowTypeMenu,
    NetWmWindowTypeUtility,
    NetWmWindowTypeSplash,
    NetWmWindowTypeDialog,
    NetWmWindowTypeNormal,
    NetWmWindowTypeDropdownMenu,
    NetWmWindowTypePopupMenu,
    NetWmWindowTypeTooltip,
    NetWmWindowTypeNotification,
    NetWmWindowTypeCombo,
    NetWmWindowTypeDnd,
    NetWmStrut,
    NetWmStrutPartial,
    NetWmIconGeometry,
    NetWmIcon,
    NetWmPid,
    NetWmUserTime,
    NetWmUserTimeWindow,
    NetFrameExtents,
    NetFrameWindow,
    NetRequestFrameExtents,

    // ICCCM
    MotifWmHints,
    KwmWmWinHint,

    // Fluxbox specific
    FluxboxWindow,
    FluxboxWindowHidden,
    FluxboxRemote,
    FluxboxRemoteCmd,

    // Atoms from X extensions
    Manager,
    Xembed,
    XembedInfo,
    XdndAware,
    XdndEnter,
    XdndPosition,
    XdndStatus,
    XdndLeave,
    XdndDrop,
    XdndFinished,
    XdndTypeList,
    XdndActionList,
    XdndActionCopy,

    // Custom
    GtkFrameExtents,
    GtkEdgeConstraint,
    Utf8String,
    String,
    Cardinal,
    Window,
    AtomEnum,
    WmMoveResize,
}

impl Atom {
    pub fn name(&self) -> &'static str {
        match self {
            Atom::WmName => "WM_NAME",
            Atom::WmIconName => "WM_ICON_NAME",
            Atom::WmHints => "WM_HINTS",
            Atom::WmNormalHints => "WM_NORMAL_HINTS",
            Atom::WmClass => "WM_CLASS",
            Atom::WmTransientFor => "WM_TRANSIENT_FOR",
            Atom::WmProtocols => "WM_PROTOCOLS",
            Atom::WmDeleteWindow => "WM_DELETE_WINDOW",
            Atom::WmTakeFocus => "WM_TAKE_FOCUS",
            Atom::WmChangeState => "WM_CHANGE_STATE",
            Atom::WmState => "WM_STATE",
            Atom::WmClientMachine => "WM_CLIENT_MACHINE",
            Atom::WmCommand => "WM_COMMAND",
            Atom::NetSupported => "_NET_SUPPORTED",
            Atom::NetClientList => "_NET_CLIENT_LIST",
            Atom::NetClientListStacking => "_NET_CLIENT_LIST_STACKING",
            Atom::NetNumberOfDesktops => "_NET_NUMBER_OF_DESKTOPS",
            Atom::NetDesktopViewport => "_NET_DESKTOP_VIEWPORT",
            Atom::NetCurrentDesktop => "_NET_CURRENT_DESKTOP",
            Atom::NetDesktopNames => "_NET_DESKTOP_NAMES",
            Atom::NetActiveWindow => "_NET_ACTIVE_WINDOW",
            Atom::NetWorkarea => "_NET_WORKAREA",
            Atom::NetSupportingWmCheck => "_NET_SUPPORTING_WM_CHECK",
            Atom::NetVirtualRoots => "_NET_VIRTUAL_ROOTS",
            Atom::NetCloseWindow => "_NET_CLOSE_WINDOW",
            Atom::NetWmName => "_NET_WM_NAME",
            Atom::NetWmVisibleName => "_NET_WM_VISIBLE_NAME",
            Atom::NetWmIconName => "_NET_WM_ICON_NAME",
            Atom::NetWmVisibleIconName => "_NET_WM_VISIBLE_ICON_NAME",
            Atom::NetWmDesktop => "_NET_WM_DESKTOP",
            Atom::NetWmState => "_NET_WM_STATE",
            Atom::NetWmStateSticky => "_NET_WM_STATE_STICKY",
            Atom::NetWmStateMaximizedVert => "_NET_WM_STATE_MAXIMIZED_VERT",
            Atom::NetWmStateMaximizedHorz => "_NET_WM_STATE_MAXIMIZED_HORZ",
            Atom::NetWmStateShaded => "_NET_WM_STATE_SHADED",
            Atom::NetWmStateSkipTaskbar => "_NET_WM_STATE_SKIP_TASKBAR",
            Atom::NetWmStateSkipPager => "_NET_WM_STATE_SKIP_PAGER",
            Atom::NetWmStateHidden => "_NET_WM_STATE_HIDDEN",
            Atom::NetWmStateFullscreen => "_NET_WM_STATE_FULLSCREEN",
            Atom::NetWmStateAbove => "_NET_WM_STATE_ABOVE",
            Atom::NetWmStateBelow => "_NET_WM_STATE_BELOW",
            Atom::NetWmStateDemandsAttention => "_NET_WM_STATE_DEMANDS_ATTENTION",
            Atom::NetWmAllowedActions => "_NET_WM_ALLOWED_ACTIONS",
            Atom::NetWmActionMove => "_NET_WM_ACTION_MOVE",
            Atom::NetWmActionResize => "_NET_WM_ACTION_RESIZE",
            Atom::NetWmActionMinimize => "_NET_WM_ACTION_MINIMIZE",
            Atom::NetWmActionShade => "_NET_WM_ACTION_SHADE",
            Atom::NetWmActionStick => "_NET_WM_ACTION_STICK",
            Atom::NetWmActionMaximizeHorz => "_NET_WM_ACTION_MAXIMIZE_HORZ",
            Atom::NetWmActionMaximizeVert => "_NET_WM_ACTION_MAXIMIZE_VERT",
            Atom::NetWmActionFullscreen => "_NET_WM_ACTION_FULLSCREEN",
            Atom::NetWmActionChangeDesktop => "_NET_WM_ACTION_CHANGE_DESKTOP",
            Atom::NetWmActionClose => "_NET_WM_ACTION_CLOSE",
            Atom::NetWmActionAbove => "_NET_WM_ACTION_ABOVE",
            Atom::NetWmActionBelow => "_NET_WM_ACTION_BELOW",
            Atom::NetWmWindowType => "_NET_WM_WINDOW_TYPE",
            Atom::NetWmWindowTypeDesktop => "_NET_WM_WINDOW_TYPE_DESKTOP",
            Atom::NetWmWindowTypeDock => "_NET_WM_WINDOW_TYPE_DOCK",
            Atom::NetWmWindowTypeToolbar => "_NET_WM_WINDOW_TYPE_TOOLBAR",
            Atom::NetWmWindowTypeMenu => "_NET_WM_WINDOW_TYPE_MENU",
            Atom::NetWmWindowTypeUtility => "_NET_WM_WINDOW_TYPE_UTILITY",
            Atom::NetWmWindowTypeSplash => "_NET_WM_WINDOW_TYPE_SPLASH",
            Atom::NetWmWindowTypeDialog => "_NET_WM_WINDOW_TYPE_DIALOG",
            Atom::NetWmWindowTypeNormal => "_NET_WM_WINDOW_TYPE_NORMAL",
            Atom::NetWmWindowTypeDropdownMenu => "_NET_WM_WINDOW_TYPE_DROPDOWN_MENU",
            Atom::NetWmWindowTypePopupMenu => "_NET_WM_WINDOW_TYPE_POPUP_MENU",
            Atom::NetWmWindowTypeTooltip => "_NET_WM_WINDOW_TYPE_TOOLTIP",
            Atom::NetWmWindowTypeNotification => "_NET_WM_WINDOW_TYPE_NOTIFICATION",
            Atom::NetWmWindowTypeCombo => "_NET_WM_WINDOW_TYPE_COMBO",
            Atom::NetWmWindowTypeDnd => "_NET_WM_WINDOW_TYPE_DND",
            Atom::NetWmStrut => "_NET_WM_STRUT",
            Atom::NetWmStrutPartial => "_NET_WM_STRUT_PARTIAL",
            Atom::NetWmIconGeometry => "_NET_WM_ICON_GEOMETRY",
            Atom::NetWmIcon => "_NET_WM_ICON",
            Atom::NetWmPid => "_NET_WM_PID",
            Atom::NetWmUserTime => "_NET_WM_USER_TIME",
            Atom::NetWmUserTimeWindow => "_NET_WM_USER_TIME_WINDOW",
            Atom::NetFrameExtents => "_NET_FRAME_EXTENTS",
            Atom::NetFrameWindow => "_NET_FRAME_WINDOW",
            Atom::NetRequestFrameExtents => "_NET_REQUEST_FRAME_EXTENTS",
            Atom::MotifWmHints => "_MOTIF_WM_HINTS",
            Atom::KwmWmWinHint => "_KDE_WM_WIN_HINT",
            Atom::FluxboxWindow => "_FLUXBOX_WINDOW",
            Atom::FluxboxWindowHidden => "_FLUXBOX_WINDOW_HIDDEN",
            Atom::FluxboxRemote => "Fluxbox/remote",
            Atom::FluxboxRemoteCmd => "FLUXBOX_REMOTE_CMD",
            Atom::Manager => "MANAGER",
            Atom::Xembed => "_XEMBED",
            Atom::XembedInfo => "_XEMBED_INFO",
            Atom::XdndAware => "XdndAware",
            Atom::XdndEnter => "XdndEnter",
            Atom::XdndPosition => "XdndPosition",
            Atom::XdndStatus => "XdndStatus",
            Atom::XdndLeave => "XdndLeave",
            Atom::XdndDrop => "XdndDrop",
            Atom::XdndFinished => "XdndFinished",
            Atom::XdndTypeList => "XdndTypeList",
            Atom::XdndActionList => "XdndActionList",
            Atom::XdndActionCopy => "XdndActionCopy",
            Atom::GtkFrameExtents => "_GTK_FRAME_EXTENTS",
            Atom::GtkEdgeConstraint => "_GTK_EDGE_CONSTRAINTS",
            Atom::Utf8String => "UTF8_STRING",
            Atom::String => "STRING",
            Atom::Cardinal => "CARDINAL",
            Atom::Window => "WINDOW",
            Atom::AtomEnum => "ATOM",
            Atom::WmMoveResize => "_WM_MOVERESIZE",
        }
    }
}

#[derive(Clone)]
pub struct AtomCache {
    cache: HashMap<Atom, X11Atom>,
}

impl AtomCache {
    pub fn new() -> Self {
        Self { cache: HashMap::new() }
    }

    pub fn init<C: Connection>(&mut self, conn: &C) -> Result<(), anyhow::Error> {
        let atoms: Vec<Atom> = vec![
            Atom::WmName, Atom::WmIconName, Atom::WmHints, Atom::WmNormalHints,
            Atom::WmClass, Atom::WmTransientFor, Atom::WmProtocols, Atom::WmDeleteWindow,
            Atom::WmTakeFocus, Atom::WmChangeState, Atom::WmState, Atom::WmClientMachine,
            Atom::WmCommand,
            Atom::NetSupported, Atom::NetClientList, Atom::NetClientListStacking,
            Atom::NetNumberOfDesktops, Atom::NetDesktopViewport, Atom::NetCurrentDesktop,
            Atom::NetDesktopNames, Atom::NetActiveWindow, Atom::NetWorkarea,
            Atom::NetSupportingWmCheck, Atom::NetVirtualRoots, Atom::NetCloseWindow,
            Atom::NetWmName, Atom::NetWmVisibleName, Atom::NetWmIconName,
            Atom::NetWmVisibleIconName, Atom::NetWmDesktop, Atom::NetWmState,
            Atom::NetWmStateSticky, Atom::NetWmStateMaximizedVert,
            Atom::NetWmStateMaximizedHorz, Atom::NetWmStateShaded,
            Atom::NetWmStateSkipTaskbar, Atom::NetWmStateSkipPager,
            Atom::NetWmStateHidden, Atom::NetWmStateFullscreen,
            Atom::NetWmStateAbove, Atom::NetWmStateBelow,
            Atom::NetWmStateDemandsAttention, Atom::NetWmAllowedActions,
            Atom::NetWmActionMove, Atom::NetWmActionResize, Atom::NetWmActionMinimize,
            Atom::NetWmActionShade, Atom::NetWmActionStick,
            Atom::NetWmActionMaximizeHorz, Atom::NetWmActionMaximizeVert,
            Atom::NetWmActionFullscreen, Atom::NetWmActionChangeDesktop,
            Atom::NetWmActionClose, Atom::NetWmActionAbove, Atom::NetWmActionBelow,
            Atom::NetWmWindowType, Atom::NetWmWindowTypeDesktop, Atom::NetWmWindowTypeDock,
            Atom::NetWmWindowTypeToolbar, Atom::NetWmWindowTypeMenu,
            Atom::NetWmWindowTypeUtility, Atom::NetWmWindowTypeSplash,
            Atom::NetWmWindowTypeDialog, Atom::NetWmWindowTypeNormal,
            Atom::NetWmWindowTypeDropdownMenu, Atom::NetWmWindowTypePopupMenu,
            Atom::NetWmWindowTypeTooltip, Atom::NetWmWindowTypeNotification,
            Atom::NetWmWindowTypeCombo, Atom::NetWmWindowTypeDnd,
            Atom::NetWmStrut, Atom::NetWmStrutPartial, Atom::NetWmIconGeometry,
            Atom::NetWmIcon, Atom::NetWmPid, Atom::NetWmUserTime,
            Atom::NetWmUserTimeWindow, Atom::NetFrameExtents, Atom::NetFrameWindow,
            Atom::NetRequestFrameExtents,
            Atom::MotifWmHints, Atom::KwmWmWinHint,
            Atom::FluxboxWindow, Atom::FluxboxWindowHidden, Atom::FluxboxRemote,
            Atom::FluxboxRemoteCmd,
            Atom::Manager, Atom::Xembed, Atom::XembedInfo,
            Atom::XdndAware, Atom::XdndEnter, Atom::XdndPosition,
            Atom::XdndStatus, Atom::XdndLeave, Atom::XdndDrop,
            Atom::XdndFinished, Atom::XdndTypeList, Atom::XdndActionList,
            Atom::XdndActionCopy,
            Atom::GtkFrameExtents, Atom::GtkEdgeConstraint,
            Atom::Utf8String, Atom::String, Atom::Cardinal,
            Atom::Window, Atom::AtomEnum, Atom::WmMoveResize,
        ];

        // Fire all intern_atom requests up front (no round-trip per call),
        // then collect the replies. This batches the startup requests into a
        // single flush instead of N serial request/reply round-trips.
        let mut cookies = Vec::with_capacity(atoms.len());
        for atom in &atoms {
            cookies.push((atom, conn.intern_atom(false, atom.name().as_bytes())?));
        }
        for (atom, cookie) in cookies {
            let reply = cookie.reply()?;
            self.cache.insert(*atom, reply.atom);
        }

        Ok(())
    }

    pub fn get(&self, atom: Atom) -> X11Atom {
        self.cache.get(&atom).copied().unwrap_or(x11rb::NONE)
    }

    pub fn get_by_name<C: Connection>(&self, conn: &C, name: &str) -> Option<X11Atom> {
        let atom = conn.intern_atom(false, name.as_bytes()).ok()?;
        let reply = atom.reply().ok()?;
        Some(reply.atom)
    }
}

impl Default for AtomCache {
    fn default() -> Self {
        Self::new()
    }
}
