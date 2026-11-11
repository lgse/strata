//! Headless tests of the decorations on a `rune::Runtime`: where presses go, what the header
//! keeps of Rune's drag region, and where each window button's request goes.
//!
//! Feature map: `app/window`, "dragging … header moves the window and dragging a window edge
//! resizes it", "Maximize maximizes the window …", "Minimize minimizes the window and Close
//! closes it", and the double-click probe.

use rune::{Cx, El, Env, PointerPress, Runtime, TitlebarAction, Tx, col, el, row};

use super::frame::pick;
use super::*;

const SIZE: (f32, f32) = (400.0, 300.0);

struct State {
    csd: Csd,
    button_presses: u32,
    field_menus: u32,
}

fn access(s: &mut State) -> &mut Csd {
    &mut s.csd
}

/// A header with a button and a field that has its own menu, over a body.
fn view(s: &State, _: &mut Cx<State>) -> El {
    let bar =
        row()
            .key("bar")
            .w_full()
            .h(40.0)
            .child(
                el().key("button")
                    .size(40.0, 40.0)
                    .role("button")
                    .label("Button")
                    .on_press(|s: &mut State| s.button_presses += 1),
            )
            .child(el().key("field").size(80.0, 40.0).on_secondary_press(
                |s: &mut State, _: PointerPress, _: &mut Tx| s.field_menus += 1,
            ));
    let content = col()
        .w_full()
        .h_full()
        .child(header(bar, &s.csd, access))
        .child(el().grow(1.0));
    decorate(content, &s.csd, access)
}

fn runtime(csd: Csd) -> Runtime<State> {
    let state = State {
        csd,
        button_presses: 0,
        field_menus: 0,
    };
    let mut rt = Runtime::new(state, view, SIZE, Env::new(true, 1.0));
    rt.frame(0.0);
    rt
}

fn borderless(backend: Backend) -> Csd {
    Csd::new(Frame::Borderless, backend)
}

/// A press and release at (x, y), 20 ms after the last frame. Returns the titlebar action the
/// press asked the shell for.
fn click(rt: &mut Runtime<State>, (x, y): (f32, f32)) -> Option<TitlebarAction> {
    rt.frame(0.01);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    let action = rt.take_titlebar_action();
    rt.pointer_up(x, y);
    rt.frame(0.01);
    action
}

fn secondary_click(rt: &mut Runtime<State>, (x, y): (f32, f32)) {
    rt.frame(0.01);
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    rt.frame(0.01);
}

/// A report from outside any handler, as `AppHandle::update` delivers it.
fn deliver(rt: &mut Runtime<State>, report: Report) {
    rt.update(move |s, _| s.csd.apply(report));
    rt.frame(0.01);
}

/// A new `Csd` in place of the old, as `attach` makes for a re-created window.
fn replace(rt: &mut Runtime<State>, csd: Csd) {
    rt.update(move |s, _| s.csd = csd);
    rt.frame(0.01);
}

/// A point inside each edge's band in a 400 × 300 window.
fn inside(edge: Edge) -> (f32, f32) {
    let (w, h) = SIZE;
    match edge {
        Edge::North => (w / 2.0, 2.0),
        Edge::NorthEast => (w - 4.0, 4.0),
        Edge::East => (w - 2.0, h / 2.0),
        Edge::SouthEast => (w - 4.0, h - 4.0),
        Edge::South => (w / 2.0, h - 2.0),
        Edge::SouthWest => (4.0, h - 4.0),
        Edge::West => (2.0, h / 2.0),
        Edge::NorthWest => (4.0, 4.0),
    }
}

#[test]
fn a_press_on_each_band_asks_for_a_resize_that_way() {
    let mut rt = runtime(borderless(Backend::Headless));
    for edge in Edge::ALL {
        let (x, y) = inside(edge);
        rt.pointer_move(x, y);
        assert_eq!(
            rt.cursor(),
            edge.cursor(),
            "the cursor over the {edge:?} band"
        );
        let action = click(&mut rt, (x, y));
        assert_eq!(action, None, "the {edge:?} band doesn't move the window");
        assert_eq!(
            rt.state.csd.last_outcome(),
            Some(&Outcome::Sent {
                request: Request::Resize(edge),
                route: Route::Headless
            }),
        );
    }
}

#[test]
fn the_header_moves_the_window_and_a_double_press_maximises_it() {
    let mut rt = runtime(borderless(Backend::Headless));
    let bare = (300.0, 20.0);
    assert_eq!(click(&mut rt, bare), Some(TitlebarAction::Drag));
    assert_eq!(click(&mut rt, bare), Some(TitlebarAction::DoubleClick));
    rt.frame(1.0);
    assert_eq!(click(&mut rt, bare), Some(TitlebarAction::Drag));
    // Rune pairs presses within its fixed 0.5 s (draft U13, not filed).
    rt.frame(0.6);
    assert_eq!(
        click(&mut rt, bare),
        Some(TitlebarAction::Drag),
        "too slow to pair"
    );

    assert_eq!(click(&mut rt, (20.0, 20.0)), None, "the header's button");
    assert_eq!(rt.state.button_presses, 1, "keeps its press");
    assert_eq!(rt.state.csd.last_outcome(), None);
}

#[test]
fn a_secondary_click_on_the_header_opens_the_window_menu_only_where_there_is_one() {
    let mut rt = runtime(borderless(Backend::Headless));
    secondary_click(&mut rt, (300.0, 20.0));
    assert_eq!(
        rt.state.csd.last_outcome(),
        None,
        "the window manager's menu is unknown"
    );

    let mut report = Report::default();
    report.capabilities.window_menu = Some(true);
    deliver(&mut rt, report);
    secondary_click(&mut rt, (300.0, 20.0));
    let menu = Some(Outcome::Sent {
        request: Request::WindowMenu,
        route: Route::Headless,
    });
    assert_eq!(rt.state.csd.last_outcome().cloned(), menu);

    // The header's button has no menu of its own, so the window menu takes it, as GTK's header
    // bar does; the field keeps its own.
    replace(&mut rt, borderless(Backend::Headless));
    deliver(&mut rt, report);
    secondary_click(&mut rt, (20.0, 20.0));
    assert_eq!(rt.state.csd.last_outcome().cloned(), menu);
    replace(&mut rt, borderless(Backend::Headless));
    deliver(&mut rt, report);
    secondary_click(&mut rt, (80.0, 20.0));
    assert_eq!(rt.state.field_menus, 1);
    assert_eq!(rt.state.csd.last_outcome(), None);
}

#[test]
fn bands_are_mounted_only_where_they_can_resize() {
    let maximized = Report {
        state: WindowState {
            maximized: Some(true),
            fullscreen: Some(false),
        },
        ..Report::default()
    };
    let fullscreen = Report {
        state: WindowState {
            maximized: Some(false),
            fullscreen: Some(true),
        },
        ..Report::default()
    };
    let cases = [
        (
            "a native frame",
            Csd::new(Frame::Native, Backend::X11),
            None,
        ),
        (
            "borderless on Wayland (U6)",
            borderless(Backend::Wayland),
            None,
        ),
        ("maximised", borderless(Backend::Headless), Some(maximized)),
        (
            "full screen",
            borderless(Backend::Headless),
            Some(fullscreen),
        ),
    ];
    for (what, csd, report) in cases {
        let mut rt = runtime(csd);
        if let Some(report) = report {
            deliver(&mut rt, report);
        }
        let (x, y) = inside(Edge::East);
        rt.pointer_move(x, y);
        assert_eq!(
            rt.cursor(),
            rune::Cursor::Default,
            "{what}: no resize cursor"
        );
        click(&mut rt, (x, y));
        assert_eq!(
            rt.state.csd.last_outcome(),
            None,
            "{what}: the edge asks for nothing"
        );
    }

    // Restored, the bands come back.
    let mut rt = runtime(borderless(Backend::Headless));
    deliver(&mut rt, maximized);
    deliver(&mut rt, Report::default());
    click(&mut rt, inside(Edge::East));
    assert_eq!(
        rt.state.csd.last_outcome().map(Outcome::request),
        Some(Request::Resize(Edge::East))
    );
}

#[test]
fn the_bands_are_silent_to_assistive_technology() {
    let rt = runtime(borderless(Backend::Headless));
    assert!(rt.state.csd.shows_bands());
    let tree = rt.accessibility_json().to_string();
    assert!(
        tree.contains("\"button\""),
        "the tree holds the header's button: {tree}"
    );
    assert!(!tree.contains("csd-edge"), "no band is a node: {tree}");
}

/// Each request on each frame and backend: through Rune, through X11, or the gap that stops it.
#[test]
fn each_request_goes_where_it_can_work() {
    use Gap::*;
    let rune = |request| Outcome::Sent {
        request,
        route: Route::Rune,
    };
    let gap = |request, gap| Outcome::Unsupported { request, gap };
    let run = |csd: &mut Csd| {
        let mut tx = Tx::default();
        [
            csd.close(&mut tx),
            csd.toggle_maximized(&mut tx),
            csd.minimize(&mut tx),
            csd.begin_resize(Edge::East, None),
            csd.window_menu(),
        ]
    };
    use Request::{Close, Minimize, Resize, ToggleMaximize, WindowMenu};
    let resize = Resize(Edge::East);

    let mut native = Csd::new(Frame::Native, Backend::Wayland);
    assert_eq!(
        run(&mut native),
        [
            rune(Close),
            rune(ToggleMaximize),
            rune(Minimize),
            gap(resize, NativeFrame),
            gap(WindowMenu, NativeFrame),
        ]
    );

    let mut wayland = borderless(Backend::Wayland);
    assert_eq!(
        run(&mut wayland),
        [
            rune(Close),
            rune(ToggleMaximize),
            gap(Minimize, RuneMinimize),
            gap(resize, RuneResize),
            gap(WindowMenu, RuneWindowMenu),
        ]
    );
    let caps = wayland.capabilities();
    assert_eq!(
        (caps.resize, caps.minimize, caps.window_menu),
        (Some(false), Some(false), Some(false)),
        "so the app can hide what can't work"
    );
    assert_eq!(RuneResize.rune_draft(), Some("U6"));
    assert_eq!(RuneMinimize.rune_draft(), Some("U7"));
    assert_eq!(RuneWindowMenu.rune_draft(), Some("U8"));

    // A native frame on X11 follows the window's state, but the frame keeps resizing and the
    // window menu, whatever the window manager supports.
    let mut native_x11 = Csd::new(Frame::Native, Backend::X11);
    let mut report = Report::default();
    report.state.maximized = Some(true);
    report.capabilities = Capabilities {
        resize: Some(true),
        minimize: Some(true),
        maximize: Some(true),
        window_menu: Some(true),
    };
    native_x11.apply(report);
    assert_eq!(native_x11.state().maximized, Some(true));
    let caps = native_x11.capabilities();
    assert_eq!(
        (caps.resize, caps.window_menu, caps.minimize),
        (Some(false), Some(false), Some(true))
    );
    assert!(!native_x11.shows_bands() && !native_x11.shows_window_menu());

    // X11 before `attach` links a window.
    let mut x11 = borderless(Backend::X11);
    assert_eq!(
        run(&mut x11),
        [
            rune(Close),
            rune(ToggleMaximize),
            gap(Minimize, NoWindow),
            gap(resize, NoWindow),
            gap(WindowMenu, NoWindow),
        ]
    );
}

/// Once linked, an X11 request goes out unless the window manager says it can't. Only the
/// window menu waits to be known: `_GTK_SHOW_WINDOW_MENU` means nothing to a window manager
/// that doesn't list it, while any may honour `WM_CHANGE_STATE` and `_NET_WM_MOVERESIZE`.
#[test]
fn an_x11_request_goes_out_only_where_the_link_and_the_window_manager_allow() {
    use super::state::{Linking, gate};
    use Gap::*;
    let menu = Some(NotKnown);
    let cases = [
        (Linking::None, Some(true), None, Some(NoWindow)),
        (Linking::Failed, Some(true), None, Some(NoConnection)),
        (Linking::Linked, Some(false), None, Some(WindowManager)),
        (Linking::Linked, Some(false), menu, Some(WindowManager)),
        (Linking::Linked, None, None, None),
        (Linking::Linked, None, menu, Some(NotKnown)),
        (Linking::Linked, Some(true), menu, None),
    ];
    for (link, can, unknown, want) in cases {
        assert_eq!(
            gate(link, can, unknown),
            want,
            "{link:?}, can {can:?}, unknown {unknown:?}"
        );
    }
}

#[test]
fn a_failed_x11_connection_says_so_and_offers_nothing_that_needs_it() {
    use std::cell::RefCell;
    thread_local! {
        static HEARD: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }
    fn hear(_: &Csd, notice: Notice<'_>) {
        let line = match notice {
            Notice::Attached => "attached".to_owned(),
            Notice::Error(e) => format!("error: {e}"),
            Notice::Report | Notice::Outcome(_) => return,
        };
        HEARD.with(|h| h.borrow_mut().push(line));
    }
    let mut csd = borderless(Backend::X11).observe(hear);
    csd.attached(state::Native::X11 {
        window: 7,
        link: Err("connection refused".into()),
    });
    assert_eq!(
        HEARD.with(|h| h.borrow().clone()),
        ["attached", "error: no X11 connection: connection refused"]
    );
    assert_eq!(csd.connection_error(), Some("connection refused"));
    let caps = csd.capabilities();
    assert_eq!(
        (caps.resize, caps.minimize, caps.window_menu),
        (Some(false), Some(false), Some(false)),
        "so the app hides them"
    );
    assert!(!csd.shows_bands() && !csd.shows_window_menu());
    let mut tx = Tx::default();
    let no_connection = |request| Outcome::Unsupported {
        request,
        gap: Gap::NoConnection,
    };
    assert_eq!(csd.minimize(&mut tx), no_connection(Request::Minimize));
    assert_eq!(
        csd.begin_resize(Edge::South, None),
        no_connection(Request::Resize(Edge::South))
    );
    assert_eq!(csd.window_menu(), no_connection(Request::WindowMenu));
    assert_eq!(
        csd.close(&mut tx),
        Outcome::Sent {
            request: Request::Close,
            route: Route::Rune
        },
        "Close needs no X11 connection"
    );
    assert_eq!(Gap::NoConnection.rune_draft(), Some("U9"));

    // In a headless view, no band is mounted.
    let mut rt = runtime(csd);
    click(&mut rt, inside(Edge::East));
    assert_eq!(
        rt.state.csd.last_outcome().map(Outcome::request),
        Some(Request::Close)
    );
}

/// A re-created window replaces the link; a report the old window's watcher had already sent
/// arrives late, and is dropped.
#[test]
fn reports_from_another_window_are_dropped() {
    let maximized = |window| Report {
        window,
        state: WindowState {
            maximized: Some(true),
            fullscreen: Some(false),
        },
        ..Report::default()
    };
    let mut csd = borderless(Backend::X11);
    csd.attached(state::Native::X11 {
        window: 8,
        link: Err("closed".into()),
    });
    let revision = csd.revision();
    csd.apply(maximized(Some(7)));
    csd.apply(maximized(None));
    assert_eq!(
        (csd.revision(), csd.state().maximized),
        (revision, None),
        "dropped"
    );
    csd.apply(maximized(Some(8)));
    assert_eq!(
        csd.state().maximized,
        Some(true),
        "its own window's report lands"
    );
}

#[test]
fn an_observer_hears_each_outcome_and_report() {
    use std::cell::RefCell;
    thread_local! {
        static HEARD: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }
    fn hear(csd: &Csd, notice: Notice<'_>) {
        let line = match notice {
            Notice::Attached => "attached".to_owned(),
            Notice::Report => format!("report {:?}", csd.state().maximized),
            Notice::Outcome(o) => format!("outcome {:?}", o.request()),
            Notice::Error(e) => format!("error {e}"),
        };
        HEARD.with(|h| h.borrow_mut().push(line));
    }
    let mut csd = borderless(Backend::Headless).observe(hear);
    csd.minimize(&mut Tx::default());
    let mut report = Report::default();
    report.state.maximized = Some(true);
    csd.apply(report);
    csd.apply(report);
    assert_eq!(
        HEARD.with(|h| h.borrow().clone()),
        ["outcome Minimize", "report Some(true)"],
        "a report that changes nothing isn't heard"
    );
}

#[test]
fn auto_is_borderless_and_the_backend_is_the_one_winit_picks() {
    assert_eq!(Policy::Auto.frame(), Frame::Borderless);

    assert_eq!(
        pick(Some("wayland-1"), None, Some(":0")),
        Backend::Wayland,
        "Wayland first"
    );
    assert_eq!(pick(None, Some("5"), None), Backend::Wayland);
    assert_eq!(
        pick(Some(""), Some(""), Some(":0")),
        Backend::X11,
        "empty is unset"
    );
    assert_eq!(pick(None, None, Some("")), Backend::Headless);
}
