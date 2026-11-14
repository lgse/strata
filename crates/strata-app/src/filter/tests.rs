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
    use strata_core::model::Location;

    use super::super::{DEBOUNCE, delivered, open, queried, session::Batch};
    use crate::{
        host::offscreen::Offscreen,
        test_support::{PATIENCE, fixture, view::hosted_in},
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
                    };
                    assert!(!delivered(app, stale, tx), "the stale batch was shown");
                });

                assert_eq!(host.app().filter.found.len(), 1, "the results shown went");
            },
        );
    }
}
