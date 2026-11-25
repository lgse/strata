// SPDX-License-Identifier: MIT

use strata_kit::icons::IconName;

use super::{HeaderLabels, Shown, WindowButtons, WindowChrome};

// app/window: "A fresh profile shows only the Close button; Minimize and Maximize are hidden."
#[test]
fn a_fresh_profile_shows_only_close() {
    let shown = Shown::new(WindowButtons::default(), &WindowChrome::default());
    assert_eq!(
        (shown.minimize, shown.maximize, shown.close),
        (false, false, true)
    );
}

#[test]
fn a_button_hides_only_when_the_window_manager_says_it_cant_work() {
    let chrome = |can: Option<bool>| WindowChrome {
        can_minimize: can,
        can_maximize: can,
        ..WindowChrome::default()
    };
    let shown = |can| {
        let s = Shown::new(WindowButtons::ALL, &chrome(can));
        (s.minimize, s.maximize)
    };
    assert_eq!(shown(None), (true, true), "unknown still shows");
    assert_eq!(shown(Some(true)), (true, true));
    assert_eq!(shown(Some(false)), (false, false));

    // Wayland, as strata-platform reports it: minimise can't work, maximise is unknown.
    let wayland = WindowChrome {
        maximized: None,
        can_minimize: Some(false),
        can_maximize: None,
        ..WindowChrome::default()
    };
    let s = Shown::new(WindowButtons::ALL, &wayland);
    assert_eq!((s.minimize, s.maximize, s.close), (false, true, true));
}

#[test]
fn maximize_names_the_state_it_knows_and_fits_both_when_it_doesnt() {
    let labels = HeaderLabels::default();
    assert_eq!(
        labels.maximize_face(Some(true)),
        (IconName::Restore, "Restore window")
    );
    assert_eq!(
        labels.maximize_face(Some(false)),
        (IconName::Maximize, "Maximize window")
    );
    assert_eq!(
        labels.maximize_face(None),
        (IconName::Maximize, "Maximize or restore window")
    );
}

// app/window: "A maximized, fullscreen, or tiled client-decorated window draws no border."
#[test]
fn the_frame_follows_what_the_window_manager_says() {
    use rune::{DecorationMode, TiledEdges};

    let framed = |maximized, fullscreen, decoration_mode, tiled_edges| {
        WindowChrome {
            maximized,
            fullscreen,
            decoration_mode,
            tiled_edges,
            ..WindowChrome::default()
        }
        .framed()
    };
    assert!(framed(
        Some(false),
        Some(false),
        DecorationMode::Client,
        Some(TiledEdges::NONE)
    ));
    assert!(!framed(
        Some(true),
        Some(false),
        DecorationMode::Client,
        None
    ));
    assert!(!framed(
        Some(false),
        Some(true),
        DecorationMode::Client,
        None
    ));
    assert!(!framed(
        Some(false),
        Some(false),
        DecorationMode::Server,
        None
    ));
    assert!(!framed(
        Some(false),
        Some(false),
        DecorationMode::Client,
        Some(TiledEdges::new(true, false, true, true))
    ));
    assert!(
        framed(None, None, DecorationMode::Unknown, None),
        "unknown keeps the line"
    );
}

#[test]
fn resize_bands_follow_wm_state_and_omit_tiled_edges() {
    use rune::{
        Cx, DecorationMode, El, Env, ResizeEdge, Runtime, TiledEdges, WindowManagerState, el,
    };

    use crate::header::{WindowChrome, resize_bands};

    struct Host {
        borderless: bool,
    }

    fn view(host: &Host, cx: &mut Cx<Host>) -> El {
        let chrome = WindowChrome::from_cx(cx, true);
        resize_bands(el().w_full().h_full(), host.borderless, &chrome)
    }

    let mut rt = Runtime::new(
        Host { borderless: true },
        view,
        (400.0, 300.0),
        Env::new(false, 1.0),
    );
    let band_keys = |rt: &Runtime<Host>| -> Vec<String> {
        let mut keys: Vec<String> = rt
            .elements()
            .into_iter()
            .filter(|e| !e.exiting)
            .filter_map(|e| e.key)
            .filter(|k| k.starts_with("window-resize-") && k != "window-resize-frame")
            .collect();
        keys.sort();
        keys
    };

    rt.frame(0.0);
    assert_eq!(band_keys(&rt).len(), 8);

    // Pressing the south-east band triggers Rune's native resize action.
    rt.pointer_move(398.0, 298.0);
    rt.pointer_down(398.0, 298.0);
    assert_eq!(rt.take_resize_action(), Some(ResizeEdge::SouthEast));
    rt.pointer_up(398.0, 298.0);

    // Left half-tiled window keeps only the untiled east edge.
    rt.set_window_manager_state(
        WindowManagerState::default()
            .with_decoration_mode(DecorationMode::Client)
            .with_tiled_edges(Some(TiledEdges::new(true, false, true, true))),
    );
    rt.frame(0.0);
    assert_eq!(band_keys(&rt), vec!["window-resize-e"]);

    // Server-decorated, maximized, or fullscreen windows mount no resize bands.
    rt.set_window_manager_state(
        WindowManagerState::default().with_decoration_mode(DecorationMode::Server),
    );
    rt.frame(0.0);
    assert!(band_keys(&rt).is_empty());

    rt.set_window_manager_state(
        WindowManagerState::default().with_decoration_mode(DecorationMode::Client),
    );
    rt.set_maximized(true);
    rt.frame(0.0);
    assert!(band_keys(&rt).is_empty());

    rt.set_maximized(false);
    rt.set_full_screen(true);
    rt.frame(0.0);
    assert!(band_keys(&rt).is_empty());

    rt.set_full_screen(false);
    rt.update(|host, _| host.borderless = false);
    rt.frame(0.0);
    assert!(band_keys(&rt).is_empty());
}

mod bar {
    use rune::{Cx, El, Env, Runtime, Tx};
    use strata_kit::look::Look;

    use crate::header::{Header, HeaderLabels, WindowAction, WindowButtons, WindowChrome};

    struct Host {
        look: Look,
    }

    fn view(h: &Host, _: &mut Cx<Host>) -> El {
        let labels = HeaderLabels::default();
        let route = |_: &mut Host, _: WindowAction, _: &mut Tx| {};
        Header::new("h", &h.look)
            .window_buttons(WindowButtons::ALL, &WindowChrome::default(), &labels, route)
            .window_buttons(
                WindowButtons::default(),
                &WindowChrome::default(),
                &labels,
                route,
            )
            .build()
    }

    #[test]
    fn calling_window_buttons_again_replaces_the_buttons() {
        let host = Host {
            look: Look::fallback(false),
        };
        let mut rt = Runtime::new(host, view, (800.0, 100.0), Env::new(false, 1.0));
        rt.frame(0.0);
        let count = |key: &str| {
            rt.elements()
                .iter()
                .filter(|e| e.key.as_deref() == Some(key))
                .count()
        };
        assert_eq!(
            (count("h-minimize"), count("h-maximize"), count("h-close")),
            (0, 0, 1)
        );
    }
}
