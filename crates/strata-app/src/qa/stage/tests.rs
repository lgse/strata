// SPDX-License-Identifier: MIT

use super::Candidate;

fn candidate(visible: [f32; 4], opacity: f32, inert: bool) -> Candidate<'static> {
    Candidate {
        role: "button",
        key: Some("location-crumb-0"),
        label: Some("documents"),
        rect: [10.0, 4.0, 60.0, 24.0],
        visible,
        opacity,
        inert,
        exiting: false,
    }
}

/// A button a user could press is presented with its drawn box; one scrolled wholly out of
/// its viewport, inert, or drawn at no opacity is not.
#[test]
fn only_a_button_in_view_and_not_inert_is_presented() {
    let in_view = candidate([10.0, 4.0, 60.0, 24.0], 1.0, false).button();
    assert_eq!(
        in_view.map(|button| button.name),
        Some("documents".to_owned())
    );

    let scrolled_away = candidate([10.0, 4.0, 0.0, 24.0], 1.0, false).button();
    assert_eq!(scrolled_away, None, "a crumb scrolled out of its strip");

    let inert = candidate([10.0, 4.0, 60.0, 24.0], 1.0, true).button();
    assert_eq!(inert, None, "an inert button");

    let transparent = candidate([10.0, 4.0, 60.0, 24.0], 0.0, false).button();
    assert_eq!(transparent, None, "a button drawn at no opacity");
}
