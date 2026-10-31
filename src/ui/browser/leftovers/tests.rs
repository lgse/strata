// SPDX-License-Identifier: MIT

use std::{
    path::Path,
    time::{Duration, Instant},
};

use gtk::glib;

use super::*;
use crate::{
    model::hidden_names::HiddenName,
    services::file_activity::{self, OperationScope},
    ui::{
        browser::{BrowserView, PeekBehavior},
        browser_modes::BrowserMode,
        preferences::PreferenceManager,
    },
};

fn pump_until(condition: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn widgets<T: IsA<gtk::Widget>>(root: &gtk::Widget, found: &mut Vec<T>) {
    if let Some(widget) = root.downcast_ref::<T>() {
        found.push(widget.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        widgets(&widget, found);
    }
}

fn shown_buttons(window: &gtk::Window, class: &str) -> Vec<gtk::Button> {
    let mut found = Vec::new();
    widgets::<gtk::Button>(window.upcast_ref(), &mut found);
    found
        .into_iter()
        .filter(|button| button.has_css_class(class) && button.is_mapped())
        .collect()
}

fn labelled(window: &gtk::Window, label: &str) -> Option<gtk::Button> {
    let mut found = Vec::new();
    widgets::<gtk::Button>(window.upcast_ref(), &mut found);
    found
        .into_iter()
        .find(|button| button.label().as_deref() == Some(label) && button.is_mapped())
}

fn labelled_count(window: &gtk::Window, label: &str) -> usize {
    let mut found = Vec::new();
    widgets::<gtk::Button>(window.upcast_ref(), &mut found);
    found
        .iter()
        .filter(|button| button.label().as_deref() == Some(label) && button.is_mapped())
        .count()
}

fn shows_text(window: &gtk::Window, text: &str) -> bool {
    let mut labels = Vec::new();
    widgets::<gtk::Label>(window.upcast_ref(), &mut labels);
    let mut inscriptions = Vec::new();
    widgets::<gtk::Inscription>(window.upcast_ref(), &mut inscriptions);
    labels
        .iter()
        .any(|label| label.is_mapped() && label.text().contains(text))
        || inscriptions.iter().any(|inscription| {
            inscription.is_mapped() && inscription.text().is_some_and(|shown| shown.contains(text))
        })
}

fn open(view: &BrowserView, folder: &Path) -> gtk::Window {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&view.widget()));
    let window = gtk::Window::builder()
        .child(&overlay)
        .default_width(1000)
        .default_height(650)
        .build();
    window.present();
    view.browser().navigate(Location::local(folder));
    pump_until(
        || {
            view.browser()
                .column_snapshot(0)
                .is_some_and(|snapshot| !snapshot.loading)
        },
        "the folder loads",
    );
    window
}

fn browsing(mode: BrowserMode) -> BrowserView {
    let view = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        PeekBehavior::default(),
    );
    view.set_operation_provider(Rc::new(crate::adapters::LocalOperationProvider));
    view.set_view_mode(mode);
    view
}

fn close(view: &BrowserView, window: &gtk::Window) {
    PreferenceManager::shared().release_bindings_within(&view.widget());
    view.browser().clear_observer();
    window.destroy();
}

#[test]
fn leftover_button_follows_the_column() {
    crate::test_support::gtk_test(
        "ui::browser::leftovers::tests::leftover_button_follows_the_column",
        || {
            for (mode, restored_in) in [
                (BrowserMode::Columns, BrowserMode::Columns),
                (BrowserMode::List, BrowserMode::List),
                (BrowserMode::Icons, BrowserMode::Icons),
                (BrowserMode::List, BrowserMode::Columns),
                (BrowserMode::Icons, BrowserMode::Columns),
            ] {
                let root = tempfile::tempdir().expect("fixture");
                let container = root.path().join(HiddenName::Originals.fresh());
                std::fs::create_dir(&container).expect("originals folder");
                std::fs::write(container.join("song.flac"), b"old").expect("original");
                std::fs::write(root.path().join("notes.txt"), b"notes").expect("file");
                let view = browsing(mode);
                let window = open(&view, root.path());
                pump_until(
                    || shown_buttons(&window, "leftovers-button").len() == 1,
                    "the button",
                );
                view.set_view_mode(restored_in);
                pump_until(
                    || shown_buttons(&window, "leftovers-button").len() == 1,
                    "the button in the view that restores",
                );

                shown_buttons(&window, "leftovers-button")[0].emit_clicked();
                pump_until(
                    || shows_text(&window, "song.flac"),
                    "the review lists the original",
                );
                labelled(&window, &crate::i18n::tr("Restore"))
                    .expect("Restore")
                    .emit_clicked();
                pump_until(
                    || root.path().join("song.flac").exists() && !container.exists(),
                    "the original is back under its name",
                );
                pump_until(
                    || shown_buttons(&window, "leftovers-button").is_empty(),
                    "the button leaves with the leftover",
                );
                if mode != restored_in {
                    view.set_view_mode(mode);
                    pump_until(
                        || shows_text(&window, "notes.txt"),
                        "the first view is back",
                    );
                    assert!(
                        shown_buttons(&window, "leftovers-button").is_empty(),
                        "{mode:?} after {restored_in:?}"
                    );
                }
                assert_eq!(
                    std::fs::read(root.path().join("song.flac")).expect("restored"),
                    b"old",
                    "{mode:?}"
                );
                close(&view, &window);
            }

            let root = tempfile::tempdir().expect("fixture");
            std::fs::create_dir(root.path().join(HiddenName::Originals.fresh()))
                .expect("originals folder");
            let chooser =
                BrowserView::new_chooser(Rc::new(crate::adapters::LocalFileSource), false);
            let window = open(&chooser, root.path());
            assert!(shown_buttons(&window, "leftovers-button").is_empty());
            close(&chooser, &window);
        },
    );
}

#[test]
fn the_review_acts_on_each_leftover_through_the_usual_commands() {
    crate::test_support::gtk_test(
        "ui::browser::leftovers::tests::the_review_acts_on_each_leftover_through_the_usual_commands",
        || {
            for action in ["Delete…", "Show", "Restore", "busy"] {
                let root = tempfile::tempdir().expect("fixture");
                let stage = HiddenName::Copy.fresh().to_string_lossy().into_owned();
                let flat = HiddenName::Replaced.fresh().to_string_lossy().into_owned();
                std::fs::write(root.path().join(&stage), b"partial").expect("stage");
                std::fs::write(root.path().join(&flat), b"original").expect("set aside");
                let view = browsing(BrowserMode::Columns);
                let window = open(&view, root.path());
                pump_until(
                    || shown_buttons(&window, "leftovers-button").len() == 1,
                    "the button",
                );
                shown_buttons(&window, "leftovers-button")[0].emit_clicked();
                pump_until(
                    || labelled(&window, &crate::i18n::tr("Delete…")).is_some(),
                    "the review",
                );
                assert_eq!(
                    labelled_count(&window, &crate::i18n::tr("Delete…")),
                    1,
                    "only the incomplete copy offers Delete…"
                );
                let activity = (action == "busy").then(|| {
                    file_activity::begin(OperationScope {
                        roots: Vec::new(),
                        folders: vec![root.path().to_path_buf()],
                    })
                });
                let label = if action == "busy" { "Restore" } else { action };
                labelled(&window, &crate::i18n::tr(label))
                    .expect(label)
                    .emit_clicked();

                match action {
                    "Delete…" => pump_until(
                        || {
                            labelled(&window, &crate::i18n::tr("Move to Trash")).is_some()
                                || shows_text(
                                    &window,
                                    &crate::i18n::tr("This location doesn't support Trash."),
                                )
                        },
                        "the usual Delete confirmation",
                    ),
                    "Show" => pump_until(
                        || {
                            view.browser()
                                .cursor_entry(0)
                                .is_some_and(|entry| entry.display_name == stage)
                        },
                        "the incomplete copy is shown and selected",
                    ),
                    "Restore" => {
                        let recovered = root.path().join(crate::i18n::tr("Recovered item"));
                        pump_until(
                            || recovered.exists() && !root.path().join(&flat).exists(),
                            "the original under a recovered name",
                        );
                        pump_until(
                            || {
                                view.active_rename_field().is_some_and(|field| {
                                    field.text() == crate::i18n::tr("Recovered item")
                                })
                            },
                            "the name editor",
                        );
                        assert_eq!(std::fs::read(recovered).expect("recovered"), b"original");
                    }
                    _ => {
                        pump_until(
                            || {
                                shows_text(
                                    &window,
                                    &crate::i18n::tr(
                                        "Wait for the running operation to finish, then try again.",
                                    ),
                                )
                            },
                            "the refusal",
                        );
                        assert!(root.path().join(&flat).exists());
                    }
                }
                drop(activity);
                close(&view, &window);
            }
        },
    );
}

#[test]
fn a_folder_holding_originals_gets_a_review_card_once_per_session() {
    crate::test_support::gtk_test(
        "ui::browser::leftovers::tests::a_folder_holding_originals_gets_a_review_card_once_per_session",
        || {
            let root = tempfile::tempdir().expect("fixture");
            let originals = root.path().join("music");
            let elsewhere = root.path().join("documents");
            std::fs::create_dir_all(originals.join(HiddenName::Merge.fresh())).expect("merge");
            std::fs::create_dir(&elsewhere).expect("folder");
            let copies = root.path().join("downloads");
            std::fs::create_dir(&copies).expect("folder");
            std::fs::write(copies.join(HiddenName::Copy.fresh()), b"partial").expect("stage");
            let view = browsing(BrowserMode::Columns);
            let window = open(&view, &copies);
            assert!(shown_buttons(&window, "leftovers-button").len() == 1);
            assert!(labelled(&window, &crate::i18n::tr("Review")).is_none());

            view.browser().navigate(Location::local(&originals));
            pump_until(
                || labelled(&window, &crate::i18n::tr("Review")).is_some(),
                "the card",
            );
            let review = labelled(&window, &crate::i18n::tr("Review")).expect("Review");
            review.grab_focus();
            pump_until(
                || shows_text(&window, &crate::i18n::tr("paused")),
                "the card waits while its Review button has focus",
            );
            review.emit_clicked();
            assert!(labelled(&window, &crate::i18n::tr("Review")).is_none());
            pump_until(
                || shows_text(&window, &crate::i18n::tr("Originals set aside by a merge")),
                "the review",
            );
            labelled(&window, &crate::i18n::tr("Close"))
                .expect("Close")
                .emit_clicked();
            pump_until(
                || labelled(&window, &crate::i18n::tr("Close")).is_none(),
                "the review closes",
            );

            view.browser().navigate(Location::local(&elsewhere));
            view.browser().navigate(Location::local(&originals));
            pump_until(
                || shown_buttons(&window, "leftovers-button").len() == 1,
                "the button",
            );
            let deadline = Instant::now() + Duration::from_millis(200);
            while Instant::now() < deadline {
                glib::MainContext::default().iteration(false);
            }
            assert!(labelled(&window, &crate::i18n::tr("Review")).is_none());
            close(&view, &window);
        },
    );
}
