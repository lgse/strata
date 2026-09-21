// SPDX-License-Identifier: MIT

use super::*;

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
                assert!(fixture.preview.state.sizing.is_compact());

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

fn assert_dismissal_focus_stays_put(fixture: &Fixture) {
    let close = fixture.preview.state.close_button.clone();
    wait_until(|| close.has_focus());
    let frames = Rc::new(Cell::new(0));
    let stable = Rc::new(Cell::new(true));
    let observed_frames = frames.clone();
    let observed_focus = stable.clone();
    fixture.split.add_tick_callback(move |_, _| {
        observed_focus.set(observed_focus.get() && close.has_focus());
        observed_frames.set(observed_frames.get() + 1);
        if observed_frames.get() >= 12 {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    wait_until(|| frames.get() >= 12);
    assert!(
        stable.get(),
        "idle preview must retain its keyboard dismissal target"
    );
}

#[test]
fn compact_preview_loads_and_returns_keyboard_focus_without_losing_selection() {
    crate::test_support::gtk_test(
        "ui::preview::layout::tests::compact::compact_preview_loads_and_returns_keyboard_focus_without_losing_selection",
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
                        assert!(fixture.preview.is_open());
                        assert_eq!(fixture.requests.borrow().len(), before + 1);
                        let request = fixture
                            .requests
                            .borrow()
                            .last()
                            .expect("compact preview request")
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
                            .expect("loaded preview content");
                        assert_dismissal_focus_stays_put(&fixture);
                        fixture.resize(1400);
                        fixture.settle();
                        assert_eq!(fixture.preview.state.content.first_child(), Some(content));
                        assert_eq!(fixture.requests.borrow().len(), before + 1);
                        fixture.resize(width);
                        fixture.settle();
                        assert_dismissal_focus_stays_put(&fixture);
                        fixture.preview.state.close_button.emit_clicked();
                        fixture.settle();
                        assert!(!fixture.preview.is_enabled());
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
