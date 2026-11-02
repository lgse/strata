// SPDX-License-Identifier: MIT

use super::*;

/// The Arabic layout types ظ on the US slash key; it is typed, never a descent.
fn press_arabic_zah(fixture: &KeyboardFixture) -> bool {
    fixture.keys.emit_by_name::<bool>(
        "key-pressed",
        &[&Key::Arabic_zah, &61u32, &ModifierType::empty()],
    )
}

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
                        let sender = crate::ui::search_session::tests::next_results();
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
                        assert!(fixture.press(Key::slash, ModifierType::empty()));
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
                            assert!(fixture.press(Key::slash, ModifierType::empty()));
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

#[test]
fn normal_filter_slash_rejects_files_and_cancels_on_edit_focus_loss_and_escape() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::filter_slash::normal_filter_slash_rejects_files_and_cancels_on_edit_focus_loss_and_escape",
        || {
            let fixture = KeyboardFixture::new();
            let root = fixture._directory.path();
            for folder in ["Documents", "reports"] {
                std::fs::create_dir(root.join(folder)).expect("fixture folder");
            }
            std::fs::write(root.join("report"), b"file").expect("fixture file");
            PreferenceManager::shared().set_filter_include_subfolders(true);
            let browser = fixture.view.browser();
            let none = ModifierType::empty();
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                fixture.view.refresh();
                wait_loaded(&browser, 0);
                focus_files(&fixture);
                assert!(fixture.press(Key::f, ModifierType::CONTROL_MASK));
                let entry = focused_entry(&fixture.window);
                for query in ["", "no-such-match", "report"] {
                    entry.set_text(query);
                    let generation = browser.navigation_generation();
                    assert!(fixture.press(Key::slash, none));
                    pump(150);
                    assert_eq!(
                        browser.navigation_generation(),
                        generation,
                        "{mode:?} {query}"
                    );
                    assert_eq!(entry.text(), query);
                    assert!(fixture.view.filter_has_focus());
                }
                entry.set_text("Documents");
                let generation = browser.navigation_generation();
                assert!(!press_arabic_zah(&fixture), "{mode:?}: ظ reaches the field");
                pump(150);
                assert_eq!(browser.navigation_generation(), generation, "{mode:?}");
                assert!(fixture.view.filter_has_focus());
                entry.set_text("doc");
                assert!(fixture.press(Key::slash, none));
                entry.set_text("report");
                pump(150);
                assert_eq!(browser.active_location(), Some(Location::local(root)));
                entry.set_text("doc");
                assert!(fixture.press(Key::slash, none));
                fixture.sidebar_toggle.grab_focus();
                assert!(fixture.view.show_filter());
                pump(150);
                assert_eq!(
                    browser.active_location(),
                    Some(Location::local(root)),
                    "{mode:?}: returning focus resumed cancelled navigation"
                );
                assert_eq!(entry.text(), "doc");
                entry.set_text("Documents");
                assert!(fixture.press(Key::slash, none));
                assert!(fixture.press(Key::Escape, none));
                pump(150);
                assert_eq!(browser.active_location(), Some(Location::local(root)));
                assert!(!fixture.view.filter_has_focus());
            }
        },
    );
}

#[test]
fn normal_filter_slash_descends_and_keeps_filtering() {
    crate::test_support::gtk_test(
        "ui::window::tests::keyboard_dispatch::footer_prompt::filter_slash::normal_filter_slash_descends_and_keeps_filtering",
        || {
            let fixture = KeyboardFixture::new();
            let root = fixture._directory.path();
            for folder in ["Documents/Projects", "nested/Archive"] {
                std::fs::create_dir_all(root.join(folder)).expect("fixture folder");
            }
            let browser = fixture.view.browser();
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                for recursive in [false, true] {
                    PreferenceManager::shared().set_filter_include_subfolders(recursive);
                    browser.navigate(Location::local(root));
                    fixture.view.refresh();
                    wait_loaded(&browser, 0);
                    focus_files(&fixture);
                    assert!(fixture.press(Key::f, ModifierType::CONTROL_MASK));
                    let entry = focused_entry(&fixture.window);
                    entry.set_text("doc");
                    assert!(fixture.press(Key::slash, ModifierType::empty()));
                    wait_until(|| {
                        browser.active_location() == Some(Location::local(root.join("Documents")))
                            && fixture.view.filter_has_focus()
                    });
                    let entry = focused_entry(&fixture.window);
                    assert_eq!(entry.text(), "");
                    entry.set_text("proj");
                    assert!(fixture.press(Key::slash, ModifierType::empty()));
                    wait_until(|| {
                        browser.active_location()
                            == Some(Location::local(root.join("Documents/Projects")))
                            && fixture.view.filter_has_focus()
                    });
                    assert_eq!(focused_entry(&fixture.window).text(), "");
                    assert!(fixture.press(Key::Escape, ModifierType::empty()));
                    if recursive {
                        browser.navigate(Location::local(root));
                        wait_loaded(&browser, 0);
                        focus_files(&fixture);
                        assert!(fixture.press(Key::f, ModifierType::CONTROL_MASK));
                        focused_entry(&fixture.window).set_text("arc");
                        assert!(fixture.press(Key::slash, ModifierType::empty()));
                        wait_until(|| {
                            browser.active_location()
                                == Some(Location::local(root.join("nested/Archive")))
                                && fixture.view.filter_has_focus()
                        });
                        assert!(fixture.press(Key::Escape, ModifierType::empty()));
                    }
                }
            }
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
            for mode in [BrowserMode::Columns, BrowserMode::List, BrowserMode::Icons] {
                fixture.view.set_view_mode(mode);
                for query in ["Documents-a", "doc", "dcm"] {
                    browser.navigate(Location::local(root));
                    wait_loaded(&browser, 0);
                    focus_files(&fixture);
                    assert!(fixture.press(Key::f, none));
                    fixture.shortcuts.prompt().set_text(query);
                    let first = if query == "Documents-a" {
                        "Documents-a".to_owned()
                    } else {
                        wait_results(&fixture, &["Documents-a", "Documents-b"]);
                        let order = fixture.view.filter_result_names();
                        assert!(fixture.press(Key::Down, none));
                        assert!(fixture.press(Key::Down, none));
                        assert_eq!(selected_result_names(&fixture), [order[1].clone()]);
                        order[0].clone()
                    };
                    let destination = root.join(first);
                    assert!(fixture.press(Key::slash, none), "{mode:?} {query}");
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
                    assert!(fixture.press(Key::slash, none));
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
                    assert!(fixture.press(Key::slash, none), "{mode:?} {query}");
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
                assert!(
                    !press_arabic_zah(&fixture),
                    "{mode:?}: ظ reaches the prompt"
                );
                pump(350);
                assert_eq!(browser.navigation_generation(), generation, "{mode:?}");
                assert!(fixture.shortcuts.prompt_has_focus());
                fixture.shortcuts.prompt().set_text("doc");
                wait_results(&fixture, &["Documents"]);
                fixture.shortcuts.prompt().set_text("arc");
                assert!(fixture.press(Key::slash, none));
                fixture.shortcuts.prompt().set_text("report");
                pump(350);
                assert_eq!(browser.active_location(), Some(Location::local(root)));
                assert_eq!(fixture.shortcuts.prompt_text(), "report");
                fixture.shortcuts.prompt().set_text("arc");
                assert!(fixture.press(Key::slash, none));
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
                fixture.shortcuts.prompt().set_text("doc");
                assert!(fixture.press(Key::slash, none));
                assert!(fixture.press(Key::Escape, none));
                pump(350);
                assert_eq!(fixture.shortcuts.open_prompt_kind(), None);
                assert_eq!(browser.active_location(), Some(Location::local(root)));
            }
        },
    );
}
