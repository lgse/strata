// SPDX-License-Identifier: MIT

use rune::{El, el};

use super::{Candidate, drawn_buttons};
use crate::{
    App,
    slots::{Entry, Frame, Place},
    test_support::{hosted_over, idle_view, view::idle},
};

const MODULE: &str = "qa::stage::tests";

fn candidate(visible: [f32; 4], opacity: f32) -> Candidate<'static> {
    Candidate {
        role: "button",
        key: Some("location-crumb-0"),
        label: Some("documents"),
        rect: [10.0, 4.0, 60.0, 24.0],
        visible,
        opacity,
        exiting: false,
    }
}

/// A button a user could press is presented with its drawn box; one scrolled wholly out of
/// its viewport, or drawn at no opacity, is not.
#[test]
fn only_a_button_in_view_and_drawn_is_presented() {
    let in_view = candidate([10.0, 4.0, 60.0, 24.0], 1.0).button();
    assert_eq!(
        in_view.map(|button| button.name),
        Some("documents".to_owned())
    );

    let scrolled_away = candidate([10.0, 4.0, 0.0, 24.0], 1.0).button();
    assert_eq!(scrolled_away, None, "a crumb scrolled out of its strip");

    let transparent = candidate([10.0, 4.0, 60.0, 24.0], 0.0).button();
    assert_eq!(transparent, None, "a button drawn at no opacity");
}

/// A column's pane, named for its folder, with its list and a header button inside, and a
/// button outside it, as a sidebar's beside a column scrolled out of the strip.
fn panes_and_buttons(_: &App, _: &mut Frame<'_, App>) -> Option<El> {
    let button = |key: &str| el().key(key).role("button").label(key).w(20.0).h(20.0);
    let pane = el()
        .label("folder")
        .w(300.0)
        .h(200.0)
        .child(el().key("made-list").w(300.0).h(100.0))
        .child(button("made-refresh"));
    Some(el().key("made-view").child(pane).child(button("made-pin")))
}

fn made_parts() -> Vec<Entry<App>> {
    struct View;
    vec![Entry {
        name: "made-view",
        kind: std::any::TypeId::of::<View>,
        place: Place::View(idle_view("made-view", panes_and_buttons, |_| true)),
        listen: |_| None,
        publish: |_| {},
        caught_up: |_| {},
        poll: |_, _, _| false,
        switched: |_, _| {},
        commands: &[],
        keyboard: |_| None,
    }]
}

/// A button belongs to the column whose pane is among its ancestors, whatever the boxes say: a
/// column scrolled out of the strip keeps its full box, over the sidebar for one, and a button
/// drawn there stays outside it.
#[test]
fn a_button_belongs_to_the_column_its_pane_holds_in_the_element_tree() {
    let name = "a_button_belongs_to_the_column_its_pane_holds_in_the_element_tree";
    hosted_over(MODULE, name, made_parts, |mut host| {
        idle(&mut host);
        let mut elements = host.elements();
        let keyed = |key: &str| {
            let keyed = |element: &rune::ElementInfo| element.key.as_deref() == Some(key);
            elements.iter().position(keyed).expect(key)
        };
        let (list, pin) = (keyed("made-list"), keyed("made-pin"));
        let [x, y, ..] = elements[list].rect;
        elements[pin].rect = [x + 40.0, y + 40.0, 20.0, 20.0];
        elements[pin].visible = elements[pin].rect;

        let buttons = drawn_buttons(&elements, &["made-list".to_owned()]);

        let columns: Vec<_> = buttons
            .iter()
            .filter(|button| button.key.starts_with("made-"))
            .map(|button| (button.key.as_str(), button.column.as_deref()))
            .collect();
        assert_eq!(
            columns,
            [("made-refresh", Some("made-list")), ("made-pin", None)]
        );
    });
}
