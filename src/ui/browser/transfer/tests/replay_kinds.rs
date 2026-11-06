// SPDX-License-Identifier: MIT

use super::*;

/// One moved item: its name, whether it was moved as a folder, and whether the item a replay
/// then meets under its name is a folder.
type KindItem = (&'static str, bool, bool);

fn file_on_file(name: &'static str) -> KindItem {
    (name, false, false)
}

fn file_on_folder(name: &'static str) -> KindItem {
    (name, false, true)
}

fn folder_on_file(name: &'static str) -> KindItem {
    (name, true, false)
}

#[derive(Clone, Copy)]
enum Answers {
    /// Replace with Apply to All on the first item, which also replaces the last, then Skip the
    /// item of the other kind between them, which keeps its own prompt.
    ReplaceAllThenSkip,
    /// Nothing can be replaced, so the prompt offers only Cancel.
    CancelOnly,
    /// Skip with Apply to All on the first prompt, which moves nothing.
    SkipAll,
}

struct KindCase {
    name: &'static str,
    redo: bool,
    items: Vec<KindItem>,
    answers: Answers,
}

fn kind_cases() -> [KindCase; 7] {
    let replace_then_skip = |name, redo, other: KindItem| KindCase {
        name,
        redo,
        items: vec![file_on_file("b.txt"), other, file_on_file("c.txt")],
        answers: Answers::ReplaceAllThenSkip,
    };
    [
        replace_then_skip(
            "undo of a file meeting a folder",
            false,
            file_on_folder("data"),
        ),
        replace_then_skip(
            "undo of a folder meeting a file",
            false,
            folder_on_file("data"),
        ),
        replace_then_skip(
            "redo of a file meeting a folder",
            true,
            file_on_folder("data"),
        ),
        replace_then_skip(
            "redo of a folder meeting a file",
            true,
            folder_on_file("data"),
        ),
        KindCase {
            name: "undo of only items meeting the other kind",
            redo: false,
            items: vec![file_on_folder("data"), folder_on_file("more")],
            answers: Answers::CancelOnly,
        },
        KindCase {
            name: "redo of only items meeting the other kind",
            redo: true,
            items: vec![folder_on_file("data"), file_on_folder("more")],
            answers: Answers::CancelOnly,
        },
        KindCase {
            name: "undo with every same-kind conflict skipped",
            redo: false,
            items: vec![file_on_file("b.txt"), file_on_file("c.txt")],
            answers: Answers::SkipAll,
        },
    ]
}

/// A file holding `contents`, or a folder whose `kept.txt` holds them.
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

fn focused_button(overlay: &gtk::Overlay) -> Option<String> {
    visible_widget(overlay, |button: &gtk::Button| button.has_focus())
        .and_then(|button| button.label())
        .map(|label| label.to_string())
}

struct KindReplay {
    _fixture: tempfile::TempDir,
    original: PathBuf,
    current: PathBuf,
    names: Vec<&'static str>,
    view: crate::ui::browser::BrowserView,
    overlay: gtk::Overlay,
    window: gtk::Window,
}

impl KindReplay {
    /// Moves the case's items from `original` to `current`, and undoes that move for a redo.
    fn moved(case: &KindCase) -> Self {
        let fixture = tempfile::tempdir().expect("replay fixture");
        let (original, current) = (
            fixture.path().join("original"),
            fixture.path().join("current"),
        );
        std::fs::create_dir_all(&original).expect("original dir");
        std::fs::create_dir_all(&current).expect("current dir");
        for &(name, moved_folder, _) in &case.items {
            make_item(&original.join(name), moved_folder, b"moved");
        }
        let (view, overlay, window) = conflict_view();
        let replay = Self {
            _fixture: fixture,
            original,
            current,
            names: case.items.iter().map(|(name, ..)| *name).collect(),
            view,
            overlay,
            window,
        };
        replay.view.state.start_transfer(
            Location::local(&replay.current),
            replay
                .names
                .iter()
                .map(|name| Location::local(replay.original.join(name)))
                .collect(),
            true,
        );
        wait_until(
            || replay.pending(false) && replay.all_in(&replay.current),
            "the move to complete",
        );
        if case.redo {
            replay.replay(false);
            wait_until(
                || replay.pending(true) && replay.all_in(&replay.original),
                "the undo to move every item back",
            );
        }
        replay
    }

    fn all_in(&self, folder: &Path) -> bool {
        self.names.iter().all(|name| folder.join(name).exists())
    }

    /// Where the replay moves the items from and onto.
    fn ends(&self, redo: bool) -> (&Path, &Path) {
        if redo {
            (&self.original, &self.current)
        } else {
            (&self.current, &self.original)
        }
    }

    /// The pending move of this case's items. The history is shared by every case on this
    /// thread, so an earlier case's entry does not count.
    fn pending_move(&self, redo: bool) -> Option<(u64, Vec<MoveRecord>)> {
        let browser = self.view.browser();
        let pending = if redo {
            browser.pending_redo_move()
        } else {
            browser.pending_undo_move()
        };
        let current = Location::local(&self.current);
        pending.filter(|(_, records)| {
            records
                .iter()
                .all(|record| record.current.parent().as_ref() == Some(&current))
        })
    }

    fn pending(&self, redo: bool) -> bool {
        self.pending_move(redo).is_some()
    }

    fn replay(&self, redo: bool) -> u64 {
        let (generation, records) = self.pending_move(redo).expect("pending replay");
        assert!(if redo {
            self.view.state.redo_move(generation, records)
        } else {
            self.view.state.undo_move(generation, records)
        });
        generation
    }

    fn wait_for_focus(&self, label: &str, what: &str) {
        wait_until(
            || focused_button(&self.overlay).as_deref() == Some(label),
            what,
        );
    }

    fn shows(&self, text: &str) -> bool {
        visible_texts(&self.overlay)
            .iter()
            .any(|shown| shown.contains(text))
    }
}

fn replace_all_then_skip(replay: &KindReplay, case: &KindCase) {
    let (name, overlay) = (case.name, &replay.overlay);
    let (from, onto) = replay.ends(case.redo);
    let [
        (replaced, ..),
        (other, moved_folder, blocked_folder),
        (also_replaced, ..),
    ] = case.items[..]
    else {
        panic!("{name}: replace-all cases move three items");
    };
    assert!(
        visible_texts(overlay).iter().any(|text| text == replaced),
        "{name}: the item meeting its own kind is asked about first"
    );
    check_button_with_label(overlay, "Apply to All").set_active(true);
    click_button(overlay, "Replace");

    wait_until(
        || {
            visible_texts(overlay).iter().any(|text| text == other)
                && !has_visible_button(overlay, "Replace")
        },
        &format!("{name}: its own prompt, without Replace"),
    );
    assert!(
        replay.shows("A file and a folder can’t replace one another"),
        "{name}: the prompt says why it cannot replace"
    );
    replay.wait_for_focus("Cancel", &format!("{name}: the prompt to start on Cancel"));
    click_button(overlay, "Skip");

    for replaced in [replaced, also_replaced] {
        wait_until(
            || holds(&onto.join(replaced), b"moved") && !from.join(replaced).exists(),
            &format!("{name}: Apply to All to replace {replaced}, which met its own kind"),
        );
    }
    assert!(
        holds_item(&onto.join(other), blocked_folder, b"blocker"),
        "{name}: the item of the other kind stays"
    );
    assert!(
        holds_item(&from.join(other), moved_folder, b"moved"),
        "{name}: the skipped item stays where it was"
    );
}

fn assert_nothing_replayed(replay: &KindReplay, case: &KindCase, before: Before) {
    let name = case.name;
    let (from, onto) = replay.ends(case.redo);
    iterate_context(20);
    assert_eq!(
        replay.view.browser().last_started_operation(),
        before.started,
        "{name}: nothing starts"
    );
    assert_eq!(
        replay
            .pending_move(case.redo)
            .map(|(generation, _)| generation),
        Some(before.generation),
        "{name}: a replay that moved nothing keeps its entry"
    );
    for &(item, moved_folder, blocked_folder) in &case.items {
        assert!(
            holds_item(&from.join(item), moved_folder, b"moved")
                && holds_item(&onto.join(item), blocked_folder, b"blocker"),
            "{name}: {item} stays where it was and so does the item it met"
        );
    }
}

/// What a replay looked like before its prompt: the last operation started and its entry.
#[derive(Clone, Copy)]
struct Before {
    started: Option<crate::services::OperationRequestId>,
    generation: u64,
}

fn cancel_only(replay: &KindReplay, case: &KindCase, before: Before) {
    let (name, overlay) = (case.name, &replay.overlay);
    replay.wait_for_focus("Cancel", &format!("{name}: the prompt to start on Cancel"));
    for hidden in ["Replace", "Skip", "Apply to All"] {
        assert!(
            !has_visible_button(overlay, hidden),
            "{name}: {hidden} cannot move anything, so it is not offered"
        );
    }
    click_button(overlay, "Cancel");
    assert_nothing_replayed(replay, case, before);
}

fn skip_all(replay: &KindReplay, case: &KindCase, before: Before) {
    let overlay = &replay.overlay;
    check_button_with_label(overlay, "Apply to All").set_active(true);
    click_button(overlay, "Skip");
    assert_nothing_replayed(replay, case, before);
}

#[test]
fn a_replay_offers_no_replace_between_kinds_and_keeps_an_entry_that_moved_nothing() {
    crate::test_support::gtk_test(
        "ui::browser::transfer::tests::replay_kinds::a_replay_offers_no_replace_between_kinds_and_keeps_an_entry_that_moved_nothing",
        || {
            for case in kind_cases() {
                let replay = KindReplay::moved(&case);
                let (_, onto) = replay.ends(case.redo);
                for &(item, _, blocked_folder) in &case.items {
                    make_item(&onto.join(item), blocked_folder, b"blocker");
                }
                let started = replay.view.browser().last_started_operation();

                let generation = replay.replay(case.redo);
                let before = Before {
                    started,
                    generation,
                };
                assert!(
                    wait_for_modal_layer(&replay.overlay),
                    "{}: no conflict prompt",
                    case.name
                );
                match case.answers {
                    Answers::ReplaceAllThenSkip => replace_all_then_skip(&replay, &case),
                    Answers::CancelOnly => cancel_only(&replay, &case, before),
                    Answers::SkipAll => skip_all(&replay, &case, before),
                }
                replay.window.destroy();
            }
        },
    );
}
