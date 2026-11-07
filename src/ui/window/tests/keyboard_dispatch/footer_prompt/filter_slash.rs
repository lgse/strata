// SPDX-License-Identifier: MIT

use super::*;
use crate::ui::window::tests::keyboard_dispatch::filter_focus::{RemoteFolders, SHARE};

#[test]
fn filter_slash_waits_through_empty_indexing_batches() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::filter_slash::filter_slash_waits_through_empty_indexing_batches",
        || {
            use crate::services::{SearchCoverage, SearchEvent, SearchItem};
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                for tenxer in [true, false] {
                    for final_match in [true, false] {
                        let fixture = KeyboardFixture::new();
                        let root = fixture._directory.path();
                        let destination = root.join("Documents");
                        std::fs::create_dir(&destination).expect("fixture folder");
                        if tenxer {
                            enable_tenxer(&fixture);
                        }
                        fixture.view.set_view_mode(mode);
                        focus_files(&fixture);
                        let sender = crate::ui::search_session::next_results_for_test();
                        assert!(fixture.press(
                            Key::f,
                            if tenxer {
                                ModifierType::empty()
                            } else {
                                ModifierType::CONTROL_MASK
                            }
                        ));
                        let entry = focused_entry(&fixture.window);
                        entry.set_text("doc");
                        type_slash(&fixture);
                        let results = |items, indexing| SearchEvent::Results {
                            query: "doc".to_owned(),
                            items,
                            indexing,
                            coverage: SearchCoverage::default(),
                            has_more: false,
                            candidates: 0,
                        };
                        sender.send(results(vec![], true)).expect("partial batch");
                        pump(100);
                        assert_eq!(
                            fixture.view.browser().active_location(),
                            Some(Location::local(root))
                        );
                        assert_eq!(entry.text(), "doc");
                        if !final_match {
                            sender.send(results(vec![], false)).expect("final miss");
                            pump(100);
                            sender
                                .send(results(
                                    vec![SearchItem::for_test(destination.clone(), true)],
                                    false,
                                ))
                                .expect("later match");
                            pump(100);
                            assert_eq!(
                                fixture.view.browser().active_location(),
                                Some(Location::local(root)),
                                "a completed miss must not navigate on later results"
                            );
                            type_slash(&fixture);
                        } else {
                            sender
                                .send(results(
                                    vec![SearchItem::for_test(destination.clone(), true)],
                                    false,
                                ))
                                .expect("final batch");
                        }
                        wait_until_or(
                            || {
                                fixture.view.browser().active_location()
                                    == Some(Location::local(&destination))
                            },
                            || format!("{mode:?} tenxer={tenxer}: pending slash was lost"),
                        );
                        wait_loaded(&fixture.view.browser(), 0);
                        assert_eq!(focused_entry(&fixture.window).text(), "");
                    }
                }
            }
        },
    );
}

/// A folder without a native path filters its listing in place, with no result rows,
/// so the highlighted match is the listing's cursor.
fn tenxer_slash_enters_the_highlighted_remote_folder(mode: BrowserMode) {
    let none = ModifierType::empty();
    let fixture = KeyboardFixture::with_parts(Rc::new(TextPreview), || {
        BrowserView::new(
            Rc::new(RemoteFolders),
            crate::ui::browser::PeekBehavior::default(),
        )
    });
    enable_tenxer(&fixture);
    fixture.view.set_view_mode(mode);
    let browser = fixture.view.browser();
    browser.navigate(Location::uri(SHARE));
    wait_loaded(&browser, 0);
    focus_files(&fixture);
    // Works around a focus bug: after a load, the first Down from the prompt
    // moves focus into the listing (ModeViews::update_selection).
    assert!(fixture.press(Key::End, none));
    assert!(fixture.press(Key::Home, none));
    assert!(fixture.press(Key::f, none));
    fixture.shortcuts.prompt().set_text("txt");
    wait_until(|| fixture.view.listing_filter().as_deref() == Some("txt"));
    assert!(fixture.press(Key::Down, none));
    assert!(fixture.press(Key::Down, none));
    let (depth, position, _) = browser.focused_item().expect("a listing cursor");
    let highlighted = browser
        .entry_at(depth, position)
        .expect("the highlighted folder");
    assert_ne!(
        highlighted.display_name, "alpha-report.txt",
        "{mode:?}: Down moves past the first match"
    );
    type_slash(&fixture);
    wait_until_or(
        || browser.active_location() == Some(highlighted.location.clone()),
        || {
            format!(
                "{mode:?} remote: location {:?}, expected {:?}",
                browser.active_location(),
                highlighted.location
            )
        },
    );
}

#[test]
fn tenxer_filter_slash_descends_and_keeps_filtering() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::filter_slash::tenxer_filter_slash_descends_and_keeps_filtering",
        || {
            let fixture = KeyboardFixture::new();
            let root = fixture._directory.path();
            for folder in ["Documents-a/Projects", "Documents-b/Projects", "Archive"] {
                std::fs::create_dir_all(root.join(folder)).expect("fixture folder");
            }
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            let none = ModifierType::empty();
            #[derive(Debug, PartialEq)]
            enum Cursor {
                Untouched,
                Stepped,
                SteppedThenEdited,
            }
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                for (query, cursor) in [
                    ("Documents-a", Cursor::Untouched),
                    ("doc", Cursor::Untouched),
                    ("dcm", Cursor::Stepped),
                    ("doc", Cursor::SteppedThenEdited),
                ] {
                    browser.navigate(Location::local(root));
                    wait_loaded(&browser, 0);
                    focus_files(&fixture);
                    assert!(fixture.press(Key::f, none));
                    fixture.shortcuts.prompt().set_text(query);
                    let entered = if query == "Documents-a" {
                        "Documents-a".to_owned()
                    } else {
                        wait_results(&fixture, &["Documents-a", "Documents-b"]);
                        let order = fixture.view.filter_result_names();
                        if cursor != Cursor::Untouched {
                            assert!(fixture.press(Key::Down, none));
                            assert!(fixture.press(Key::Down, none));
                            assert_eq!(selected_result_names(&fixture), [order[1].clone()]);
                        }
                        if cursor == Cursor::SteppedThenEdited {
                            fixture.shortcuts.prompt().set_text("do");
                            wait_results(&fixture, &["Documents-a", "Documents-b"]);
                        }
                        let order = fixture.view.filter_result_names();
                        if cursor == Cursor::Stepped {
                            order[1].clone()
                        } else {
                            order[0].clone()
                        }
                    };
                    let query = format!("{query} {cursor:?}");
                    let destination = root.join(entered);
                    type_slash(&fixture);
                    wait_until_or(
                        || browser.active_location() == Some(Location::local(&destination)),
                        || {
                            format!(
                                "{mode:?} {query}: location {:?}, prompt {:?}, results {:?}",
                                browser.active_location(),
                                fixture.shortcuts.prompt_text(),
                                fixture.view.filter_result_names()
                            )
                        },
                    );
                    wait_loaded(&browser, 0);
                    assert_eq!(fixture.shortcuts.open_prompt_kind(), Some(Prompt::Filter));
                    assert!(fixture.shortcuts.prompt_has_focus(), "{mode:?} {query}");
                    assert_eq!(fixture.shortcuts.prompt_text(), "");
                    assert_eq!(fixture.view.listing_filter(), None);
                    fixture.shortcuts.prompt().set_text("proj");
                    type_slash(&fixture);
                    wait_until_or(
                        || {
                            browser.active_location()
                                == Some(Location::local(destination.join("Projects")))
                        },
                        || {
                            format!(
                                "{mode:?} {query}, child: location {:?}, prompt {:?}, results {:?}",
                                browser.active_location(),
                                fixture.shortcuts.prompt_text(),
                                fixture.view.filter_result_names()
                            )
                        },
                    );
                    wait_loaded(&browser, 0);
                    assert!(fixture.shortcuts.prompt_has_focus(), "{mode:?}");
                    assert_eq!(fixture.shortcuts.prompt_text(), "");
                    assert!(fixture.press(Key::Escape, none));
                }
                tenxer_slash_enters_the_highlighted_remote_folder(mode);
            }
        },
    );
}

#[test]
fn tenxer_filter_slash_uses_the_latest_query_and_rejects_files_and_misses() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::filter_slash::tenxer_filter_slash_uses_the_latest_query_and_rejects_files_and_misses",
        || {
            let fixture = KeyboardFixture::new();
            let root = fixture._directory.path();
            for folder in ["Documents", "Archive", "reports"] {
                std::fs::create_dir(root.join(folder)).expect("fixture folder");
            }
            std::fs::write(root.join("report"), b"file").expect("fixture file");
            enable_tenxer(&fixture);
            let browser = fixture.view.browser();
            let none = ModifierType::empty();
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                browser.navigate(Location::local(root));
                fixture.view.refresh();
                wait_loaded(&browser, 0);
                focus_files(&fixture);
                assert!(fixture.press(Key::f, none));
                for query in ["", "no-such-match", "report"] {
                    fixture.shortcuts.prompt().set_text(query);
                    let generation = browser.navigation_generation();
                    type_slash(&fixture);
                    pump(350);
                    assert_eq!(
                        browser.navigation_generation(),
                        generation,
                        "{mode:?} {query}"
                    );
                    assert!(fixture.shortcuts.prompt_has_focus());
                    assert_eq!(fixture.shortcuts.prompt_text(), query);
                }
                fixture.shortcuts.prompt().set_text("Documents");
                let generation = browser.navigation_generation();
                crate::test_support::commit_text(fixture.shortcuts.prompt(), "/Projects");
                pump(350);
                assert_eq!(browser.navigation_generation(), generation, "{mode:?}");
                assert!(
                    fixture.shortcuts.prompt_text().contains('/'),
                    "{mode:?}: a pasted path is typed"
                );
                assert!(fixture.shortcuts.prompt_has_focus());
                fixture.shortcuts.prompt().set_text("doc");
                wait_results(&fixture, &["Documents"]);
                fixture.shortcuts.prompt().set_text("arc");
                type_slash(&fixture);
                fixture.shortcuts.prompt().set_text("report");
                pump(350);
                assert_eq!(browser.active_location(), Some(Location::local(root)));
                assert_eq!(fixture.shortcuts.prompt_text(), "report");
                fixture.shortcuts.prompt().set_text("arc");
                type_slash(&fixture);
                wait_until(|| {
                    browser.active_location() == Some(Location::local(root.join("Archive")))
                });
                wait_loaded(&browser, 0);
                assert!(fixture.shortcuts.prompt_has_focus());
                assert_eq!(fixture.shortcuts.prompt_text(), "");
                assert!(fixture.press(Key::Escape, none));
                browser.navigate(Location::local(root));
                wait_loaded(&browser, 0);
                focus_files(&fixture);
                assert!(fixture.press(Key::f, none));
                // Focus returns before the prompt's idle close; the blur alone cancels.
                fixture.shortcuts.prompt().set_text("arc");
                type_slash(&fixture);
                fixture.sidebar_toggle.grab_focus();
                fixture.shortcuts.prompt().grab_focus();
                pump(350);
                assert_eq!(fixture.shortcuts.prompt_text(), "arc", "{mode:?}");
                assert_eq!(browser.active_location(), Some(Location::local(root)));
                fixture.shortcuts.prompt().set_text("doc");
                type_slash(&fixture);
                assert!(fixture.press(Key::Escape, none));
                pump(350);
                assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
                assert_eq!(browser.active_location(), Some(Location::local(root)));
            }
        },
    );
}
