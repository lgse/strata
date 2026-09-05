// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use crate::ui::entry_list_model::EntryListModel;
use gtk::glib;

#[test]
fn recursive_search_arrows_select_and_clamp_results() {
    assert_eq!(search_result_navigation_position(None, 3, 1), Some(0));
    assert_eq!(search_result_navigation_position(None, 3, -1), Some(2));
    assert_eq!(search_result_navigation_position(Some(0), 3, -1), Some(0));
    assert_eq!(search_result_navigation_position(Some(1), 3, 1), Some(2));
    assert_eq!(search_result_navigation_position(Some(2), 3, 1), Some(2));
    assert_eq!(search_result_navigation_position(None, 0, 1), None);
}

#[test]
fn recursive_search_activation_accepts_enter_and_right_arrow() {
    assert!(recursive_search_activation_key(gtk::gdk::Key::Return));
    assert!(recursive_search_activation_key(gtk::gdk::Key::KP_Enter));
    assert!(recursive_search_activation_key(gtk::gdk::Key::Right));
    assert!(!recursive_search_activation_key(gtk::gdk::Key::Left));
    assert!(!recursive_search_activation_key(gtk::gdk::Key::Down));
}

fn mapped_source(values: &[&str]) -> EntryListModel {
    let owned: Vec<String> = values.iter().map(|value| (*value).to_owned()).collect();
    let model = EntryListModel::new(std::rc::Rc::new(move |position| {
        owned.get(position as usize).cloned()
    }));
    model.replace(values.len() as u32);
    model
}

#[test]
fn unfiltered_visible_listings_keep_source_order() {
    let source = mapped_source(&["fv\ta", "fv\tb", "fv\tc"]);
    let map = rebuild_position_map(&source, "", true, 1);
    assert_eq!(map.forward, vec![0, 1, 2]);
    assert_eq!(map.reverse, vec![0, 1, 2]);
}

#[test]
fn hidden_entries_are_omitted_from_the_position_map() {
    let source = mapped_source(&["fv\talpha", "dh\t.secret", "fv\tzulu"]);
    let map = rebuild_position_map(&source, "", false, 1);
    assert_eq!(map.forward, vec![0, 2]);
    assert_eq!(map.reverse[0], 0);
    assert_eq!(map.reverse[1], NO_FILTERED_POSITION);
    assert_eq!(map.reverse[2], 1);
}

#[test]
fn filter_queries_keep_matches_near_the_end() {
    let source = mapped_source(&["fv\talpha", "fv\tbeta", "fv\tzulu"]);
    let map = rebuild_position_map(&source, "zu", true, 4);
    assert_eq!(map.forward, vec![2]);
    assert_eq!(map.query, "zu");
    assert_eq!(map.generation, 4);
}

#[test]
fn filter_change_for_classifies_tightening_and_loosening() {
    assert_eq!(filter_change_for("", "a"), gtk::FilterChange::MoreStrict);
    assert_eq!(filter_change_for("a", "ab"), gtk::FilterChange::MoreStrict);
    assert_eq!(filter_change_for("ab", "a"), gtk::FilterChange::LessStrict);
    assert_eq!(filter_change_for("a", ""), gtk::FilterChange::LessStrict);
    assert_eq!(filter_change_for("a", "b"), gtk::FilterChange::Different);
    assert_eq!(filter_change_for("ab", "ac"), gtk::FilterChange::Different);
}

const FILTER_QUERY_GTK_CHILD: &str = "STRATA_FILTER_QUERY_GTK_CHILD";

const FILTER_QUERY_TEST: &str =
    "ui::browser::collection::tests::notify_filter_query_skips_unchanged_folded_text";

fn assert_notify_filter_query_skips_unchanged_folded_text() {
    use gtk::prelude::*;
    use std::{cell::Cell, rc::Rc};

    let filter = gtk::CustomFilter::new(|_| true);
    let emissions = Rc::new(Cell::new(0u32));
    let emissions_for_signal = emissions.clone();
    filter.connect_changed(move |_, _| {
        emissions_for_signal.set(emissions_for_signal.get() + 1);
    });
    let query = std::cell::RefCell::new(String::new());

    notify_filter_query(&filter, &query, "Abc".into());
    assert_eq!(query.borrow().as_str(), "abc");
    assert_eq!(emissions.get(), 1);

    notify_filter_query(&filter, &query, "ABC".into());
    assert_eq!(query.borrow().as_str(), "abc");
    assert_eq!(emissions.get(), 1);
}

#[test]
fn notify_filter_query_skips_unchanged_folded_text() {
    if std::env::var_os(FILTER_QUERY_GTK_CHILD).is_some() {
        if gtk::init().is_err() {
            return;
        }
        assert_notify_filter_query_skips_unchanged_folded_text();
        return;
    }

    let status =
        std::process::Command::new(std::env::current_exe().expect("test executable should exist"))
            .args(["--exact", FILTER_QUERY_TEST])
            .env(FILTER_QUERY_GTK_CHILD, "1")
            .status()
            .expect("isolated GTK filter-query test should start");
    assert!(status.success(), "isolated GTK filter-query test failed");
}

#[test]
#[ignore = "requires a mapped GTK window; run this test alone"]
fn seeded_filter_keeps_first_character_when_typing_continues() {
    const CHILD: &str = "STRATA_SEEDED_FILTER_GTK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(
            std::env::current_exe().expect("test executable should exist"),
        )
        .args([
            "--exact",
            "ui::browser::collection::tests::seeded_filter_keeps_first_character_when_typing_continues",
            "--ignored",
        ])
        .env(CHILD, "1")
        .status()
        .expect("isolated seeded filter test should start");
        assert!(status.success());
        return;
    }
    if gtk::init().is_err() {
        return;
    }
    let entry = gtk::Entry::new();
    let window = gtk::Window::builder().child(&entry).build();
    window.present();
    let text = entry
        .delegate()
        .expect("entry should have an editable delegate")
        .downcast::<gtk::Text>()
        .expect("entry delegate should be GtkText");
    let suffix = &"LICENSE"[1..];
    for seed in ["L", "é", "文"] {
        entry.grab_focus();
        super::focus_filter_entry(&entry, Some(seed));
        assert_eq!(entry.selection_bounds(), None);
        assert_eq!(entry.position(), seed.chars().count() as i32);
        text.emit_by_name::<()>("insert-at-cursor", &[&suffix]);
        assert_eq!(entry.text(), format!("{seed}{suffix}"));
    }
    super::focus_filter_entry(&entry, None);
    assert_eq!(entry.text(), format!("文{suffix}"));
    window.destroy();
}

const SCROLL_PIN_GTK_CHILD: &str = "STRATA_SCROLL_PIN_GTK_CHILD";

const SCROLL_PIN_TEST: &str =
    "ui::browser::collection::tests::waiting_to_scroll_does_not_pin_an_unallocated_view";

fn assert_waiting_to_scroll_does_not_pin_an_unallocated_view() {
    let model = gtk::StringList::new(&["fv\talpha"]);
    let selection = gtk::NoSelection::new(Some(model));
    let list = gtk::ListView::new(Some(selection), Some(gtk::SignalListItemFactory::new()));
    let weak = list.downgrade();
    scroll_collection_when_allocated(list.upcast_ref(), 0);
    drop(list);
    while glib::MainContext::default().iteration(false) {}
    assert!(
        weak.upgrade().is_none(),
        "deferred scroll must not pin the collection view"
    );
}

#[test]
fn waiting_to_scroll_does_not_pin_an_unallocated_view() {
    if std::env::var_os(SCROLL_PIN_GTK_CHILD).is_some() {
        if gtk::init().is_err() {
            return;
        }
        assert_waiting_to_scroll_does_not_pin_an_unallocated_view();
        return;
    }

    let status =
        std::process::Command::new(std::env::current_exe().expect("test executable should exist"))
            .args(["--exact", SCROLL_PIN_TEST])
            .env(SCROLL_PIN_GTK_CHILD, "1")
            .status()
            .expect("isolated GTK scroll pin test should start");
    assert!(status.success(), "isolated GTK scroll pin test failed");
}
