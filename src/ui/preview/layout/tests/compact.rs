// SPDX-License-Identifier: MIT

use super::*;
use crate::services::{ArchiveFileEntry, archive_preview_tree};

#[test]
fn breadcrumb_navigation_restores_scrolling_after_compact_preview() {
    crate::test_support::gtk_test(
        "ui::preview::layout::tests::compact::breadcrumb_navigation_restores_scrolling_after_compact_preview",
        || {
            let preferences = PreferenceManager::shared();
            preferences.set_browser_mode(BrowserMode::Columns);
            for reduced_motion in [true, false] {
                preferences.set_reduce_motion(reduced_motion);
                let fixture = Fixture::new(false);
                fixture.preview.observe_browser(&fixture.browser.browser());
                fixture.resize(480);
                fixture.enter_descendants(4);
                wait_until(|| fixture.adjustment().value() > 0.0);
                fixture.preview.show(entry("preview.png"), Some(4));
                fixture.settle();
                assert!(fixture.preview.state.sizing.is_suspended());

                let breadcrumbs =
                    find(&fixture.browser.location_widget(), "breadcrumbs").expect("breadcrumbs");
                let mut child = breadcrumbs.first_child();
                let mut ancestor = None;
                while let Some(widget) = child {
                    child = widget.next_sibling();
                    if let Ok(button) = widget.downcast::<gtk::Button>()
                        && button.label().as_deref() == Some("child")
                    {
                        ancestor = Some(button);
                        break;
                    }
                }
                ancestor.expect("ancestor breadcrumb").emit_clicked();
                wait_until(|| !fixture.preview.is_open());
                wait_until(|| {
                    fixture
                        .browser
                        .browser()
                        .column_snapshot(0)
                        .is_some_and(|column| !column.loading)
                });
                assert_eq!(
                    fixture.browser.browser().location_at(0),
                    Some(Location::local(fixture.root.path().join("child")))
                );
                // Breadcrumb navigation starts a new column chain, so its old offset must reset.
                wait_until(|| fixture.adjustment().value() == 0.0);
                fixture.close();
            }
        },
    );
}

#[test]
fn constrained_previews_defer_loading_and_return_focus_without_losing_selection() {
    crate::test_support::gtk_test(
        "ui::preview::layout::tests::compact::constrained_previews_defer_loading_and_return_focus_without_losing_selection",
        || {
            let preferences = PreferenceManager::shared();
            preferences.set_reduce_motion(true);
            for chooser in [false, true] {
                for mode in [BrowserMode::Columns, BrowserMode::Icons, BrowserMode::List] {
                    preferences.set_browser_mode(mode);
                    let fixture = Fixture::new(chooser);
                    std::fs::write(fixture.root.path().join("notes.txt"), "Preview me")
                        .expect("preview file");
                    let browser = fixture.browser.browser();
                    wait_until(|| browser.entry_at(0, 1).is_some());
                    browser.select(0, 1);
                    let selected = browser.focused_entry().expect("selected file");
                    for width in [360, 320] {
                        fixture.resize(width);
                        browser.focus_active();
                        let before = fixture.requests.borrow().len();
                        fixture.preview.toggle(Some(selected.clone()), Some(0));
                        fixture.settle();
                        assert!(fixture.preview.is_enabled());
                        assert!(!fixture.preview.is_open());
                        assert!(fixture.browser.widget().is_mapped());
                        assert_eq!(fixture.requests.borrow().len(), before);

                        fixture.resize(1400);
                        wait_until(|| fixture.requests.borrow().len() == before + 1);
                        assert!(fixture.preview.is_open());
                        let request = fixture
                            .requests
                            .borrow()
                            .last()
                            .expect("resumed request")
                            .clone();
                        fixture.preview.state.handle_event(
                            request.id,
                            PreviewEvent::Ready(Preview {
                                request_id: request.id,
                                entry: request.entry,
                                content_type: "text/plain".into(),
                                content: PreviewContent::Text {
                                    content: "Preview me".into(),
                                    truncated: false,
                                },
                            }),
                        );
                        let content = fixture
                            .preview
                            .state
                            .content
                            .first_child()
                            .expect("loaded content");
                        for focus_preview in [false, true] {
                            if focus_preview {
                                find(&fixture.preview.widget(), "preview-close")
                                    .expect("preview close button")
                                    .grab_focus();
                            } else {
                                browser.focus_active();
                            }
                            fixture.resize(width);
                            wait_until(|| fixture.preview.state.sizing.is_suspended());
                            assert!(!fixture.preview.is_open());
                            assert!(fixture.browser.widget().is_mapped());
                            wait_until(|| {
                                RootExt::focus(&fixture.window).is_some_and(|focus| {
                                    focus == fixture.browser.widget()
                                        || focus.is_ancestor(&fixture.browser.widget())
                                })
                            });
                            fixture.resize(1400);
                            wait_until(|| fixture.preview.is_open());
                            assert_eq!(
                                fixture.preview.state.content.first_child(),
                                Some(content.clone())
                            );
                            assert_eq!(fixture.requests.borrow().len(), before + 1);
                        }

                        fixture.resize(width);
                        wait_until(|| !fixture.preview.is_open());
                        fixture.preview.toggle(Some(selected.clone()), Some(0));
                        fixture.resize(1400);
                        fixture.settle();
                        assert!(!fixture.preview.is_enabled());
                        assert!(!fixture.preview.is_open());
                        assert_eq!(
                            browser
                                .focused_entry()
                                .expect("preserved selection")
                                .location,
                            selected.location
                        );
                        assert!(
                            RootExt::focus(&fixture.window).is_some_and(|focus| {
                                focus == fixture.browser.widget()
                                    || focus.is_ancestor(&fixture.browser.widget())
                                    || fixture.browser.widget().is_ancestor(&focus)
                            }),
                            "chooser={chooser}, mode={mode:?}, width={width}, focus={:?}",
                            RootExt::focus(&fixture.window)
                        );
                    }
                    fixture.close();
                }
            }
        },
    );
}

#[test]
fn suspended_archive_open_focuses_the_tree_when_the_preview_returns() {
    crate::test_support::gtk_test(
        "ui::preview::layout::tests::compact::suspended_archive_open_focuses_the_tree_when_the_preview_returns",
        || {
            let preferences = PreferenceManager::shared();
            preferences.set_browser_mode(BrowserMode::Columns);
            preferences.set_reduce_motion(true);
            let fixture = Fixture::with_files(false, &["archive.zip"]);
            let browser = fixture.browser.browser();
            let position = (0..)
                .map_while(|position| browser.entry_at(0, position).map(|entry| (position, entry)))
                .find(|(_, entry)| entry.display_name == "archive.zip")
                .map(|(position, _)| position)
                .expect("archive position");
            browser.select(0, position);
            browser.focus_active();
            fixture.resize(400);
            fixture.settle();
            let selected = browser.focused_entry().expect("archive entry");
            fixture.preview.toggle(Some(selected.clone()), Some(0));
            fixture.settle();
            assert!(!fixture.preview.is_open());
            assert!(fixture.preview.state.sizing.is_suspended());
            assert!(fixture.requests.borrow().is_empty());
            fixture.resize(1400);
            wait_until(|| fixture.preview.is_open());
            let request = fixture
                .requests
                .borrow()
                .last()
                .expect("resumed archive request")
                .clone();
            fixture.preview.state.handle_event(
                request.id,
                PreviewEvent::Ready(Preview {
                    request_id: request.id,
                    entry: request.entry,
                    content_type: "application/zip".into(),
                    content: PreviewContent::Archive {
                        tree: archive_preview_tree(vec![
                            ArchiveFileEntry {
                                name: "docs/readme.md".to_owned(),
                                directory: false,
                                size: 1,
                            },
                            ArchiveFileEntry {
                                name: "top.txt".to_owned(),
                                directory: false,
                                size: 2,
                            },
                        ]),
                    },
                }),
            );
            fixture.settle();
            let tree =
                find(&fixture.preview.widget(), "preview-archive-list").expect("archive tree");
            let focused = RootExt::focus(&fixture.window).expect("window focus");
            assert!(
                focused == tree || focused.is_ancestor(&tree),
                "archive tree must own keyboard focus, got {focused:?}"
            );
            {
                let archive = fixture.preview.state.archive_browser.borrow();
                let tree_browser = archive.as_ref().expect("archive browser");
                assert_eq!(tree_browser.selected_index(), Some(0));
            }
            assert_eq!(
                browser.focused_entry().expect("listing selection").location,
                selected.location
            );
            fixture.close();
        },
    );
}
