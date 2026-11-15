// SPDX-License-Identifier: MIT

use super::descent::slash_over;

#[test]
fn a_lone_slash_over_a_query_enters_and_any_other_edit_types() {
    let cases = [
        // Typed after the query, over the whole of it (Ctrl+A), and inside it.
        ("doc", "doc/", Some("doc")),
        ("doc", "/", Some("doc")),
        ("ab", "a/b", Some("ab")),
        ("a/", "a//", Some("a/")),
        // A pasted path holds more than the `/`; a deletion holds no `/`.
        ("doc", "doc/x", None),
        ("doc", "do", None),
        ("doc", "doc", None),
        // No query to enter by.
        ("", "/", None),
        ("  ", "  /", None),
        // Text before the edit that is not ASCII.
        ("été", "été/", Some("été")),
    ];
    for (before, after, entered) in cases {
        assert_eq!(
            slash_over(before, after),
            entered,
            "{before:?} to {after:?}"
        );
    }
}

/// The hosted tests: the part's own bookkeeping, driven inside a window.
mod hosted {
    use rune::{Key, Mods};
    use strata_components::collection::selection::Extend;
    use strata_core::model::Location;

    use super::super::{DEBOUNCE, delivered, open, picked, queried, session::Batch};
    use crate::{
        host::offscreen::Offscreen,
        test_support::{
            PATIENCE, fixture,
            view::{hosted_in, press},
        },
    };

    const MODULE: &str = "filter::tests";

    fn opened(host: &mut Offscreen) -> tempfile::TempDir {
        let folder = fixture();
        host.app().open(Location::local(folder.path()));
        let listed = host.run_until(PATIENCE, |host| host.app().columns.listed(0));
        assert!(listed, "the fixture was not listed");
        folder
    }

    /// A debounce whose wait ended, its search queued, before a later edit: GTK's
    /// `debounce_filter_entry` drops it, so only the text typed now is searched.
    #[test]
    fn a_wait_queued_before_a_later_edit_searches_nothing() {
        hosted_in(
            MODULE,
            "columns",
            "hosted::a_wait_queued_before_a_later_edit_searches_nothing",
            |mut host| {
                let _folder = opened(&mut host);
                host.runtime().update(|app, tx| {
                    assert!(open(app, None, tx));
                    queried(app, "readme".to_owned(), tx);
                });
                // The wait ends and queues its search, and no frame applies it yet.
                std::thread::sleep(DEBOUNCE * 3);
                while glib::MainContext::default().iteration(false) {}

                host.runtime()
                    .update(|app, tx| queried(app, "todo".to_owned(), tx));
                // A frame applies the queue, and GLib's clock, which holds the later wait,
                // does not turn.
                host.runtime().frame(0.0);

                assert!(
                    !host.app().filter.session.is_active(),
                    "the earlier text was searched after the later edit"
                );
            },
        );
    }

    /// A batch for another text than the field's is no answer (GTK's `expect_query`): the
    /// results shown stay.
    #[test]
    fn a_batch_for_another_text_than_the_fields_is_dropped() {
        hosted_in(
            MODULE,
            "columns",
            "hosted::a_batch_for_another_text_than_the_fields_is_dropped",
            |mut host| {
                let _folder = opened(&mut host);
                host.runtime().update(|app, tx| {
                    assert!(open(app, Some("photo"), tx));
                });
                let found = host.run_until(PATIENCE, |host| host.app().filter.found.len() == 1);
                assert!(found, "no result for the query");

                host.runtime().update(|app, tx| {
                    let stale = Batch {
                        generation: app.filter.session.generation(),
                        query: "readme".to_owned(),
                        items: Vec::new(),
                        incomplete: false,
                        has_more: false,
                        candidates: 0,
                        indexing: false,
                        coverage: Default::default(),
                    };
                    assert!(!delivered(app, stale, tx), "the stale batch was shown");
                });

                assert_eq!(host.app().filter.found.len(), 1, "the results shown went");
            },
        );
    }

    /// Over a range of results whose cursor is at the end, Ctrl+F refocuses the open field and
    /// Down lands on the first selected result alone (`filtered-results-group-selection`).
    #[test]
    fn ctrl_f_then_down_over_a_range_selects_only_the_first_result() {
        hosted_in(
            MODULE,
            "icons",
            "hosted::ctrl_f_then_down_over_a_range_selects_only_the_first_result",
            |mut host| {
                let folder = fixture();
                host.app().open(Location::local(folder.path()));
                let listed = host.run_until(PATIENCE, |host| {
                    super::super::Where::Icons.listed(host.app(), 0)
                });
                assert!(listed, "the fixture was not listed");

                let ctrl = Mods {
                    ctrl: true,
                    ..Mods::default()
                };
                press(&mut host, Key::Text("f".to_owned()), ctrl);
                for ch in "txt".chars() {
                    press(&mut host, Key::Text(ch.to_string()), Mods::default());
                }
                let found =
                    host.run_until(PATIENCE, |host| host.app().filter.pane.source().len() == 4);
                assert!(found, "expected four .txt matches");

                host.runtime().update(|app, tx| {
                    let (source, collection) = app.filter.pane.parts_mut();
                    let _ = collection.selection.select_only(source, 0);
                    let _ = collection.selection.extend_to(source, 3, Extend::Replace);
                    collection.note_selection(source);
                    picked(app);
                    tx.focus("icons", true);
                });
                host.runtime().frame(0.0);
                {
                    let pane = &host.app().filter.pane;
                    let sel = &pane.collection().selection;
                    assert_eq!(sel.len(), 4);
                    assert_eq!(sel.cursor_index(pane.source()), Some(3));
                }

                press(&mut host, Key::Text("f".to_owned()), ctrl);
                press(&mut host, Key::Down, Mods::default());

                let pane = &host.app().filter.pane;
                let sel = &pane.collection().selection;
                assert_eq!(sel.len(), 1, "only one result stays selected");
                assert!(
                    sel.is_selected_at(pane.source(), 0),
                    "the first result is selected"
                );
                assert_eq!(sel.cursor_index(pane.source()), Some(0));
            },
        );
    }

    /// In Columns (`src/ui/browser/columns/append/keys.rs`), Down from the refocused filter
    /// field over a range whose cursor is at row 0 lands on the last selected result alone.
    #[test]
    fn ctrl_f_then_down_in_columns_selects_only_the_last_result() {
        hosted_in(
            MODULE,
            "columns",
            "hosted::ctrl_f_then_down_in_columns_selects_only_the_last_result",
            |mut host| {
                let _folder = opened(&mut host);
                let ctrl = Mods {
                    ctrl: true,
                    ..Mods::default()
                };
                let shift = Mods {
                    shift: true,
                    ..Mods::default()
                };
                press(&mut host, Key::Text("f".to_owned()), ctrl);
                for ch in "txt".chars() {
                    press(&mut host, Key::Text(ch.to_string()), Mods::default());
                }
                let found = host.run_until(PATIENCE, |host| {
                    host.app()
                        .columns
                        .shown_with_selection(0)
                        .is_some_and(|(source, _)| source.len() == 4)
                });
                assert!(found, "expected four .txt matches");

                press(&mut host, Key::Down, Mods::default());
                press(&mut host, Key::End, Mods::default());
                press(&mut host, Key::Home, shift);
                {
                    let (source, sel) = host
                        .app()
                        .columns
                        .shown_with_selection(0)
                        .expect("shown results");
                    assert_eq!(sel.len(), 4);
                    assert_eq!(sel.cursor_index(source), Some(0));
                }

                press(&mut host, Key::Text("f".to_owned()), ctrl);
                press(&mut host, Key::Down, Mods::default());

                let (source, sel) = host
                    .app()
                    .columns
                    .shown_with_selection(0)
                    .expect("shown results");
                assert_eq!(sel.len(), 1, "only one result stays selected");
                assert!(sel.is_selected_at(source, 3), "the last result is selected");
                assert_eq!(sel.cursor_index(source), Some(3));
            },
        );
    }
}
