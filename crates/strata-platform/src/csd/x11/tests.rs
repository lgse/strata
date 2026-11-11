//! What the window manager's properties say the app can do: the report the watcher sends.

use super::{Atom, Atoms, report_from};

/// Atoms numbered 1… in declaration order; the values only need to differ.
fn atoms() -> Atoms {
    let mut n: Atom = 0;
    let mut next = || {
        n += 1;
        n
    };
    Atoms {
        wm_change_state: next(),
        net_supported: next(),
        net_wm_moveresize: next(),
        net_wm_state: next(),
        net_wm_state_maximized_horz: next(),
        net_wm_state_maximized_vert: next(),
        net_wm_state_fullscreen: next(),
        net_wm_allowed_actions: next(),
        net_wm_action_resize: next(),
        net_wm_action_minimize: next(),
        net_wm_action_maximize_horz: next(),
        net_wm_action_maximize_vert: next(),
        gtk_show_window_menu: next(),
    }
}

#[test]
fn without_an_ewmh_window_manager_nothing_is_known_but_the_state() {
    let a = atoms();
    let r = report_from(&a, &[a.net_wm_state_fullscreen], None, None);
    assert_eq!(r.capabilities, Default::default());
    assert_eq!(
        (r.state.maximized, r.state.fullscreen),
        (Some(false), Some(true))
    );
}

/// wlroots' X11 window manager: `_NET_WM_MOVERESIZE` and the maximised states, no window menu,
/// no allowed actions.
#[test]
fn a_window_manager_that_lists_no_window_menu_or_allowed_actions_reports_what_it_lists() {
    let a = atoms();
    let supported = [
        a.net_wm_moveresize,
        a.net_wm_state_maximized_vert,
        a.net_wm_state_maximized_horz,
    ];
    let r = report_from(&a, &[a.net_wm_state_maximized_horz], Some(&supported), None);
    let c = r.capabilities;
    assert_eq!(
        (c.resize, c.maximize, c.window_menu),
        (Some(true), Some(true), Some(false))
    );
    assert_eq!(c.minimize, None, "WM_CHANGE_STATE may still work");
    assert_eq!(r.state.maximized, Some(false), "maximised one way only");
}

/// mutter's kind: a window menu, and allowed actions that can take resizing away (a maximised
/// window, or one of fixed size).
#[test]
fn allowed_actions_narrow_what_the_window_manager_supports() {
    let a = atoms();
    let supported = [
        a.net_wm_moveresize,
        a.gtk_show_window_menu,
        a.net_wm_state_maximized_vert,
    ];
    let maximized = [a.net_wm_state_maximized_horz, a.net_wm_state_maximized_vert];
    let allowed = [a.net_wm_action_minimize];
    let r = report_from(&a, &maximized, Some(&supported), Some(&allowed));
    let c = r.capabilities;
    assert_eq!(r.state.maximized, Some(true));
    assert_eq!(
        (c.resize, c.minimize, c.maximize, c.window_menu),
        (Some(false), Some(true), Some(false), Some(true))
    );
}
