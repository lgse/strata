// SPDX-License-Identifier: MIT

use super::*;

/// An undo or a redo of a move of `b.txt` and `data`, replayed onto a `b.txt` file and a `data`
/// of the other kind than the one moved.
struct KindCase {
    redo: bool,
    moved_folder: bool,
}

const KIND_CASES: [KindCase; 4] = [
    KindCase {
        redo: false,
        moved_folder: false,
    },
    KindCase {
        redo: false,
        moved_folder: true,
    },
    KindCase {
        redo: true,
        moved_folder: false,
    },
    KindCase {
        redo: true,
        moved_folder: true,
    },
];

impl KindCase {
    fn name(&self) -> String {
        let (replay, meeting) = match (self.redo, self.moved_folder) {
            (false, false) => ("undo", "a file meeting a folder"),
            (false, true) => ("undo", "a folder meeting a file"),
            (true, false) => ("redo", "a file meeting a folder"),
            (true, true) => ("redo", "a folder meeting a file"),
        };
        format!("{replay} of {meeting}")
    }
}

/// `data` as a file holding `contents`, or as a folder whose `kept.txt` holds them.
fn make_item(path: &Path, folder: bool, contents: &[u8]) {
    if folder {
        write_files(path, &[("kept.txt", contents)]);
    } else {
        std::fs::write(path, contents).expect("fixture file");
    }
}

fn holds_item(path: &Path, folder: bool, contents: &[u8]) -> bool {
    if folder {
        path.is_dir() && holds(&path.join("kept.txt"), contents)
    } else {
        path.is_file() && holds(path, contents)
    }
}

struct KindReplay {
    _fixture: tempfile::TempDir,
    original: PathBuf,
    current: PathBuf,
    view: crate::ui::browser::BrowserView,
    overlay: gtk::Overlay,
    window: gtk::Window,
}

impl KindReplay {
    /// Moves both items from `original` to `current`, and undoes that move for a redo case.
    fn moved(case: &KindCase) -> Self {
        let fixture = tempfile::tempdir().expect("replay fixture");
        let (original, current) = (
            fixture.path().join("original"),
            fixture.path().join("current"),
        );
        std::fs::create_dir_all(&current).expect("current dir");
        write_files(&original, &[("b.txt", b"moved b")]);
        make_item(&original.join("data"), case.moved_folder, b"moved");
        let (view, overlay, window) = conflict_view();
        let replay = Self {
            _fixture: fixture,
            original,
            current,
            view,
            overlay,
            window,
        };
        replay.view.state.start_transfer(
            Location::local(&replay.current),
            ["b.txt", "data"]
                .map(|name| Location::local(replay.original.join(name)))
                .to_vec(),
            true,
        );
        let browser = replay.view.browser();
        wait_until(
            || browser.pending_undo_move().is_some() && replay.all_in(&replay.current),
            "the move to complete",
        );
        if case.redo {
            replay.replay(false);
            wait_until(
                || browser.pending_redo_move().is_some() && replay.all_in(&replay.original),
                "the undo to move both items back",
            );
        }
        replay
    }

    fn all_in(&self, folder: &Path) -> bool {
        ["b.txt", "data"]
            .iter()
            .all(|name| folder.join(name).exists())
    }

    fn ends(&self, redo: bool) -> (&Path, &Path) {
        if redo {
            (&self.original, &self.current)
        } else {
            (&self.current, &self.original)
        }
    }

    fn replay(&self, redo: bool) {
        let browser = self.view.browser();
        let (generation, records) = if redo {
            browser.pending_redo_move()
        } else {
            browser.pending_undo_move()
        }
        .expect("pending replay");
        assert!(if redo {
            self.view.state.redo_move(generation, records)
        } else {
            self.view.state.undo_move(generation, records)
        });
    }

    fn skip_focused(&self) -> bool {
        button_with_label(&self.overlay, "Skip").is_some_and(|skip| skip.has_focus())
    }
}

#[test]
fn a_replayed_move_never_offers_replace_between_a_file_and_a_folder() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::replay_kinds::a_replayed_move_never_offers_replace_between_a_file_and_a_folder",
        || {
            for case in KIND_CASES {
                let name = case.name();
                let replay = KindReplay::moved(&case);
                let (from, onto) = replay.ends(case.redo);
                std::fs::write(onto.join("b.txt"), b"blocker b").expect("blocking file");
                make_item(&onto.join("data"), !case.moved_folder, b"blocker");
                let overlay = &replay.overlay;

                replay.replay(case.redo);
                assert!(wait_for_modal_layer(overlay), "{name}: no conflict prompt");
                assert!(
                    visible_texts(overlay).iter().any(|text| text == "b.txt"),
                    "{name}: the file meeting a file is asked about first"
                );
                check_button_with_label(overlay, "Apply to All").set_active(true);
                click_button(overlay, "Replace");

                wait_until(
                    || {
                        visible_texts(overlay).iter().any(|text| text == "data")
                            && !has_visible_button(overlay, "Replace")
                    },
                    &format!("{name}: its own prompt, without Replace"),
                );
                assert!(
                    visible_texts(overlay)
                        .iter()
                        .any(|text| text.contains("A file and a folder can’t replace one another")),
                    "{name}: the prompt says why it cannot replace"
                );
                wait_until(
                    || replay.skip_focused(),
                    &format!("{name}: the prompt to start on Skip"),
                );
                click_button(overlay, "Skip");

                wait_until(
                    || holds(&onto.join("b.txt"), b"moved b") && !from.join("b.txt").exists(),
                    &format!("{name}: Apply to All to replace the file that met a file"),
                );
                assert!(
                    holds_item(&onto.join("data"), !case.moved_folder, b"blocker"),
                    "{name}: the item of the other kind stays"
                );
                assert!(
                    holds_item(&from.join("data"), case.moved_folder, b"moved"),
                    "{name}: the skipped item stays where it was"
                );
                replay.window.destroy();
            }
        },
    );
}
