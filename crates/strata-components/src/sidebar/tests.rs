use std::borrow::Cow;
use std::rc::Rc;

use strata_kit::icons::IconName;

use super::build::gap_at;
use super::data::{DeviceLine, device_lines};
use super::*;
use crate::contract::dnd::{DropAction, DropActions, DropHover, DropOffer, URI_LIST};

fn offer(actions: DropActions, local: bool, serial: u64) -> DropOffer {
    let mimes: Rc<[Cow<'static, str>]> = Rc::from([Cow::Borrowed(URI_LIST)]);
    DropOffer::new(mimes, actions).local(local).serial(serial)
}

// browser/sidebar/rail: "each press of the header toggle or Ctrl+B steps the sidebar from
// full to icon rail to hidden, then back to full", and the tooltip naming the next step.
#[test]
fn the_toggle_steps_full_rail_hidden_full_and_names_the_next_step() {
    let mut w = Width::default();
    let mut seen = Vec::new();
    for _ in 0..3 {
        seen.push((w.shown(), w.next_label()));
        w.advance();
    }
    assert_eq!(
        seen,
        [
            (Step::Full, "Compact sidebar"),
            (Step::Rail, "Hide sidebar"),
            (Step::Hidden, "Show sidebar"),
        ]
    );
    assert_eq!(w.shown(), Step::Full);
}

// browser/sidebar/rail: "A rail chosen with the toggle stays when the window widens".
#[test]
fn a_chosen_rail_ignores_the_fit_rule() {
    let mut w = Width::default();
    w.advance();
    w.set_auto_rail(true);
    w.set_auto_rail(false);
    assert_eq!(w.shown(), Step::Rail);
}

// browser/sidebar/rail: "Choosing the rail while space has already railed the sidebar keeps the
// rail when the window widens again".
#[test]
fn choosing_the_rail_over_an_automatic_rail_keeps_it_when_room_returns() {
    let mut w = Width::default();
    w.set_auto_rail(true);
    assert_eq!(w.shown(), Step::Rail);
    w.choose(Step::Rail);
    w.set_auto_rail(false);
    assert_eq!(w.shown(), Step::Rail);
}

// browser/sidebar/rail: "stepping from hidden to full shows the full sidebar even when too
// narrow. It stays full until the window or the preview next runs out of room."
#[test]
fn stepping_from_hidden_shows_the_full_sidebar_until_the_fit_rule_next_rails_it() {
    let mut w = Width::default();
    w.set_auto_rail(true);
    w.advance();
    assert_eq!(w.shown(), Step::Hidden);
    w.advance();
    assert_eq!(w.shown(), Step::Full, "full, though the rule had railed it");
    w.set_auto_rail(true);
    assert_eq!(w.shown(), Step::Rail);
    assert_eq!(w.chosen(), Step::Full, "an automatic rail isn't a choice");
}

#[test]
fn a_hidden_sidebar_ignores_the_fit_rule() {
    let mut w = Width::default().with_step(Step::Hidden);
    w.set_auto_rail(true);
    assert_eq!(w.shown(), Step::Hidden);
    assert!(!w.auto_rail());
}

// browser/sidebar/rail: "Reopening the sidebar at a 10px text size restores at least its
// measured minimum width".
#[test]
fn the_full_width_comes_back_at_least_at_the_minimum() {
    let mut w = Width::new(40.0);
    assert_eq!(w.full_width(), MIN_WIDTH);
    w.pane.set_width(260.0);
    w.advance();
    w.advance();
    w.advance();
    assert_eq!(
        w.full_width(),
        260.0,
        "the rail and hidden steps keep the width"
    );
}

// drag-to-pin: "Holding Shift during a drag from Strata still pins the folder and moves
// nothing", and "A Move-only drag from another application is rejected by the gaps; a Copy
// drag from it is accepted."
#[test]
fn gaps_take_copies_and_local_moves_but_refuse_a_foreign_move_only_drag() {
    let both = DropActions::COPY | DropActions::MOVE;
    let unchecked = PinVerdict::Unchecked;
    assert_eq!(
        pin_action(&offer(both, true, 1), unchecked),
        Some(DropAction::Copy)
    );
    assert_eq!(
        pin_action(&offer(DropActions::MOVE, true, 1), unchecked),
        Some(DropAction::Move),
        "a local move pins, and Strata's sources never delete"
    );
    assert_eq!(
        pin_action(&offer(DropActions::COPY, false, 1), unchecked),
        Some(DropAction::Copy)
    );
    assert_eq!(
        pin_action(&offer(DropActions::MOVE, false, 1), unchecked),
        None
    );
    assert_eq!(
        pin_action(&offer(DropActions::LINK, false, 1), unchecked),
        None
    );
}

// drag-to-pin: "Dragging a file, or a selection mixing files and folders, over a gap shows no
// insertion line, and releasing it pins and transfers nothing."
#[test]
fn a_refused_drag_shows_no_line_and_is_refused() {
    let check = PinCheck {
        local: PinVerdict::Refused,
        ..PinCheck::default()
    };
    let o = offer(DropActions::COPY, true, 3);
    assert!(!check.shows(check.verdict(&o)));
    assert_eq!(pin_action(&o, check.verdict(&o)), None);
}

// drag-to-pin: "A folder dragged from another file manager shows the insertion line only once
// every item is confirmed to be a directory."
#[test]
fn under_wait_a_foreign_drag_shows_the_line_only_once_confirmed() {
    let mut check = PinCheck {
        unchecked: Unchecked::Wait,
        ..PinCheck::default()
    };
    let o = offer(DropActions::COPY, false, 7);
    assert!(!check.shows(check.verdict(&o)), "unchecked");
    check.foreign = Some((7, PinVerdict::Checking));
    assert!(!check.shows(check.verdict(&o)), "checking");
    check.foreign = Some((6, PinVerdict::Folders));
    assert!(!check.shows(check.verdict(&o)), "another drag's verdict");
    check.foreign = Some((7, PinVerdict::Folders));
    assert!(check.shows(check.verdict(&o)));
}

#[test]
fn the_gaps_read_the_verdict_on_the_drag_over_them() {
    let mut s = SidebarState::default();
    s.pin_check.foreign = Some((9, PinVerdict::Refused));
    s.pin_check.local = PinVerdict::Folders;
    s.note_drag(&offer(DropActions::COPY, false, 9));
    assert_eq!(s.gap_verdict(), PinVerdict::Refused);
    s.note_drag(&offer(DropActions::COPY, true, 10));
    assert_eq!(s.gap_verdict(), PinVerdict::Folders);
}

// browser/sidebar: "Only one sidebar drop site shows an indicator at a time".
#[test]
fn one_site_holds_the_indicator_and_a_late_leave_never_clears_another() {
    let mut s = SidebarState::default();
    let row = DropSite::Item(ItemId::Pin("a".into()));
    s.set_drop_hover(row.clone(), DropHover::Accepting);
    s.set_drop_hover(DropSite::Gap(1), DropHover::Accepting);
    s.set_drop_hover(row, DropHover::Left);
    assert_eq!(
        s.drop_over(),
        Some(&(DropSite::Gap(1), DropHover::Accepting))
    );
    s.set_drop_hover(DropSite::Gap(1), DropHover::Left);
    assert!(!s.dragging());
}

// drag-to-pin: "After another window unpins a gap's neighbour, a drop on that gap in a window
// that has not rebuilt lands beside the following pin, else the preceding one." The gap
// carries both neighbours for the host to resolve.
#[test]
fn a_gap_names_the_pins_on_either_side() {
    let pins = [Pin::new("a", "A"), Pin::new("b", "B")];
    let ids = |g: Gap| (g.index, g.before, g.after);
    assert_eq!(ids(gap_at(&pins, 0)), (0, None, Some("a".into())));
    assert_eq!(
        ids(gap_at(&pins, 1)),
        (1, Some("a".into()), Some("b".into()))
    );
    assert_eq!(ids(gap_at(&pins, 2)), (2, Some("b".into()), None));
    assert_eq!(ids(gap_at(&[], 0)), (0, None, None));
}

fn dev(id: &str) -> Device {
    Device::new(id, id, IconName::Drive, "/")
}

// devices/volumes/drive-order: "No divider shows above the first group or below the last", and
// "A pending-release stand-in for a device no longer listed follows every group, with no
// divider before it."
#[test]
fn dividers_sit_only_between_groups() {
    let groups = [
        DeviceGroup::new(vec![]),
        DeviceGroup::new(vec![dev("a"), dev("b")]),
        DeviceGroup::new(vec![dev("c")]),
        DeviceGroup::new(vec![]),
        DeviceGroup::new(vec![dev("ghost")]).divided(false),
    ];
    let lines: Vec<String> = device_lines(&groups)
        .into_iter()
        .map(|l| match l {
            DeviceLine::Divider(g) => format!("|{g}"),
            DeviceLine::Device(d) => d.id.clone(),
        })
        .collect();
    assert_eq!(lines, ["a", "b", "|2", "c", "ghost"]);
}

// browser/sidebar: "A collapsed DEVICES badge counts device rows only, not the dividers between
// drives: two partitions plus one other drive read `3`."
#[test]
fn the_devices_count_is_rows_not_dividers() {
    let groups = [
        DeviceGroup::new(vec![dev("p1"), dev("p2")]),
        DeviceGroup::new(vec![dev("usb")]),
    ];
    let data = SidebarData {
        devices: &groups,
        ..SidebarData::default()
    };
    assert_eq!(data.device_count(), 3);
}

fn gap_zone(check: PinCheck, verdict: PinVerdict) -> crate::contract::dnd::DropZone<SidebarState> {
    let emit: Emit<SidebarState> = Rc::new(|_, _, _| {});
    super::drops::gap(
        |s: &mut SidebarState| s,
        &emit,
        gap_at(&[], 0),
        verdict,
        &check,
    )
}

// drag-to-pin: "A folder dragged from another file manager shows the insertion line only once
// every item is confirmed to be a directory." Under `Wait`, the gap takes a drop only once the
// host confirmed it; under `Accept`, the host checks at the drop.
#[test]
fn under_wait_a_gap_refuses_a_drop_the_host_hasnt_confirmed() {
    let state = SidebarState::default();
    let o = offer(DropActions::COPY, false, 4);
    let mods = rune::Mods::default();
    let wait = PinCheck {
        unchecked: Unchecked::Wait,
        ..PinCheck::default()
    };
    for v in [
        PinVerdict::Unchecked,
        PinVerdict::Checking,
        PinVerdict::Refused,
    ] {
        assert_eq!(
            gap_zone(wait.clone(), v).choose(&state, &o, mods),
            None,
            "{v:?}"
        );
    }
    assert_eq!(
        gap_zone(wait, PinVerdict::Folders).choose(&state, &o, mods),
        Some(DropAction::Copy)
    );
    let accept = PinCheck::default();
    for v in [PinVerdict::Unchecked, PinVerdict::Checking] {
        assert_eq!(
            gap_zone(accept.clone(), v).choose(&state, &o, mods),
            Some(DropAction::Copy),
            "{v:?}"
        );
    }
    assert_eq!(
        gap_zone(accept, PinVerdict::Refused).choose(&state, &o, mods),
        None
    );
}
