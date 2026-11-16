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
