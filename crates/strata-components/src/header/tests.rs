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
    let framed = |maximized, fullscreen| {
        WindowChrome {
            maximized,
            fullscreen,
            ..WindowChrome::default()
        }
        .framed()
    };
    assert!(framed(Some(false), Some(false)));
    assert!(!framed(Some(true), Some(false)));
    assert!(!framed(Some(false), Some(true)));
    assert!(framed(None, None), "unknown (Wayland) keeps the line");
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
