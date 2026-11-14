// SPDX-License-Identifier: MIT

use serde_json::json;

use super::{headless, private};

fn output(name: &str, active: bool) -> serde_json::Value {
    json!({ "name": name, "active": active, "rect": { "x": 0, "y": 0, "width": 1440, "height": 900 } })
}

#[test]
fn only_a_sway_whose_every_output_is_headless_is_taken() {
    let private_sway = json!([output("HEADLESS-1", true)]);
    assert_eq!(headless(&private_sway), Ok((1440, 900)));

    for desktop in [
        json!([output("DP-1", true)]),
        json!([output("HEADLESS-1", true), output("eDP-1", false)]),
    ] {
        let refused = headless(&desktop).expect_err("a desktop's outputs");
        assert!(refused.contains("not a headless one"), "{refused}");
    }
    assert!(headless(&json!([])).is_err(), "no output");
}

#[test]
fn a_variable_into_the_desktops_runtime_directory_is_refused() {
    let session = |bus: &str| {
        [
            ("XDG_RUNTIME_DIR", Some("/tmp/strata-session-1".to_owned())),
            ("WAYLAND_DISPLAY", Some("wayland-1".to_owned())),
            ("DBUS_SESSION_BUS_ADDRESS", Some(bus.to_owned())),
        ]
    };
    assert_eq!(
        private(&session("unix:path=/tmp/strata-session-1/bus")),
        Ok(())
    );
    assert_eq!(private(&[("DBUS_SESSION_BUS_ADDRESS", None)]), Ok(()));

    let refused = private(&session("unix:path=/run/user/1000/bus")).expect_err("the desktop's bus");
    assert!(
        refused.starts_with("DBUS_SESSION_BUS_ADDRESS="),
        "{refused}"
    );
}
