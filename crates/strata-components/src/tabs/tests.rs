// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_kit::icons::IconName;

use super::*;
use crate::contract::dnd::{
    DropAction, DropActions, DropData, DropHover, DropOffer, Dropped, URI_LIST,
};

fn tabs(n: u64) -> Vec<Tab<'static>> {
    (1..=n)
        .map(|i| Tab::new(TabId(i), format!("f{i}"), IconName::Folder))
        .collect()
}

// browser/tabs: "Ctrl+Tab and Ctrl+Page Down select the next tab; Ctrl+Shift+Tab and Ctrl+Page
// Up select the previous one. Each wraps at either end."
#[test]
fn step_wraps_at_either_end() {
    let t = tabs(3);
    let last = TabsData::new(&t, Some(TabId(3)));
    assert_eq!(last.step(1), Some(TabId(1)));
    let first = TabsData::new(&t, Some(TabId(1)));
    assert_eq!(first.step(-1), Some(TabId(3)));
    assert_eq!(TabsData::new(&t, None).step(1), None);
}

// browser/tabs: "Ctrl+Shift+Page Up and Ctrl+Shift+Page Down move the active tab one position";
// "Moving stops at either end of the strip instead of wrapping."
#[test]
fn moving_stops_at_the_ends() {
    let t = tabs(3);
    let d = TabsData::new(&t, Some(TabId(3)));
    assert_eq!(d.moved(TabId(3), 1), None);
    assert_eq!(d.moved(TabId(3), -1), Some(1));
    assert_eq!(d.moved(TabId(1), -1), None);
}

// browser/tabs: "Closing the active tab selects its right neighbour, or its left neighbour when
// it was last."
#[test]
fn closing_hands_over_to_the_right_then_the_left() {
    let t = tabs(3);
    assert_eq!(
        TabsData::new(&t, Some(TabId(2))).after_close(TabId(2)),
        Some(TabId(3))
    );
    assert_eq!(
        TabsData::new(&t, Some(TabId(3))).after_close(TabId(3)),
        Some(TabId(2))
    );
    // An inactive tab's close leaves the active one.
    assert_eq!(
        TabsData::new(&t, Some(TabId(1))).after_close(TabId(3)),
        Some(TabId(1))
    );
    let one = tabs(1);
    assert_eq!(
        TabsData::new(&one, Some(TabId(1))).after_close(TabId(1)),
        None
    );
}

// browser/tabs: "Names longer than 22 characters are ellipsized."
#[test]
fn names_past_22_characters_end_in_an_ellipsis() {
    let fits = "a".repeat(22);
    assert_eq!(elide_end(&fits, LABEL_CHARS), fits.as_str());
    let long = "Quarterly reports and invoices";
    let short = elide_end(long, LABEL_CHARS);
    assert_eq!(short, "Quarterly reports and…");
    assert_eq!(short.chars().count(), LABEL_CHARS);
    // Characters, not bytes.
    let wide = "äöü".repeat(8);
    assert_eq!(elide_end(&wide, LABEL_CHARS).chars().count(), LABEL_CHARS);
}

// browser/tabs: "A window whose active tab is at folder docs is titled docs — Strata"; "A window
// with no folder open and no location awaited is titled Strata"; "The title shows a folder
// name over 22 characters in full".
#[test]
fn the_title_carries_the_whole_name_or_the_app_alone() {
    let f = TitleFormat::default();
    assert_eq!(window_title(Some("docs"), &f), "docs — Strata");
    assert_eq!(window_title(None, &f), "Strata");
    assert_eq!(window_title(Some(""), &f), "Strata");
    let long = "a folder name well over twenty-two characters";
    assert_eq!(window_title(Some(long), &f), format!("{long} — Strata"));
}

// browser/tabs: "Repeated Ctrl+Shift+T reopens closed tabs newest first, up to 20; a 21st close
// drops the oldest entry."
#[test]
fn the_stack_keeps_twenty_newest_first() {
    let mut closed = ClosedTabs::new();
    for i in 1..=21 {
        closed.record(i, 0);
    }
    assert_eq!(closed.len(), CLOSED_TABS_KEPT);
    let order: Vec<_> = std::iter::from_fn(|| closed.reopen().map(|c| c.item)).collect();
    assert_eq!(order, (2..=21).rev().collect::<Vec<_>>());
    assert_eq!(closed.reopen(), None);
}

// browser/tabs: "When fewer tabs are open than the former position, the reopened tab stays at
// the right end."
#[test]
fn a_reopened_tab_goes_back_to_its_place_or_the_end() {
    let mut closed = ClosedTabs::new();
    closed.record("b", 4);
    let c = closed.reopen().unwrap();
    assert_eq!(c.index_in(6), 4);
    assert_eq!(c.index_in(2), 2);
}

fn tick(st: &mut BrowserTabsState, now: f64, d: &TabsData<'_>) -> Vec<TabsIntent> {
    let mut tx = Tx::default();
    st.tick(now, d, &mut tx)
        .intents
        .into_iter()
        .filter(|i| !matches!(i, TabsIntent::Title(_)))
        .collect()
}

fn sprung(id: u64) -> Vec<TabsIntent> {
    vec![TabsIntent::Select {
        id: TabId(id),
        via: Via::Spring,
    }]
}

// browser/tabs: "Holding a file or browser-content drag over a tab for 450 ms selects it". The
// same hover reported again (X2's binder does, on each motion) keeps the spring's start.
#[test]
fn a_repeated_hover_on_the_same_tab_keeps_its_start() {
    let t = tabs(3);
    let d = TabsData::new(&t, Some(TabId(1)));
    let mut st = BrowserTabsState::default();
    assert!(st.hover_tab(TabId(2), DropHover::Accepting, Some(HOVER_DELAY), 1.0));
    assert!(tick(&mut st, 1.0, &d).is_empty());
    assert!(!st.hover_tab(TabId(2), DropHover::Accepting, Some(HOVER_DELAY), 1.3));
    assert!(tick(&mut st, 1.3, &d).is_empty());
    assert!(!st.hover_tab(TabId(2), DropHover::Accepting, Some(HOVER_DELAY), 1.45));
    assert_eq!(tick(&mut st, 1.45, &d), sprung(2));
    assert_eq!(st.spring(), None, "it fires once");
}

#[test]
fn another_tab_restarts_the_wait_and_a_leave_ends_it() {
    let t = tabs(3);
    let d = TabsData::new(&t, Some(TabId(1)));
    let mut st = BrowserTabsState::default();
    st.hover_tab(TabId(2), DropHover::Accepting, Some(HOVER_DELAY), 1.0);
    let _ = tick(&mut st, 1.0, &d);
    st.hover_tab(TabId(2), DropHover::Left, None, 1.3);
    st.hover_tab(TabId(3), DropHover::Accepting, Some(HOVER_DELAY), 1.3);
    assert!(tick(&mut st, 1.3, &d).is_empty());
    assert!(
        tick(&mut st, 1.5, &d).is_empty(),
        "tab 3's wait began at 1.3"
    );
    assert_eq!(tick(&mut st, 1.75, &d), sprung(3));
    st.hover_tab(TabId(2), DropHover::Accepting, Some(HOVER_DELAY), 2.0);
    let _ = tick(&mut st, 2.0, &d);
    st.hover_tab(TabId(2), DropHover::Left, None, 3.0);
    assert!(tick(&mut st, 3.0, &d).is_empty(), "a leave ends the wait");
}

// browser/tabs: "A folder-only drag held over a tab does not select it" (the zone arms with no
// delay for it), and a tab that became active, or closed, while the drag rested springs nothing.
#[test]
fn nothing_springs_unarmed_or_for_a_tab_now_active_or_gone() {
    let t = tabs(3);
    let mut st = BrowserTabsState::default();
    assert!(!st.hover_tab(TabId(2), DropHover::Accepting, None, 0.0));
    assert_eq!(st.spring(), None);
    st.hover_tab(TabId(2), DropHover::Accepting, Some(HOVER_DELAY), 1.0);
    let _ = tick(&mut st, 1.0, &TabsData::new(&t, Some(TabId(1))));
    assert!(tick(&mut st, 2.0, &TabsData::new(&t, Some(TabId(2)))).is_empty());
    st.hover_tab(TabId(3), DropHover::Accepting, Some(HOVER_DELAY), 3.0);
    let _ = tick(&mut st, 3.0, &TabsData::new(&t, Some(TabId(1))));
    assert!(tick(&mut st, 4.0, &TabsData::new(&t[..2], Some(TabId(1)))).is_empty());
}

// browser/tabs: "Navigating the active tab, or selecting another tab, retitles the window";
// "A background tab that navigates leaves the window title unchanged."
#[test]
fn the_title_is_emitted_once_per_change_of_the_active_tab() {
    let titles = |st: &mut BrowserTabsState, d: &TabsData<'_>| -> Vec<String> {
        let mut tx = Tx::default();
        st.tick(0.0, d, &mut tx)
            .intents
            .into_iter()
            .filter_map(|i| match i {
                TabsIntent::Title(t) => Some(t),
                _ => None,
            })
            .collect()
    };
    let mut st = BrowserTabsState::default();
    let mut t = tabs(2);
    assert_eq!(
        titles(&mut st, &TabsData::new(&t, Some(TabId(1)))),
        ["f1 — Strata"]
    );
    assert!(titles(&mut st, &TabsData::new(&t, Some(TabId(1)))).is_empty());
    t[1].name = "elsewhere".into();
    assert!(titles(&mut st, &TabsData::new(&t, Some(TabId(1)))).is_empty());
    t[0].name = "docs".into();
    assert_eq!(
        titles(&mut st, &TabsData::new(&t, Some(TabId(1)))),
        ["docs — Strata"]
    );
    assert_eq!(
        titles(&mut st, &TabsData::new(&t, Some(TabId(2)))),
        ["elsewhere — Strata"]
    );
}

#[test]
fn a_leave_clears_the_drop_hover_only_where_it_is() {
    let mut st = BrowserTabsState::default();
    st.set_drop_hover(DropSite::Tab(TabId(1)), DropHover::Accepting);
    st.set_drop_hover(DropSite::Tab(TabId(2)), DropHover::Left);
    assert_eq!(
        st.drop_hover(),
        Some((DropSite::Tab(TabId(1)), DropHover::Accepting))
    );
    st.set_drop_hover(DropSite::Tab(TabId(1)), DropHover::Left);
    assert_eq!(st.drop_hover(), None);
}

struct MiniHost {
    state: BrowserTabsState,
    open: Option<TabOpen>,
    heard: Vec<TabsIntent>,
}

fn mini_ctx() -> drops::Ctx<MiniHost> {
    drops::Ctx {
        access: |h| &mut h.state,
        emit: std::rc::Rc::new(|h, intent, _| h.heard.push(intent)),
        action: Some(std::rc::Rc::new(|_, _, _, _| Some(DropAction::Move))),
        opens: Some(std::rc::Rc::new(|h, offer| {
            offer.local.then_some(h.open)?
        })),
        guard_none: true,
        content: &[],
        spring: Some(HOVER_DELAY),
        len: 3,
    }
}

fn local_offer() -> DropOffer {
    DropOffer::new(vec![URI_LIST.into()], DropActions::COPY | DropActions::MOVE).local(true)
}

#[test]
fn view_folder_drags_transfer_on_tabs_refuse_trash_and_open_at_end_on_strip() {
    let cx = mini_ctx();
    let offer = local_offer();
    let mods = rune::Mods::default();
    let folder_tab = drops::tab(
        &cx,
        drops::TabSpec {
            id: TabId(2),
            index: 1,
            accepts: Accepts::ALL,
            active: false,
        },
    )
    .expect("folder tab zone");
    let trash_tab = drops::tab(
        &cx,
        drops::TabSpec {
            id: TabId(3),
            index: 2,
            accepts: Accepts::NONE,
            active: false,
        },
    )
    .expect("trash tab guard zone");
    let strip = drops::opener(&cx, DropSite::Strip);
    let mut host = MiniHost {
        state: BrowserTabsState::default(),
        open: Some(TabOpen::AtEnd),
        heard: Vec::new(),
    };

    assert_eq!(
        folder_tab.choose(&host, &offer, mods),
        Some(DropAction::Move)
    );
    assert_eq!(trash_tab.choose(&host, &offer, mods), None);
    assert!(
        trash_tab.rejects(),
        "stops AtEnd from falling through to strip"
    );
    assert_eq!(strip.choose(&host, &offer, mods), Some(DropAction::Copy));

    let mut tx = Tx::default();
    let dropped = Dropped::new(offer.clone(), DropData::default(), mods);
    folder_tab.deliver(&mut host, dropped.clone(), &mut tx);
    strip.deliver(&mut host, dropped.clone(), &mut tx);
    assert!(matches!(
        host.heard.as_slice(),
        [
            TabsIntent::DropInto { id: TabId(2), .. },
            TabsIntent::OpenTabs { at: 3, .. },
        ]
    ));

    host.open = Some(TabOpen::AtGap);
    host.heard.clear();
    assert_eq!(
        trash_tab.choose(&host, &offer, mods),
        Some(DropAction::Copy)
    );
    trash_tab.deliver(&mut host, dropped, &mut tx);
    assert!(matches!(
        host.heard.as_slice(),
        [TabsIntent::OpenTabs { at: 3, .. }]
    ));
}

#[test]
fn words_names_each_tabs_close_button() {
    struct State {
        strip: BrowserTabsState,
        look: strata_kit::look::Look,
    }
    fn view(s: &State, cx: &mut rune::Cx<State>) -> rune::El {
        let t = tabs(2);
        let d = TabsData::new(&t, Some(TabId(1)));
        BrowserTabs::new(
            &d,
            &s.strip,
            |s: &mut State| &mut s.strip,
            |s| &mut s.strip.drag,
        )
        .words(TabWords {
            close: "Fermer l’onglet",
            ..TabWords::default()
        })
        .build(&s.look, cx)
    }
    let state = State {
        strip: BrowserTabsState::default(),
        look: strata_kit::look::Look::default(),
    };
    let mut rt = rune::Runtime::new(state, view, (600.0, 48.0), rune::Env::default());
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    let close_buttons: Vec<_> = rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting && e.role == "button" && e.label.as_deref() != Some("New tab"))
        .map(|e| e.label.unwrap_or_default())
        .collect();
    assert_eq!(close_buttons, ["Fermer l’onglet", "Fermer l’onglet"]);
}

// Packet entry 108: a `Tx::element_at` callback inside `Runtime::frame` sees `tx.clock() == 0.0`
// even when the window's clock is seconds ahead; the spring stamps its start on the next tick.
#[test]
fn a_zero_clock_pick_hover_waits_the_full_delay_from_next_tick() {
    let t = tabs(3);
    let d = TabsData::new(&t, Some(TabId(1)));
    let mut st = BrowserTabsState::default();
    assert!(st.hover_tab(TabId(2), DropHover::Accepting, Some(HOVER_DELAY), 0.0));
    assert!(tick(&mut st, 2.5, &d).is_empty());
    assert!(tick(&mut st, 2.8, &d).is_empty());
    assert_eq!(tick(&mut st, 2.5 + HOVER_DELAY, &d), sprung(2));
}

#[test]
fn width_cap_keeps_fitting_tabs_natural_and_shrinks_wide_tabs_first() {
    let cases: &[(&str, &[f32], Option<f32>, f32)] = &[
        ("unknown viewport", &[72.0, 155.0], None, 240.0),
        ("empty strip", &[], Some(400.0), 240.0),
        ("all tabs fit", &[72.0, 155.0], Some(600.0), 240.0),
        // Total 412 > 350: short 72 stays natural, remaining 278 splits across two wide tabs -> 139.
        (
            "wide tabs shrink while short stays compact",
            &[72.0, 160.0, 180.0],
            Some(350.0),
            139.0,
        ),
        // Too many tabs: clamps at min_w so horizontal scrolling takes over.
        (
            "overflow clamps at min cap",
            &[72.0, 160.0, 180.0, 180.0],
            Some(350.0),
            128.0,
        ),
    ];
    for &(label, natural, available, expected) in cases {
        let got = sizing::width_cap(natural, available, 0.0, (128.0, 240.0));
        assert!(
            (got - expected).abs() < 0.01,
            "{label}: expected {expected}, got {got}"
        );
    }
}

#[test]
fn variable_width_drag_reorders_narrow_and_wide_tabs_and_cancels_on_escape() {
    let widths = [155.0_f32, 74.0];
    // Drag narrow tab 1 left to the centre of wide tab 0.
    assert_eq!(drag::target_index(&widths, 0.0, 1, -114.5), 0);
    // Drag wide tab 0 right across narrow tab 1.
    assert_eq!(drag::target_index(&widths, 0.0, 0, 74.0), 1);

    // Incoming landing slot uses each tab's actual midpoint.
    assert_eq!(drag::landing_slot(&widths, 0.0, [50.0, 20.0], 0.0), 0);
    assert_eq!(drag::landing_slot(&widths, 0.0, [100.0, 20.0], 0.0), 1);
    assert_eq!(drag::landing_slot(&widths, 0.0, [210.0, 20.0], 0.0), 2);

    // ReorderState lifecycle: below slop stays Click; past slop emits Reorder; cancelled emits None.
    let press = |state: &mut drag::ReorderState| {
        state.press(drag::PressSpec {
            from: 1,
            grab: [20.0, 10.0],
            widths: widths.to_vec(),
            gap: 0.0,
            tear: None,
        });
    };
    let mut state = drag::ReorderState::default();
    press(&mut state);
    assert!(!state.follow(-2.0, 0.0), "motion inside slop stays pressed");
    assert!(matches!(state.release(false, true), drag::Release::Click));

    press(&mut state);
    assert!(state.follow(-114.5, 0.0));
    assert!(matches!(
        state.release(false, false),
        drag::Release::Reorder { from: 1, to: 0 }
    ));

    press(&mut state);
    assert!(state.follow(-114.5, 0.0));
    assert!(matches!(state.release(true, false), drag::Release::None));
    assert!(!state.take_swallowed_click());

    // Cancelling a drag with Escape while the pointer is still over the tab must NOT leave a
    // stale `swallow_click` behind to eat the next keyboard activation.
    press(&mut state);
    assert!(state.follow(-114.5, 0.0));
    assert!(matches!(state.release(true, true), drag::Release::None));
    assert!(
        !state.take_swallowed_click(),
        "cancelled drag over a tab must not swallow subsequent activation"
    );
}

#[test]
fn escape_cancelled_drag_over_tab_does_not_swallow_keyboard_activation() {
    struct State {
        strip: BrowserTabsState,
        look: strata_kit::look::Look,
        intents: Vec<TabsIntent>,
    }
    fn view(s: &State, cx: &mut rune::Cx<State>) -> rune::El {
        let items = [
            Tab::new(TabId(1), "Applications", IconName::Folder),
            Tab::new(TabId(2), "~", IconName::Home),
        ];
        let d = TabsData::new(&items, Some(TabId(1)));
        BrowserTabs::new(
            &d,
            &s.strip,
            |s: &mut State| &mut s.strip,
            |s| &mut s.strip.drag,
        )
        .on_intent(|s, intent, _| s.intents.push(intent))
        .build(&s.look, cx)
    }
    let state = State {
        strip: BrowserTabsState::default(),
        look: strata_kit::look::Look::default(),
        intents: Vec::new(),
    };
    let mut rt = rune::Runtime::new(state, view, (600.0, 48.0), rune::Env::default());
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);

    let tab1 = rt
        .elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some("t-tabs-1"))
        .expect("tab 1");
    let (x, y) = (
        tab1.visible[0] + 24.0,
        tab1.visible[1] + tab1.visible[3] / 2.0,
    );
    // Start a reorder drag on tab 1, move slightly past slop while remaining inside tab 1, and
    // cancel with Escape.
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_move(x + 10.0, y);
    rt.key(rune::KeyEvent {
        key: rune::Key::Escape,
        mods: rune::Mods::default(),
    });
    rt.pointer_up(x + 10.0, y);
    rt.frame(0.0);
    rt.state.intents.clear();

    // Focus tab 2 and activate it from the keyboard with Space; it must not be swallowed.
    rt.update(|_, tx| tx.focus("t-tabs-2", true));
    rt.frame(0.0);
    rt.key(rune::KeyEvent {
        key: rune::Key::Text(" ".to_owned()),
        mods: rune::Mods::default(),
    });
    rt.frame(0.0);
    assert_eq!(
        rt.state.intents,
        [TabsIntent::Select {
            id: TabId(2),
            via: Via::Strip,
        }]
    );
}

#[test]
fn layered_tear_out_anchors_ghost_and_hides_source_face_until_hand_off() {
    struct State {
        strip: BrowserTabsState,
        look: strata_kit::look::Look,
        intents: Vec<TabsIntent>,
    }
    fn view(s: &State, cx: &mut rune::Cx<State>) -> rune::El {
        let items = [
            Tab::new(TabId(1), "Applications", IconName::Folder),
            Tab::new(TabId(2), "~", IconName::Home),
        ];
        let d = TabsData::new(&items, Some(TabId(1)));
        let mut layer = Vec::new();
        let band = BrowserTabs::new(
            &d,
            &s.strip,
            |s: &mut State| &mut s.strip,
            |s| &mut s.strip.drag,
        )
        .tear_out(true)
        .on_intent(|s, intent, _| s.intents.push(intent))
        .build_layered(&s.look, cx, &mut layer);
        rune_kit::root(band, layer)
    }
    let state = State {
        strip: BrowserTabsState::default(),
        look: strata_kit::look::Look::default(),
        intents: Vec::new(),
    };
    let mut rt = rune::Runtime::new(state, view, (600.0, 200.0), rune::Env::default());
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);

    let tab2 = rt
        .elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some("t-tabs-2"))
        .expect("tab 2");
    let (x, y) = (
        tab2.visible[0] + 24.0,
        tab2.visible[1] + tab2.visible[3] / 2.0,
    );
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    // Drag 50 pt vertically off the strip (> 36 pt detach threshold).
    rt.pointer_move(x, y + 50.0);
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);

    let els = rt.elements();
    assert!(
        els.iter()
            .any(|e| !e.exiting && e.key.as_deref() == Some("tabs-ghost")),
        "floating ghost mounts in overlay layer"
    );
    assert!(
        els.iter()
            .any(|e| !e.exiting && e.key.as_deref() == Some("tabs-anchor")),
        "anchor mounts in source tab"
    );
    let source = els
        .iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some("t-tabs-2"))
        .expect("source tab stays mounted for pointer capture");
    assert!(
        source.opacity < 0.01,
        "source tab hides while floating ghost flies"
    );

    // Handing off to a new window removes the floating ghost while keeping source hidden.
    rt.update(|s, _| s.strip.hand_off());
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);
    let els = rt.elements();
    assert!(
        els.iter()
            .all(|e| e.exiting || e.key.as_deref() != Some("tabs-ghost")),
        "hand_off removes floating ghost"
    );
    let source = els
        .iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some("t-tabs-2"))
        .expect("source tab stays mounted until release");
    assert!(
        source.opacity < 0.01,
        "source tab stays hidden after hand_off"
    );
}

#[test]
fn tab_strip_pointer_actions_emit_select_close_and_new_tab_intents() {
    struct State {
        strip: BrowserTabsState,
        look: strata_kit::look::Look,
        intents: Vec<TabsIntent>,
    }
    fn view(s: &State, cx: &mut rune::Cx<State>) -> rune::El {
        let items = [
            Tab::new(TabId(1), "Applications", IconName::Folder),
            Tab::new(TabId(2), "~", IconName::Home),
        ];
        let d = TabsData::new(&items, Some(TabId(1)));
        BrowserTabs::new(
            &d,
            &s.strip,
            |s: &mut State| &mut s.strip,
            |s| &mut s.strip.drag,
        )
        .on_intent(|s, intent, _| s.intents.push(intent))
        .build(&s.look, cx)
    }
    let state = State {
        strip: BrowserTabsState::default(),
        look: strata_kit::look::Look::default(),
        intents: Vec::new(),
    };
    let mut rt = rune::Runtime::new(state, view, (600.0, 48.0), rune::Env::default());
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);

    let click_at = |rt: &mut rune::Runtime<State>, x: f32, y: f32| {
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(0.0);
    };

    // Click inactive tab 2 (on its left label area, clear of its close button).
    let tab2 = rt
        .elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some("t-tabs-2"))
        .expect("tab 2");
    click_at(
        &mut rt,
        tab2.visible[0] + 24.0,
        tab2.visible[1] + tab2.visible[3] / 2.0,
    );
    assert_eq!(
        rt.state.intents,
        [TabsIntent::Select {
            id: TabId(2),
            via: Via::Strip,
        }]
    );

    // Click tab 2's close button: emits Close without Select.
    rt.state.intents.clear();
    let close2 = rt
        .elements()
        .into_iter()
        .filter(|e| !e.exiting && e.key.as_deref() == Some("close"))
        .nth(1)
        .expect("tab 2 close button");
    click_at(
        &mut rt,
        close2.visible[0] + close2.visible[2] / 2.0,
        close2.visible[1] + close2.visible[3] / 2.0,
    );
    assert_eq!(rt.state.intents, [TabsIntent::Close { id: TabId(2) }]);

    // Click New tab (+) button.
    rt.state.intents.clear();
    let new_btn = rt
        .elements()
        .into_iter()
        .find(|e| !e.exiting && e.key.as_deref() == Some("tabs-new"))
        .expect("new tab button");
    click_at(
        &mut rt,
        new_btn.visible[0] + new_btn.visible[2] / 2.0,
        new_btn.visible[1] + new_btn.visible[3] / 2.0,
    );
    assert_eq!(rt.state.intents, [TabsIntent::New]);
}

#[test]
fn custom_tab_icons_mount_in_strip_and_yield_to_number_badges() {
    struct State {
        strip: BrowserTabsState,
        look: strata_kit::look::Look,
        badges: bool,
    }
    fn view(s: &State, cx: &mut rune::Cx<State>) -> rune::El {
        let red = s.look.kit.c.danger.icon;
        let blue = s.look.kit.c.info.icon;
        let items = [
            Tab::new(TabId(1), "Code", TabIcon::Catalog("strata-terminal".into())).tint(Some(red)),
            Tab::new(TabId(2), "Launch", TabIcon::Emoji("🚀".into())),
            Tab::new(TabId(3), "Home", IconName::Home).tint(Some(blue)),
        ];
        let d = TabsData::new(&items, Some(TabId(1)));
        BrowserTabs::new(
            &d,
            &s.strip,
            |s: &mut State| &mut s.strip,
            |s| &mut s.strip.drag,
        )
        .badges(s.badges)
        .build(&s.look, cx)
    }
    let state = State {
        strip: BrowserTabsState::default(),
        look: strata_kit::look::Look::default(),
        badges: false,
    };
    let mut rt = rune::Runtime::new(state, view, (600.0, 48.0), rune::Env::default());
    rt.frame(0.0);
    rt.frame(1.0 / 60.0);

    let els = rt.elements();
    let tab_icons = els
        .iter()
        .filter(|e| {
            !e.exiting
                && e.key.as_deref() == Some("icon")
                && e.parent
                    .and_then(|p| els.get(p))
                    .is_some_and(|parent| parent.key.as_deref() == Some("avatar"))
        })
        .count();
    assert_eq!(
        tab_icons, 3,
        "every tab mounts its customized or semantic icon inside its avatar"
    );

    rt.update(|s, _| s.badges = true);
    rt.frame(0.0);
    let els = rt.elements();
    let badge_count = els
        .iter()
        .filter(|e| !e.exiting && e.key.as_deref() == Some("badge"))
        .count();
    let avatar_count = els
        .iter()
        .filter(|e| !e.exiting && e.key.as_deref() == Some("avatar"))
        .count();
    assert_eq!(
        badge_count, 3,
        "number badges replace custom icons while active"
    );
    assert_eq!(avatar_count, 0);
}
