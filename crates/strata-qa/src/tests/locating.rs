// SPDX-License-Identifier: MIT

use serde_json::json;

use super::{
    super::{Aim, LookupError, Mark, Rect, Remembered, Scope, Target},
    fake::{Fake, Node, WINDOW, drawn},
};

fn target(json: serde_json::Value) -> Target {
    serde_json::from_value(json).expect("the target should parse")
}

fn scope<'a>(fake: &'a Fake, kept: &'a Remembered<Fake>) -> Scope<'a, Fake> {
    Scope {
        backend: fake,
        window: WINDOW,
        kept,
    }
}

/// A pane `home` with one entry `notes`, which has an icon and a name label.
fn listing(fake: &mut Fake) -> usize {
    let pane = fake.add(
        WINDOW,
        Node {
            mark: Some(Mark::Pane("List")),
            ..drawn("grouping", "home", [0.0, 0.0, 400.0, 600.0])
        },
    );
    let list = fake.add(
        pane,
        Node {
            mark: Some(Mark::EntryContainer),
            ..drawn("list", "home", [0.0, 40.0, 400.0, 560.0])
        },
    );
    let row = fake.add(
        list,
        Node {
            mark: Some(Mark::Entry),
            ..drawn("list item", "notes", [0.0, 40.0, 400.0, 30.0])
        },
    );
    fake.add(row, drawn("image", "", [4.0, 44.0, 22.0, 22.0]));
    fake.add(row, drawn("label", "notes", [30.0, 44.0, 100.0, 22.0]));
    fake.add(row, drawn("label", "3 items", [300.0, 44.0, 90.0, 22.0]));
    row
}

#[test]
fn an_entry_target_aims_at_its_row_or_at_the_part_it_names() {
    let mut fake = Fake::new();
    let row = listing(&mut fake);
    let kept = Remembered::default();
    let scope = scope(&fake, &kept);
    let pixels = |json: serde_json::Value, aim: Aim| {
        let located = scope
            .locate_now(&target(json), None, aim)
            .expect("the target is on screen");
        (located.node, located.rect.pixels(), located.point)
    };

    assert_eq!(
        pixels(json!({ "entry": "notes" }), Aim::Point),
        (row, [0, 40, 400, 30], (200.0, 55.0))
    );
    assert_eq!(
        pixels(json!({ "entry": "notes", "part": "icon" }), Aim::Point).1,
        [4, 44, 22, 22]
    );
    assert_eq!(
        pixels(json!({ "entry": "notes", "part": "metadata" }), Aim::Point).1,
        [300, 44, 90, 22]
    );
    assert_eq!(
        pixels(json!({ "entry": "notes" }), Aim::DragOrigin),
        (row, [4, 44, 22, 22], (15.0, 55.0)),
        "a drag starts on the icon and still names the row"
    );
    assert_eq!(
        scope
            .locate_now(&target(json!({ "entry": "gone" })), None, Aim::Point)
            .map(|located| located.node),
        Err(LookupError::Missing(
            "no entry \"gone\" on screen".to_owned()
        ))
    );
}

/// Where a scroll pane stands against a pane's entry list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scroller {
    AroundList,
    InsideList,
    InsideEntry,
}

/// How the toolkit tells an entry's row.
#[derive(Clone, Copy, Debug)]
enum Row {
    /// The row is the node marked as an entry, as on Rune.
    Marked,
    /// The row is no marked node and the toolkit lists it as an entry, as GTK's row widget.
    Listed,
}

/// A pane `home` whose entry `notes`, a row as `row` says, is under scroll panes placed as
/// `scrollers` say, the outermost first; and those scroll panes, in the same order.
fn scrolled_listing(fake: &mut Fake, scrollers: &[Scroller], row: Row) -> Vec<usize> {
    let frame = [0.0, 40.0, 400.0, 560.0];
    let mut panes = Vec::new();
    let mut wrap = |fake: &mut Fake, mut parent: usize, at: Scroller| {
        for _ in scrollers.iter().filter(|placed| **placed == at) {
            parent = fake.add(parent, drawn("scroll pane", "", frame));
            panes.push(parent);
        }
        parent
    };
    let pane = Node {
        mark: Some(Mark::Pane("List")),
        ..drawn("grouping", "home", [0.0, 0.0, 400.0, 600.0])
    };
    let pane = fake.add(WINDOW, pane);
    let around = wrap(fake, pane, Scroller::AroundList);
    let list = Node {
        mark: Some(Mark::EntryContainer),
        ..drawn("list", "home", frame)
    };
    let list = fake.add(around, list);
    fake.add(list, drawn("label", "Loading", [0.0, 40.0, 400.0, 20.0]));
    let inside = wrap(fake, list, Scroller::InsideList);
    let entry = drawn("list item", "notes", [0.0, 40.0, 400.0, 30.0]);
    let entry = match row {
        Row::Marked => fake.add(
            inside,
            Node {
                mark: Some(Mark::Entry),
                ..entry
            },
        ),
        Row::Listed => {
            let entry = fake.add(inside, entry);
            fake.rows.push(entry);
            entry
        }
    };
    wrap(fake, entry, Scroller::InsideEntry);
    panes
}

#[test]
fn a_panes_viewport_is_the_scroll_pane_nearest_inside_its_list_else_around_it() {
    use Row::{Listed, Marked};
    use Scroller::{AroundList, InsideEntry, InsideList};
    // The scroll panes, outermost first, how the row is told, and which of the panes is the
    // pane's viewport.
    let cases: [(&str, &[Scroller], Row, Option<usize>); 8] = [
        (
            "GTK's scrolled window around the list",
            &[AroundList],
            Listed,
            Some(0),
        ),
        (
            "a Rune collection's, inside it",
            &[InsideList],
            Marked,
            Some(0),
        ),
        (
            "Rune's List: rows inside, a strip around",
            &[AroundList, InsideList],
            Marked,
            Some(1),
        ),
        (
            "of two inside, the nearer",
            &[InsideList, InsideList],
            Marked,
            Some(0),
        ),
        (
            "not the one inside a Rune row",
            &[AroundList, InsideEntry],
            Marked,
            Some(0),
        ),
        (
            "not the one inside a GTK row",
            &[AroundList, InsideEntry],
            Listed,
            Some(0),
        ),
        ("none of a Rune list's", &[InsideEntry], Marked, None),
        ("none of a GTK list's", &[InsideEntry], Listed, None),
    ];
    for (case, scrollers, row, wanted) in cases {
        let mut fake = Fake::new();
        let panes = scrolled_listing(&mut fake, scrollers, row);
        let kept = Remembered::default();
        let found = scope(&fake, &kept)
            .locate_now(&target(json!({ "viewport": null })), None, Aim::Point)
            .map(|located| located.node);
        let wanted = wanted.map(|at| panes[at]).ok_or_else(|| {
            LookupError::Missing("the entry list has no scroll viewport".to_owned())
        });
        assert_eq!(found, wanted, "{case}");
    }
}

#[test]
fn a_role_target_takes_the_nth_drawn_match_inside_its_scope() {
    let mut fake = Fake::new();
    let bar = fake.add(
        WINDOW,
        drawn("tool bar", "Actions", [0.0, 0.0, 800.0, 40.0]),
    );
    let first = fake.add(bar, drawn("button", "Open", [0.0, 0.0, 80.0, 40.0]));
    fake.add(
        bar,
        Node {
            role: "button",
            name: "Open",
            ..Node::default()
        },
    );
    let last = fake.add(bar, drawn("button", "Open", [200.0, 0.0, 80.0, 40.0]));
    fake.add(WINDOW, drawn("button", "Open", [0.0, 500.0, 80.0, 40.0]));
    let kept = Remembered::default();
    let scope = scope(&fake, &kept);
    let node = |json: serde_json::Value| scope.resolve(&target(json));

    assert_eq!(
        node(json!({ "role": "button", "name": "Open" })),
        Ok(Some(first))
    );
    let inside = json!({ "role": "tool bar", "name": "Actions" });
    assert_eq!(
        node(json!({ "role": "button", "name": "Open", "nth": -1, "within": inside })),
        Ok(Some(last)),
        "the undrawn button and the one outside the tool bar do not count"
    );
    assert_eq!(node(json!({ "role": "button", "name": "Close" })), Ok(None));
    assert_eq!(
        scope
            .locate_now(
                &target(json!({ "role": "button", "name": "Close" })),
                None,
                Aim::Point
            )
            .map(|located| located.node),
        Err(LookupError::Missing(
            "no button named \"Close\" on screen".to_owned()
        ))
    );
}

#[test]
fn what_was_never_kept_is_a_broken_lookup_and_not_a_missing_node() {
    let mut fake = Fake::new();
    listing(&mut fake);
    let kept = Remembered::default();
    kept.boxes.borrow_mut().insert(
        "spot".to_owned(),
        Rect {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
        },
    );
    let scope = scope(&fake, &kept);

    let located = scope
        .locate_now(&target(json!({ "kept": "spot" })), None, Aim::Point)
        .expect("a kept box needs no node");
    assert_eq!(
        (located.rect.pixels(), located.point),
        ([10, 20, 30, 40], (25.0, 40.0))
    );
    assert_eq!(
        scope.resolve(&target(json!({ "kept": "nothing" }))),
        Err("no box or point was kept as \"nothing\"".to_owned())
    );
    assert_eq!(
        scope.resolve(&target(json!({ "remembered": "nothing" }))),
        Err("no widget was remembered as \"nothing\"".to_owned())
    );
    assert_eq!(
        scope.resolve(&target(json!({ "dialog": true }))),
        Ok(None),
        "a dialog that is not open is only missing"
    );
}
