//! The page's fake data: a copy of part of Strata's shortcut catalog (the real one is Strata's,
//! built from its key map per view, map and chooser request), and a small listing to count.

use strata_components::collection::keyboard::{Command, KeyMap, Letters, Target, Vertical};
use strata_components::contract::Direction;
use strata_components::footer::reference::{
    Keys, Section, Shortcut, ShortcutData, chord_label, keymap_rows,
};

/// Which view's navigation section the reference shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    Columns,
    Icons,
    #[default]
    List,
}

impl View {
    fn title(self) -> &'static str {
        match self {
            View::Columns => "Columns navigation",
            View::Icons => "Icons navigation",
            View::List => "List navigation",
        }
    }
}

/// What the catalog depends on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scope {
    pub view: View,
    pub tenxer: bool,
    /// Type to search off: `h j k l` stand in for the arrows.
    pub arrows: bool,
    /// Ctrl+K unbound from global search.
    pub search_unbound: bool,
    /// The interface in German (the search's locale script).
    pub german: bool,
}

/// The navigation commands Strata names, in its words.
fn describe(command: Command) -> Option<String> {
    use Direction::*;
    let s = match command {
        Command::Select(Target::Step(Up)) | Command::Cursor(Target::Step(Up)) => "Previous item",
        Command::Select(Target::Step(Down)) | Command::Cursor(Target::Step(Down)) => "Next item",
        Command::Select(Target::Step(Left)) | Command::Cursor(Target::Step(Left)) => {
            "Go to the parent folder"
        }
        Command::Select(Target::Step(Right)) | Command::Cursor(Target::Step(Right)) => {
            "Open the focused directory"
        }
        Command::Extend(Target::Step(Up)) => "Extend the selection up",
        Command::Extend(Target::Step(Down)) => "Extend the selection down",
        Command::Extend(Target::Step(Left)) => "Extend the selection left",
        Command::Extend(Target::Step(Right)) => "Extend the selection right",
        Command::Select(Target::First) | Command::Cursor(Target::First) => "First item",
        Command::Select(Target::Last) | Command::Cursor(Target::Last) => "Last item",
        Command::Extend(Target::First) => "Extend to the first item",
        Command::Extend(Target::Last) => "Extend to the last item",
        Command::Select(Target::Page(Vertical::Up))
        | Command::Cursor(Target::Page(Vertical::Up)) => "Page up",
        Command::Select(Target::Page(Vertical::Down))
        | Command::Cursor(Target::Page(Vertical::Down)) => "Page down",
        Command::Extend(Target::Page(Vertical::Up)) => "Extend a page up",
        Command::Extend(Target::Page(Vertical::Down)) => "Extend a page down",
        Command::Cursor(Target::HalfPage(Vertical::Up)) => "Half a page up",
        Command::Cursor(Target::HalfPage(Vertical::Down)) => "Half a page down",
        Command::Nudge(Left) => "Move the cursor left, keeping the selection",
        Command::Nudge(Right) => "Move the cursor right, keeping the selection",
        Command::Toggle => "Toggle the focused item",
        Command::ToggleAdvance => "Toggle the focused item and move down",
        Command::SelectAll => "Select all",
        Command::Invert => "Invert the selection",
        Command::Range(strata_components::collection::keyboard::RangeKind::Select) => {
            "Start or end a range"
        }
        Command::Range(strata_components::collection::keyboard::RangeKind::Unset) => {
            "Start or end an unselect range"
        }
        Command::Clear => "Clear the selection",
        Command::Activate => "Open the focused item",
        _ => return None,
    };
    Some(s.to_owned())
}

fn one(action: &str, key: &str) -> Shortcut {
    Shortcut::new(action, Keys::one(key))
}

/// The catalog for `scope`, at `revision`.
pub fn data(scope: Scope, revision: u64) -> ShortcutData {
    if scope.german {
        return german(revision);
    }
    let map = if scope.tenxer {
        KeyMap::tenxer()
    } else {
        KeyMap::default_map()
    };
    let letters = if scope.arrows {
        Letters::Arrows
    } else {
        Letters::Pass
    };
    let nav = keymap_rows(
        &map,
        letters,
        describe,
        Some("Same as \u{2190} \u{2193} \u{2191} \u{2192} (type-to-search off)"),
        chord_label,
    );
    let mut files = vec![
        one("Copy", "Ctrl+C").search(["ctrl+c"]),
        one("Cut", "Ctrl+X"),
        one("Paste", "Ctrl+V"),
        one("Rename the focused item", "F2"),
        Shortcut::new(
            "Rename the next / previous item",
            Keys::any(["Tab", "Shift+Tab"]),
        )
        .context("", " in a rename"),
        one("Duplicate", "Ctrl+D"),
        one("Move to Trash", "Delete"),
        one("Delete permanently", "Shift+Delete"),
        Shortcut::new(
            "Open the focused folder in a new tab / window",
            Keys::any(["Ctrl+Enter", "Shift+Enter"]),
        ),
    ];
    let mut tools = vec![
        one("Filter the list", "Ctrl+F"),
        one("Reopen the last closed tab", "Ctrl+Shift+T"),
        Shortcut::new(
            "Increase / decrease / reset text size",
            Keys::any(["Ctrl++", "\u{2212}", "0"]),
        )
        .search(["ctrl++", "ctrl+-", "ctrl+0"]),
        Shortcut::new(
            "Go to tab 1–9 / the last tab",
            Keys::any(["Ctrl+Shift+1\u{2013}9", "0"]),
        )
        .search(["ctrl+shift+1-9", "ctrl+shift+0"]),
        one("Open Settings", "Ctrl+,"),
        Shortcut::new("Show or hide hidden files", Keys::any(["Ctrl+H", "Ctrl+."])),
    ];
    if !scope.search_unbound {
        tools.insert(0, one("Search everywhere", "Ctrl+K"));
    }
    let mut sections = vec![Section {
        title: scope.view.title().to_owned(),
        rows: nav,
    }];
    if scope.tenxer {
        files.retain(|r| r.action != "Rename the focused item");
        files.insert(3, one("Rename the focused item in the footer", "r"));
        sections.push(Section {
            title: "Files and selection".to_owned(),
            rows: files,
        });
        sections.push(Section {
            title: "Places".to_owned(),
            rows: vec![
                Shortcut::new("First item", Keys::seq(["g", "g"])),
                Shortcut::new("Home folder", Keys::seq(["g", "h"])),
                Shortcut::new("Downloads", Keys::seq(["g", "d"])),
                Shortcut::new("Type a path", Keys::seq(["g", "Space"])),
            ],
        });
        sections.push(Section {
            title: "10xer mode".to_owned(),
            rows: vec![
                one("Filter this folder", "f"),
                one("Search below this folder", "s"),
                Shortcut::new("Find next / previous", Keys::any(["/", "?"])),
                one("Shortcut reference", "~"),
            ],
        });
    } else {
        sections.push(Section {
            title: "Files and selection".to_owned(),
            rows: files,
        });
        sections.push(Section {
            title: "Search and tools".to_owned(),
            rows: tools,
        });
        sections.push(Section {
            title: "Preview media".to_owned(),
            rows: vec![
                one("Play or pause", "Space"),
                Shortcut::new(
                    "Seek \u{2212}5 / +5 seconds",
                    Keys::any(["Shift+\u{2190}", "Shift+\u{2192}"]),
                )
                .context("", " in media"),
            ],
        });
    }
    ShortcutData { sections, revision }
}

/// A few rows in German, with English search terms beside them, as Strata's search matches
/// key names in English and in the interface language.
fn german(revision: u64) -> ShortcutData {
    ShortcutData {
        sections: vec![Section {
            title: "Dateien und Auswahl".to_owned(),
            rows: vec![
                one("Kopieren", "Strg+C").search(["Copy", "ctrl+c"]),
                one("Ausschneiden", "Strg+X").search(["Cut", "ctrl+x"]),
                one("Einfügen", "Strg+V").search(["Paste", "ctrl+v"]),
                one("In den Papierkorb verschieben", "Entf").search(["Move to Trash", "delete"]),
            ],
        }],
        revision,
    }
}

/// `rows` rows in 40 sections: the performance fixture.
pub fn large(rows: usize, revision: u64) -> ShortcutData {
    let per = rows.div_ceil(40);
    let sections = (0..40)
        .map(|s| Section {
            title: format!("Section {s}"),
            rows: (0..per)
                .filter(|r| s * per + r < rows)
                .map(|r| {
                    Shortcut::new(
                        format!("Action {} of section {s}", s * per + r),
                        Keys::any([format!("Ctrl+{}", r % 10), format!("Alt+{}", r % 7)]),
                    )
                })
                .collect(),
        })
        .collect();
    ShortcutData { sections, revision }
}

/// One entry of the page's listing.
pub struct Entry {
    pub name: &'static str,
    pub folder: bool,
    pub size: Option<u64>,
}

/// The listing the footer counts: two folders and three files, one of unknown size.
pub const ENTRIES: [Entry; 5] = [
    Entry {
        name: "Documents",
        folder: true,
        size: None,
    },
    Entry {
        name: "Music",
        folder: true,
        size: None,
    },
    Entry {
        name: "notes.txt",
        folder: false,
        size: Some(1_200),
    },
    Entry {
        name: "photo.png",
        folder: false,
        size: Some(2_400_000),
    },
    Entry {
        name: "archive.zip",
        folder: false,
        size: None,
    },
];

/// The page's `s` search hits, relative to the searched folder.
pub const HITS: [&str; 3] = [
    "git/trading/README.md",
    "notes/2026/october/minutes.txt",
    "photo.png",
];
