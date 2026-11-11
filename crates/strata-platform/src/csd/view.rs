//! The elements: resize bands over the content, the header's drag region, and the link to
//! the native window.

use rune::raw_window_handle::RawWindowHandle;
use rune::{AppHandle, Drag, El, Phase, PointerPress, Tx, WindowHandle, col, el};

use super::state::Native;
use super::x11::Link;
use super::{Csd, Edge};

/// `content` with the resize bands over its edges, where [`Csd::shows_bands`] says they can
/// work. `content` should fill the window (`w_full`, `h_full`). Put the result at the root of
/// the view, under any popup layers.
///
/// The bands have no role and no name: assistive technology hears nothing of them.
pub fn decorate<S: 'static>(content: El, csd: &Csd, access: fn(&mut S) -> &mut Csd) -> El {
    let root = col().key("csd-window").w_full().h_full().child(content);
    if !csd.shows_bands() {
        return root;
    }
    let (t, c) = (csd.bands().thickness, csd.bands().corner);
    root.children(Edge::ALL.map(|edge| {
        // Rune reads an element with a drag handler as a button: hide it, since a band has
        // nothing to say to assistive technology.
        let band = el()
            .key(edge.key())
            .cursor(edge.cursor())
            .a11y_hidden()
            .on_drag(move |s: &mut S, d: &mut Drag, _: &mut Tx| {
                // Phase::Down runs while the button is still held, which the window manager
                // needs to take the resize over.
                if d.phase == Phase::Down {
                    access(s).begin_resize(edge, d.screen_point());
                }
            });
        match edge {
            Edge::North => band.pin(Some(c), Some(0.0), Some(c), None).h(t),
            Edge::South => band.pin(Some(c), None, Some(c), Some(0.0)).h(t),
            Edge::West => band.pin(Some(0.0), Some(c), None, Some(c)).w(t),
            Edge::East => band.pin(None, Some(c), Some(0.0), Some(c)).w(t),
            Edge::NorthWest => band.pin(Some(0.0), Some(0.0), None, None).size(c, c),
            Edge::NorthEast => band.pin(None, Some(0.0), Some(0.0), None).size(c, c),
            Edge::SouthEast => band.pin(None, None, Some(0.0), Some(0.0)).size(c, c),
            Edge::SouthWest => band.pin(Some(0.0), None, None, Some(0.0)).size(c, c),
        }
    }))
}

/// `header` as the window's title bar: a press on it, or on any part of it without press
/// handlers of its own, moves the window, and a double press maximises or restores it (Rune's
/// `.window_drag()`, with Rune's 0.5 s double-click interval: draft U13, not filed). Where the
/// window manager has a window menu ([`Csd::shows_window_menu`]), a secondary click opens it.
///
/// Don't give `header` itself press, double-press or drag handlers: Rune then stops treating it
/// as a drag region. Its buttons keep their presses. A text field without its own
/// `on_secondary_press` gets Rune's field menu instead, which Linux can't show, so a right-click
/// there shows nothing: give a header field its own `on_secondary_press`.
pub fn header<S: 'static>(header: El, csd: &Csd, access: fn(&mut S) -> &mut Csd) -> El {
    let header = header.window_drag();
    if !csd.shows_window_menu() {
        return header;
    }
    header.on_secondary_press(move |s: &mut S, _: PointerPress, _: &mut Tx| {
        access(s).window_menu();
    })
}

/// Links the app's [`Csd`] to its native window. Call it from `App::on_window`, which runs
/// again whenever the native window is re-created:
///
/// ```ignore
/// let handle = app.handle();
/// app.on_window(move |w| csd::attach(w, &handle, |s: &mut State| &mut s.csd))
/// ```
///
/// On X11 it opens this module's own connections: one for requests, and one for a thread that
/// watches the window's state and the window manager's capabilities and reports them through
/// `AppHandle::update`. The thread stops when the window is destroyed or the link is dropped
/// (a re-created window replaces it). If the connection fails, the [`Csd`] says so through
/// [`Notice::Error`](super::Notice::Error) and [`Csd::connection_error`], its capabilities
/// say resize, minimise and the window menu can't work, and those requests answer
/// [`Gap::NoConnection`](super::Gap::NoConnection).
pub fn attach<S: 'static>(
    window: &WindowHandle,
    app: &AppHandle<S>,
    access: fn(&mut S) -> &mut Csd,
) {
    let found = window
        .with_raw_handle(|h| match h.as_raw() {
            RawWindowHandle::Wayland(_) => Found::Wayland,
            RawWindowHandle::Xlib(x) => u32::try_from(x.window).map_or(Found::Other, Found::X11),
            RawWindowHandle::Xcb(x) => Found::X11(x.window.get()),
            _ => Found::Other,
        })
        .unwrap_or(Found::Other);
    let native = match found {
        Found::Wayland => Native::Wayland,
        Found::Other => Native::Other,
        Found::X11(id) => {
            let reports = app.clone();
            let link = Link::connect(id, move |report| {
                reports.update(move |s, _| access(s).apply(report));
            })
            .map(Box::new);
            Native::X11 { window: id, link }
        }
    };
    app.update(move |s, _| access(s).attached(native));
}

enum Found {
    Wayland,
    X11(u32),
    Other,
}
