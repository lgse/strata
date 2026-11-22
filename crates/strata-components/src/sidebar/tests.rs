// SPDX-License-Identifier: MIT

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

struct MountedHost {
    sidebar: SidebarState,
    look: strata_kit::look::Look,
    places: Vec<Place>,
    pins: Vec<Pin>,
    groups: Vec<DeviceGroup>,
    intents: Vec<SidebarIntent>,
}

fn mounted_view(h: &MountedHost, _: &mut rune::Cx<MountedHost>) -> rune::El {
    let data = SidebarData {
        places: &h.places,
        pins: &h.pins,
        devices: &h.groups,
        labels: SidebarLabels {
            pinned: "PINNED",
            devices: "DEVICES",
            rows: |n| format!("{n} items"),
            ..SidebarLabels::default()
        },
        ..SidebarData::default()
    };
    PlacesSidebar::new(&h.sidebar, |h: &mut MountedHost| &mut h.sidebar, data)
        .on_intent(|h, intent, _| h.intents.push(intent))
        .build(&h.look)
}

fn mount_sidebar() -> rune::Runtime<MountedHost> {
    let look = strata_kit::look::Look::default();
    let host = MountedHost {
        sidebar: SidebarState::new(&look),
        look,
        places: vec![Place::folder("home", "Home", IconName::Home)],
        pins: vec![Pin::new("docs", "Docs")],
        groups: vec![DeviceGroup::new(vec![
            dev("mounted").mounted(true),
            dev("unmounted").mounted(false),
        ])],
        intents: Vec::new(),
    };
    let mut rt = rune::Runtime::new(host, mounted_view, (260.0, 500.0), rune::Env::default());
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    rt
}

fn element_center<S: 'static>(rt: &rune::Runtime<S>, key: &str) -> (f32, f32) {
    let e = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some(key))
        .expect(key);
    (
        e.visible[0] + e.visible[2] / 2.0,
        e.visible[1] + e.visible[3] / 2.0,
    )
}

#[test]
fn row_press_middle_click_and_keyboard_activation_route_cleanly() {
    use rune::{Key, KeyEvent, Mods, PointerButton};

    let mut rt = mount_sidebar();
    let (hx, hy) = element_center(&rt, "places-place-home");
    rt.pointer_move(hx, hy);
    rt.pointer_down(hx, hy);
    rt.pointer_up(hx, hy);
    rt.frame(1.0 / 60.0);
    rt.button_down(hx, hy, PointerButton::Middle);
    rt.button_up(hx, hy, PointerButton::Middle);
    rt.frame(1.0 / 60.0);

    let (ux, uy) = element_center(&rt, "places-device-unmounted");
    rt.pointer_move(ux, uy);
    rt.button_down(ux, uy, PointerButton::Middle);
    rt.button_up(ux, uy, PointerButton::Middle);
    rt.frame(1.0 / 60.0);

    // An off-row drag release leaves `dropped` set; keyboard Enter/Space still opens via Keyboard.
    let home = ItemId::Place("home".into());
    rt.state.sidebar.dropped = Some(home.clone());
    rt.update(|_, tx| tx.focus("places-place-home", true));
    rt.frame(1.0 / 60.0);
    for key in [Key::Enter, Key::Text(" ".into())] {
        rt.key(KeyEvent {
            key,
            mods: Mods::default(),
        });
        rt.frame(1.0 / 60.0);
    }
    rt.update(|_, tx| tx.focus("places-heading-pinned", true));
    rt.frame(1.0 / 60.0);
    rt.key(KeyEvent {
        key: Key::Enter,
        mods: Mods::default(),
    });
    rt.frame(1.0 / 60.0);

    assert_eq!(
        rt.state.intents,
        [
            SidebarIntent::Open {
                item: home.clone(),
                via: Via::Pointer,
            },
            SidebarIntent::OpenInNewTab { item: home.clone() },
            SidebarIntent::Open {
                item: home.clone(),
                via: Via::Keyboard,
            },
            SidebarIntent::Open {
                item: home,
                via: Via::Keyboard,
            },
            SidebarIntent::SectionToggled {
                section: Section::Pinned,
                collapsed: true,
            },
        ]
    );
}

#[test]
fn row_drag_outside_sidebar_reports_start_move_and_outside_release() {
    let look = strata_kit::look::Look::default();
    let host = MountedHost {
        sidebar: SidebarState::new(&look),
        look,
        places: vec![Place::folder("home", "Home", IconName::Home)],
        pins: vec![Pin::new("docs", "Docs")],
        groups: Vec::new(),
        intents: Vec::new(),
    };
    let mut rt = rune::Runtime::new(
        host,
        |h: &MountedHost, _: &mut rune::Cx<MountedHost>| {
            let data = SidebarData {
                places: &h.places,
                pins: &h.pins,
                devices: &h.groups,
                labels: SidebarLabels {
                    pinned: "PINNED",
                    devices: "DEVICES",
                    rows: |n| format!("{n} items"),
                    ..SidebarLabels::default()
                },
                ..SidebarData::default()
            };
            PlacesSidebar::new(&h.sidebar, |h: &mut MountedHost| &mut h.sidebar, data)
                .reorder(true)
                .on_intent(|h, intent, _| h.intents.push(intent))
                .build(&h.look)
        },
        (260.0, 500.0),
        rune::Env::default(),
    );
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    let (hx, hy) = element_center(&rt, "places-place-home");
    rt.pointer_move(hx, hy);
    rt.pointer_down(hx, hy);
    rt.pointer_move(hx + 24.0, hy);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(360.0, hy);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(360.0, hy);
    rt.frame(1.0 / 60.0);

    let home = ItemId::Place("home".into());
    assert!(
        rt.state
            .intents
            .contains(&SidebarIntent::RowDragStart { item: home.clone() })
    );
    assert!(
        rt.state
            .intents
            .iter()
            .any(|intent| matches!(intent, SidebarIntent::RowDragMove { item, landing: None, .. } if *item == home))
    );
    assert_eq!(
        rt.state.intents.last(),
        Some(&SidebarIntent::RowDragEnd {
            item: home,
            dropped_outside: Some([360.0, hy]),
        })
    );
}

struct ResizeHost {
    sidebar: SidebarState,
    look: strata_kit::look::Look,
    places: Vec<Place>,
    confined: bool,
    show_overlay: bool,
    bg_drags: usize,
    file_drags: usize,
    overlay_presses: usize,
}

fn resize_view(h: &ResizeHost, _: &mut rune::Cx<ResizeHost>) -> rune::El {
    let data = SidebarData {
        places: &h.places,
        ..SidebarData::default()
    };
    let sidebar = PlacesSidebar::new(&h.sidebar, |h: &mut ResizeHost| &mut h.sidebar, data)
        .confined_grip(h.confined)
        .on_background_drag(|h, d, _| {
            if d.phase == rune::Phase::Down {
                h.bg_drags += 1;
            }
        })
        .build(&h.look);
    let files = rune::col().key("files-pane").grow(1.0).h_full().on_drag(
        |h: &mut ResizeHost, d: &mut rune::Drag, _| {
            if d.phase == rune::Phase::Down {
                h.file_drags += 1;
            }
        },
    );
    let window = rune::row()
        .key("split")
        .w_full()
        .h_full()
        .child(sidebar)
        .child(files);
    let mut layer = Vec::new();
    if h.show_overlay {
        let edge = h.sidebar.width.full_width();
        layer.push(
            rune::el()
                .key("overlay-menu")
                .absolute(edge - 24.0, 200.0)
                .size(120.0, 120.0)
                .fill(h.look.kit.c.surface_raised)
                .on_press(|h: &mut ResizeHost| h.overlay_presses += 1),
        );
    }
    rune_kit::root(window, layer)
}

fn mount_resize(confined: bool, show_overlay: bool) -> rune::Runtime<ResizeHost> {
    let look = strata_kit::look::Look::default();
    let host = ResizeHost {
        sidebar: SidebarState::new(&look),
        look,
        places: vec![Place::folder("home", "Home", IconName::Home)],
        confined,
        show_overlay,
        bg_drags: 0,
        file_drags: 0,
        overlay_presses: 0,
    };
    let mut rt = rune::Runtime::new(host, resize_view, (800.0, 500.0), rune::Env::default());
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    rt
}

fn drag_horizontal(rt: &mut rune::Runtime<ResizeHost>, from_x: f32, to_x: f32, y: f32) {
    rt.pointer_move(from_x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_down(from_x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(to_x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(to_x, y);
    rt.frame(1.0 / 60.0);
}

#[test]
fn wide_grip_resizes_clamps_cancels_resets_and_leaves_file_pane_untouched() {
    use rune::{Cursor, Key, KeyEvent, Mods};

    let mut rt = mount_resize(false, false);
    let default = rt.state.sidebar.width.pane.default_width();
    let y = 250.0;

    // Clicking or dragging inside the adjacent file pane (2 pt past the divider) stays in the
    // file pane rather than resizing the sidebar.
    drag_horizontal(&mut rt, default + 2.0, default + 42.0, y);
    assert_eq!(rt.state.file_drags, 1);
    assert_eq!(rt.state.sidebar.width.full_width(), default);

    // Grabbing inside the 7 pt trailing grip takes the resize cursor and resizes cleanly.
    rt.pointer_move(default - 5.0, y);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.cursor(), Cursor::ResizeColumn);
    drag_horizontal(&mut rt, default - 5.0, default + 35.0, y);
    assert_eq!(rt.state.sidebar.width.full_width(), default + 40.0);
    assert_eq!(rt.state.bg_drags, 0);

    // Dragging left past MIN_WIDTH clamps at MIN_WIDTH.
    let now = rt.state.sidebar.width.full_width();
    drag_horizontal(&mut rt, now - 3.0, now - 303.0, y);
    assert_eq!(rt.state.sidebar.width.full_width(), MIN_WIDTH);

    // Dragging right past MAX_WIDTH clamps at MAX_WIDTH.
    drag_horizontal(&mut rt, MIN_WIDTH - 2.0, MIN_WIDTH + 498.0, y);
    assert_eq!(rt.state.sidebar.width.full_width(), MAX_WIDTH);

    // Pressing Escape mid-drag restores the starting width and clears resizing state.
    let start_x = MAX_WIDTH - 2.0;
    rt.pointer_move(start_x, y);
    rt.pointer_down(start_x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(start_x - 60.0, y);
    rt.frame(1.0 / 60.0);
    assert!(rt.state.sidebar.resizing);
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    rt.pointer_up(start_x - 60.0, y);
    for _ in 0..40 {
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(rt.state.sidebar.width.full_width(), MAX_WIDTH);
    assert!(!rt.state.sidebar.resizing);

    // Double-clicking the grip resets to the default width.
    let reset_x = MAX_WIDTH - 3.0;
    rt.pointer_move(reset_x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_down(reset_x, y);
    rt.pointer_up(reset_x, y);
    rt.frame(0.1);
    rt.pointer_down(reset_x, y);
    rt.pointer_up(reset_x, y);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.sidebar.width.full_width(), default);
}

#[test]
fn confined_grip_leaves_adjacent_whitespace_to_marquee_and_file_pane() {
    use rune::Cursor;

    let mut rt = mount_resize(true, false);
    let edge = rt.state.sidebar.width.full_width();
    let y = 250.0;

    // 2 pt left of the divider reaches the sidebar's background drag instead of resizing.
    drag_horizontal(&mut rt, edge - 2.0, edge + 38.0, y);
    assert_eq!(rt.state.bg_drags, 1);
    assert_eq!(rt.state.sidebar.width.full_width(), edge);

    // 2 pt right of the divider reaches the sibling file pane instead of resizing.
    drag_horizontal(&mut rt, edge + 2.0, edge + 42.0, y);
    assert_eq!(rt.state.file_drags, 1);
    assert_eq!(rt.state.sidebar.width.full_width(), edge);

    // The 1 pt trailing border still resizes.
    rt.pointer_move(edge - 0.5, y);
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.cursor(), Cursor::ResizeColumn);
    drag_horizontal(&mut rt, edge - 0.5, edge + 29.5, y);
    assert_eq!(rt.state.sidebar.width.full_width(), edge + 30.0);
}

#[test]
fn rail_and_hidden_modes_expose_no_resize_grip() {
    use rune::Cursor;

    let mut rt = mount_resize(false, false);
    let initial = rt.state.sidebar.width.full_width();
    let rail_edge = strata_kit::slots::sidebar::rail_width(&rt.state.look);
    let y = 250.0;

    rt.update(|h, _| h.sidebar.width.choose(Step::Rail));
    for _ in 0..40 {
        rt.frame(1.0 / 60.0);
    }
    rt.pointer_move(rail_edge, y);
    rt.frame(1.0 / 60.0);
    assert_ne!(rt.cursor(), Cursor::ResizeColumn);
    drag_horizontal(&mut rt, rail_edge, rail_edge + 50.0, y);
    assert_eq!(rt.state.sidebar.width.full_width(), initial);

    rt.update(|h, _| h.sidebar.width.choose(Step::Hidden));
    for _ in 0..40 {
        rt.frame(1.0 / 60.0);
    }
    assert!(
        rt.elements()
            .into_iter()
            .all(|e| e.key.as_deref() != Some("places-grip"))
    );
}

#[test]
fn overlay_above_sidebar_wins_pointer_hits_over_wide_grip() {
    let mut rt = mount_resize(false, true);
    let edge = rt.state.sidebar.width.full_width();
    let y = 250.0;

    // Clicking and dragging on the grip coordinates covered by an overlay hits the overlay,
    // not the sidebar's resize grip.
    drag_horizontal(&mut rt, edge - 3.0, edge + 37.0, y);
    assert_eq!(rt.state.overlay_presses, 1);
    assert_eq!(rt.state.sidebar.width.full_width(), edge);
}

#[test]
fn device_eject_button_heading_toggle_and_pending_row_behave_cleanly() {
    let look = strata_kit::look::Look::default();
    let host = MountedHost {
        sidebar: SidebarState::new(&look),
        look,
        places: vec![Place::folder("home", "Home", IconName::Home)],
        pins: vec![Pin::new("docs", "Docs")],
        groups: vec![DeviceGroup::new(vec![
            dev("usb").mounted(true).release(Release::Eject),
            dev("busy").mounted(true).pending(true),
        ])],
        intents: Vec::new(),
    };
    let mut rt = rune::Runtime::new(host, mounted_view, (260.0, 500.0), rune::Env::default());
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);

    // Clicking the eject button emits Eject without opening the row or triggering resize.
    let (ex, ey) = element_center(&rt, "places-device-usb-eject");
    rt.pointer_move(ex, ey);
    rt.pointer_down(ex, ey);
    rt.pointer_up(ex, ey);
    rt.frame(1.0 / 60.0);

    // Clicking a pending-release row emits nothing because it is disabled.
    let (bx, by) = element_center(&rt, "places-device-busy");
    rt.pointer_move(bx, by);
    rt.pointer_down(bx, by);
    rt.pointer_up(bx, by);
    rt.frame(1.0 / 60.0);

    // Clicking the DEVICES heading toggles the section cleanly.
    let (dx, dy) = element_center(&rt, "places-heading-devices");
    rt.pointer_move(dx, dy);
    rt.pointer_down(dx, dy);
    rt.pointer_up(dx, dy);
    rt.frame(1.0 / 60.0);

    assert_eq!(
        rt.state.intents,
        [
            SidebarIntent::Eject("usb".into()),
            SidebarIntent::SectionToggled {
                section: Section::Devices,
                collapsed: true,
            },
        ]
    );
}
