// SPDX-License-Identifier: MIT

//! The key sequences of gate G2's Columns runs, against the adapter. Each step names the
//! scenario file it comes from under `tests/qa/scenarios/`, and expects what that file
//! waits for. After every step the browser must hold what the view shows.

use super::{Seen, assert_browser_agrees, fixture, folder_name, hosted, open, press, wait};
use crate::host::offscreen::Offscreen;

/// What a scenario waits for after a key. `None` is not checked.
#[derive(Clone, Copy, Default)]
struct Expect {
    focused: Option<&'static str>,
    /// The names selected in the first pane.
    selected: Option<&'static [&'static str]>,
    /// Another pane, and the names selected in it.
    within: Option<(&'static str, &'static [&'static str])>,
    panes: Option<&'static [&'static str]>,
    /// The folder the browser is working in: a scenario's `directory`.
    directory: Option<&'static str>,
}

impl Expect {
    fn met(&self, seen: &Seen, root: &str) -> bool {
        let named = |pane: &'static str| if pane == "$root" { root } else { pane };
        let focused = self
            .focused
            .is_none_or(|name| seen.focused.as_deref() == Some(name));
        let holds = |pane: &str, names: &[&str]| {
            seen.pane(pane).is_some_and(|pane| pane.selected() == names)
        };
        let selected = self.selected.is_none_or(|names| holds(root, names))
            && self.within.is_none_or(|(pane, names)| holds(pane, names));
        let panes = self.panes.is_none_or(|names| {
            let names: Vec<&str> = names.iter().copied().map(named).collect();
            seen.pane_names() == names
        });
        let directory = self
            .directory
            .is_none_or(|name| seen.directory.as_deref() == Some(named(name)));
        focused && selected && panes && directory
    }
}

const NOTHING: Expect = Expect {
    focused: None,
    selected: None,
    within: None,
    panes: None,
    directory: None,
};

const fn focused(name: &'static str) -> Expect {
    NOTHING.on(name)
}

const fn selected(names: &'static [&'static str]) -> Expect {
    NOTHING.with(names)
}

impl Expect {
    const fn on(mut self, name: &'static str) -> Self {
        self.focused = Some(name);
        self
    }

    const fn with(mut self, names: &'static [&'static str]) -> Self {
        self.selected = Some(names);
        self
    }

    const fn within(mut self, pane: &'static str, names: &'static [&'static str]) -> Self {
        self.within = Some((pane, names));
        self
    }

    const fn panes(mut self, names: &'static [&'static str]) -> Self {
        self.panes = Some(names);
        self
    }

    const fn working_in(mut self, pane: &'static str) -> Self {
        self.directory = Some(pane);
        self
    }
}

type Step = (&'static str, Expect);

const ALL: &[&str] = &["archive", "documents", "pictures", "readme.md", "todo.txt"];

/// Every sequence starts from a fresh launch: `archive` focused and selected.
const SEQUENCES: &[(&str, &[Step])] = &[
    (
        "keyboard_navigation/arrow-keys-move-focus (columns-arrows)",
        &[
            ("Down", focused("documents")),
            ("Down", focused("pictures")),
            ("Down", focused("readme.md")),
            ("Down", focused("todo.txt").with(&["todo.txt"])),
            ("Up", focused("readme.md")),
        ],
    ),
    (
        "keyboard_navigation/shift-arrow-extends-selection",
        &[
            ("End", focused("todo.txt")),
            ("Up", focused("readme.md")),
            ("shift+Down", selected(&["readme.md", "todo.txt"])),
            ("shift+Up", selected(&["readme.md"])),
        ],
    ),
    (
        "keyboard_navigation/columns-home-end-mirror",
        &[
            ("Down", focused("documents").panes(&["$root", "documents"])),
            ("End", focused("todo.txt").panes(&["$root"])),
            ("Up", focused("readme.md")),
            ("Up", focused("pictures").panes(&["$root", "pictures"])),
            ("Home", focused("archive").panes(&["$root", "archive"])),
        ],
    ),
    (
        "keyboard_navigation/select-all-selects-every-entry",
        &[("End", focused("todo.txt")), ("ctrl+a", selected(ALL))],
    ),
    (
        "keyboard_navigation/back-then-file-keys-target-focused-folder (Left, and Right back)",
        &[
            ("Down", focused("documents").panes(&["$root", "documents"])),
            (
                "Right",
                focused("notes.txt").within("documents", &["notes.txt"]),
            ),
            (
                "Left",
                focused("documents")
                    .with(&["documents"])
                    .panes(&["$root", "documents"]),
            ),
        ],
    ),
    (
        "escape_selection/escape-only-clears-active-column",
        &[
            // The scenario clicks its way to `notes.txt`; the keys reach the same state.
            ("Down", focused("documents").panes(&["$root", "documents"])),
            (
                "Right",
                focused("notes.txt").within("documents", &["notes.txt"]),
            ),
            (
                "Escape",
                focused("notes.txt")
                    .within("documents", &[])
                    .with(&["documents"])
                    .panes(&["$root", "documents"]),
            ),
        ],
    ),
    (
        "escape_selection/shift-after-escape-starts-on-focused (no-range)",
        &[
            ("Escape", selected(&[]).on("archive")),
            ("shift+Down", selected(&["archive"]).on("archive")),
            (
                "shift+Down",
                selected(&["archive", "documents"]).on("documents"),
            ),
        ],
    ),
    (
        "escape_selection/shift-after-escape-starts-on-focused (had-range)",
        &[
            (
                "shift+Down",
                selected(&["archive", "documents"]).on("documents"),
            ),
            ("Escape", selected(&[]).on("documents")),
            ("shift+Down", selected(&["documents"]).on("documents")),
            (
                "shift+Down",
                selected(&["documents", "pictures"]).on("pictures"),
            ),
        ],
    ),
    (
        "escape_selection/enter-after-escape-opens-folder",
        &[
            ("Down", focused("documents")),
            ("Escape", selected(&[]).on("documents")),
            (
                "Return",
                NOTHING
                    .working_in("documents")
                    .panes(&["$root", "documents"]),
            ),
        ],
    ),
    (
        "selection/home-end-page-move-shift-anchor",
        &[
            ("Down", focused("documents")),
            ("Down", selected(&["pictures"])),
            ("End", selected(&["todo.txt"]).on("todo.txt")),
            ("shift+Home", selected(ALL)),
            ("Home", selected(&["archive"]).on("archive")),
            ("shift+Down", selected(&["archive", "documents"])),
            ("Page_Down", selected(&["todo.txt"]).on("todo.txt")),
            ("shift+Up", selected(&["readme.md", "todo.txt"])),
            ("Home", selected(&["archive"])),
            ("Down", selected(&["documents"])),
            (
                "shift+Page_Down",
                selected(&["documents", "pictures", "readme.md", "todo.txt"]),
            ),
        ],
    ),
];

/// The five of slice decision 17's six behaviours the fixture can show: what the GTK view
/// does, which the Columns component now does too, with the browser holding the result.
/// The sixth, a key that only changes the active column, is
/// `a_key_between_open_columns_mirrors_though_the_pointer_moved_last`.
const AS_THE_GTK_VIEW: &[(&str, &[Step])] = &[
    (
        "Return on a folder whose column mirroring opened lands on its first entry",
        &[
            ("Down", focused("documents").panes(&["$root", "documents"])),
            (
                "Return",
                focused("notes.txt")
                    .within("documents", &["notes.txt"])
                    .working_in("documents"),
            ),
        ],
    ),
    (
        "an arrow at the list's end selects the cursor's entry alone",
        &[
            ("shift+End", selected(ALL).on("todo.txt")),
            ("Down", selected(&["todo.txt"]).on("todo.txt")),
            ("shift+Home", selected(ALL).on("archive")),
            ("Up", selected(&["archive"]).on("archive")),
        ],
    ),
    (
        "a key that leaves the cursor where it is mirrors its folder",
        &[("Home", focused("archive").panes(&["$root", "archive"]))],
    ),
    (
        "Ctrl+A takes the cursor to the last entry",
        &[
            ("Down", focused("documents")),
            ("ctrl+a", selected(ALL).on("todo.txt")),
            ("Up", selected(&["readme.md"]).on("readme.md")),
        ],
    ),
    (
        "Right opens the cursor's folder in place of another folder's column",
        &[
            ("Down", focused("documents").panes(&["$root", "documents"])),
            (
                "shift+Down",
                selected(&["documents", "pictures"])
                    .on("pictures")
                    .panes(&["$root", "documents"]),
            ),
            (
                "Right",
                focused("diagram.txt")
                    .panes(&["$root", "pictures"])
                    .working_in("pictures"),
            ),
        ],
    ),
];

/// Runs each sequence from a fresh launch of the fixture, `archive` focused and selected.
fn run(host: &mut Offscreen, sequences: &[(&str, &[Step])]) {
    for (scenario, steps) in sequences {
        let folder = fixture();
        let root = folder_name(folder.path());
        let launched = open(host, folder.path());
        assert_eq!(launched.focused.as_deref(), Some("archive"), "{scenario}");
        for (step, (key, expect)) in steps.iter().enumerate() {
            let after = format!("{scenario}, step {} ({key})", step + 1);

            press(host, key);

            wait(host, &after, |seen| expect.met(seen, &root));
            assert_browser_agrees(host, &after);
        }
    }
}

#[test]
fn keys_move_as_the_g2_scenarios_expect_and_the_browser_holds_the_result() {
    hosted(
        "keys::keys_move_as_the_g2_scenarios_expect_and_the_browser_holds_the_result",
        |mut host| run(&mut host, SEQUENCES),
    );
}

#[test]
fn keys_do_what_the_gtk_view_does_where_the_component_differed() {
    hosted(
        "keys::keys_do_what_the_gtk_view_does_where_the_component_differed",
        |mut host| run(&mut host, AS_THE_GTK_VIEW),
    );
}

/// `Browser::focus_child` raises `FocusChanged`, so the GTK views mirror the entry the
/// keyboard lands on whatever moved last. The view now reports the key, so the adapter knows
/// the keyboard is navigating again.
#[test]
fn a_key_between_open_columns_mirrors_though_the_pointer_moved_last() {
    hosted(
        "keys::a_key_between_open_columns_mirrors_though_the_pointer_moved_last",
        |mut host| {
            let folder = tempfile::tempdir().expect("folder");
            std::fs::create_dir_all(folder.path().join("outer/inner")).expect("folders");
            std::fs::write(folder.path().join("outer/inner/file.txt"), "file").expect("file");
            let root = folder_name(folder.path());
            open(&mut host, folder.path());
            press(&mut host, "Home");
            wait(&mut host, "outer's column", |seen| {
                seen.pane_names() == [root.as_str(), "outer"]
            });
            // The pointer moves over the view, which ends keyboard navigation.
            for at in [(700.0, 500.0), (720.0, 520.0)] {
                host.runtime().pointer_move(at.0, at.1);
                super::idle(&mut host);
            }

            press(&mut host, "Right");

            let entered = wait(&mut host, "inner's column", |seen| {
                seen.pane_names() == [root.as_str(), "outer", "inner"]
            });
            assert_eq!(entered.focused.as_deref(), Some("inner"));
            assert_browser_agrees(&host, "Right into the open column");
        },
    );
}
