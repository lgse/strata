// SPDX-License-Identifier: MIT

use rune::raw_window_handle::RawWindowHandle;
use rune::{AppHandle, Drag, El, Phase, PointerPress, Tx, WindowHandle, col, el};

use super::state::Native;
use super::x11::Link;
use super::{Csd, Edge};

pub fn decorate<S: 'static>(content: El, csd: &Csd, access: fn(&mut S) -> &mut Csd) -> El {
    let root = col().key("csd-window").w_full().h_full().child(content);
    if !csd.shows_bands() {
        return root;
    }
    let (t, c) = (csd.bands().thickness, csd.bands().corner);
    root.children(Edge::ALL.map(|edge| {
        let band = el()
            .key(edge.key())
            .cursor(edge.cursor())
            .a11y_hidden()
            .on_drag(move |s: &mut S, d: &mut Drag, _: &mut Tx| {
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

pub fn header<S: 'static>(header: El, csd: &Csd, access: fn(&mut S) -> &mut Csd) -> El {
    let header = header.window_drag();
    if !csd.shows_window_menu() {
        return header;
    }
    header.on_secondary_press(move |s: &mut S, _: PointerPress, _: &mut Tx| {
        access(s).window_menu();
    })
}

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
