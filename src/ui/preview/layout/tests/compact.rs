// SPDX-License-Identifier: MIT

use super::*;

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
                    for width in [640, 320] {
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
                            assert!(
                                RootExt::focus(&fixture.window).is_some_and(|focus| {
                                    focus.is_ancestor(&fixture.browser.widget())
                                }),
                                "chooser={chooser}, mode={mode:?}: hiding the preview must retain browser input"
                            );
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
                    }
                    fixture.close();
                }
            }
        },
    );
}
