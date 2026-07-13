use x11rb::protocol::randr;
use x11rb::protocol::xproto::*;

/// High-level, rustbox-specific event enum. A thin wrapper over the raw
/// `x11rb::protocol::Event` that only retains the variants we handle, which
/// keeps the rest of the codebase free of the giant generated event union.
pub enum Event {
    KeyPress(KeyPressEvent),
    KeyRelease(KeyReleaseEvent),
    ButtonPress(ButtonPressEvent),
    ButtonRelease(ButtonReleaseEvent),
    MotionNotify(MotionNotifyEvent),
    EnterNotify(EnterNotifyEvent),
    LeaveNotify(LeaveNotifyEvent),
    FocusIn(FocusInEvent),
    FocusOut(FocusOutEvent),
    KeymapNotify(KeymapNotifyEvent),
    Expose(ExposeEvent),
    GraphicsExposure(GraphicsExposureEvent),
    NoExposure(NoExposureEvent),
    VisibilityNotify(VisibilityNotifyEvent),
    CreateNotify(CreateNotifyEvent),
    DestroyNotify(DestroyNotifyEvent),
    UnmapNotify(UnmapNotifyEvent),
    MapNotify(MapNotifyEvent),
    MapRequest(MapRequestEvent),
    ReparentNotify(ReparentNotifyEvent),
    ConfigureNotify(ConfigureNotifyEvent),
    ConfigureRequest(ConfigureRequestEvent),
    GravityNotify(GravityNotifyEvent),
    ResizeRequest(ResizeRequestEvent),
    CirculateNotify(CirculateNotifyEvent),
    CirculateRequest(CirculateRequestEvent),
    PropertyNotify(PropertyNotifyEvent),
    SelectionClear(SelectionClearEvent),
    SelectionRequest(SelectionRequestEvent),
    SelectionNotify(SelectionNotifyEvent),
    ColormapNotify(ColormapNotifyEvent),
    ClientMessage(ClientMessageEvent),
    MappingNotify(MappingNotifyEvent),
    RandRNotify(randr::NotifyEvent),
    RandRScreenChangeNotify(randr::ScreenChangeNotifyEvent),
    Generic(x11rb::protocol::Event),
}

impl Event {
    pub fn from_x11(event: x11rb::protocol::Event) -> Option<Self> {
        Some(match event {
            x11rb::protocol::Event::KeyPress(e) => Event::KeyPress(e),
            x11rb::protocol::Event::KeyRelease(e) => Event::KeyRelease(e),
            x11rb::protocol::Event::ButtonPress(e) => Event::ButtonPress(e),
            x11rb::protocol::Event::ButtonRelease(e) => Event::ButtonRelease(e),
            x11rb::protocol::Event::MotionNotify(e) => Event::MotionNotify(e),
            x11rb::protocol::Event::EnterNotify(e) => Event::EnterNotify(e),
            x11rb::protocol::Event::LeaveNotify(e) => Event::LeaveNotify(e),
            x11rb::protocol::Event::FocusIn(e) => Event::FocusIn(e),
            x11rb::protocol::Event::FocusOut(e) => Event::FocusOut(e),
            x11rb::protocol::Event::KeymapNotify(e) => Event::KeymapNotify(e),
            x11rb::protocol::Event::Expose(e) => Event::Expose(e),
            x11rb::protocol::Event::GraphicsExposure(e) => Event::GraphicsExposure(e),
            x11rb::protocol::Event::NoExposure(e) => Event::NoExposure(e),
            x11rb::protocol::Event::VisibilityNotify(e) => Event::VisibilityNotify(e),
            x11rb::protocol::Event::CreateNotify(e) => Event::CreateNotify(e),
            x11rb::protocol::Event::DestroyNotify(e) => Event::DestroyNotify(e),
            x11rb::protocol::Event::UnmapNotify(e) => Event::UnmapNotify(e),
            x11rb::protocol::Event::MapNotify(e) => Event::MapNotify(e),
            x11rb::protocol::Event::MapRequest(e) => Event::MapRequest(e),
            x11rb::protocol::Event::ReparentNotify(e) => Event::ReparentNotify(e),
            x11rb::protocol::Event::ConfigureNotify(e) => Event::ConfigureNotify(e),
            x11rb::protocol::Event::ConfigureRequest(e) => Event::ConfigureRequest(e),
            x11rb::protocol::Event::GravityNotify(e) => Event::GravityNotify(e),
            x11rb::protocol::Event::ResizeRequest(e) => Event::ResizeRequest(e),
            x11rb::protocol::Event::CirculateNotify(e) => Event::CirculateNotify(e),
            x11rb::protocol::Event::CirculateRequest(e) => Event::CirculateRequest(e),
            x11rb::protocol::Event::PropertyNotify(e) => Event::PropertyNotify(e),
            x11rb::protocol::Event::SelectionClear(e) => Event::SelectionClear(e),
            x11rb::protocol::Event::SelectionRequest(e) => Event::SelectionRequest(e),
            x11rb::protocol::Event::SelectionNotify(e) => Event::SelectionNotify(e),
            x11rb::protocol::Event::ColormapNotify(e) => Event::ColormapNotify(e),
            x11rb::protocol::Event::ClientMessage(e) => Event::ClientMessage(e),
            x11rb::protocol::Event::MappingNotify(e) => Event::MappingNotify(e),
            x11rb::protocol::Event::RandrNotify(e) => Event::RandRNotify(e),
            x11rb::protocol::Event::RandrScreenChangeNotify(e) => Event::RandRScreenChangeNotify(e),
            _ => {
                return None;
            }
        })
    }
}
