//! The gallery page for C18 FilterBar and FooterPrompt: its scenes, demo state, view and key and
//! poll hooks. Scene names start with `c18-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-2-composites/C18-filter-bar-footer-prompt.md`.
//!
//! A stand-in file pane over one fake folder: a header with the filter toggle, the filter bar,
//! the listing (a C1e collection whose keys come from the pane, its one focus stop, so a chord
//! hears every key first), a footer with the prompt, the flash and the chord pill, and two
//! readouts: `c18-outlined` (the Enter target's outline) and `c18-log` (what the host last
//! did). The page stands in for Strata's key map and policy: matching is a case-insensitive
//! substring, and "opening" logs.
//!
//! | Key | Map | Does |
//! | --- | --- | --- |
//! | Ctrl+F, `/` | default | Open the filter |
//! | A printable key in the listing | default | Type to search: open the filter with it |
//! | Escape in the listing | default | Close an open filter |
//! | Up on the first result | default | Back to the filter field |
//! | `/`, `?`, `f`, `s`, `r`, F2 | 10xer | Open the find, filter, search and rename prompts |
//! | `g`, `,`, `c` | 10xer | Arm the go, sort and copy chords; `g Space` opens `go ›` |
//! | Ctrl+Shift+M | both | Toggle 10xer mode, closing prompts and chords |
//! | F6 | both | Another program adds `report-new.pdf` to the folder |
//! | F7 | both | Another program deletes the cursor's entry |

use rune::{Chord, Cx, El, Key, KeyEvent, Mods, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant};
use strata_components::collection::element::{Collection, CollectionState, Intent, RowMarks};
use strata_components::collection::keyboard::KeySettings;
use strata_components::collection::selection::{AnchorRule, SyncRule};
use strata_components::context_menu::{FieldMenu, FieldMenuState};
use strata_components::contract::{EntryId, EntryKind, EntrySource, ListEntry, ListSource};
use strata_components::filter_bar::chord::{self, ChordMap, ChordOption, ChordOutcome, ChordState};
use strata_components::filter_bar::filter::{
    FilterBar, FilterIntent, FilterLabels, FilterState, Results, Shows, status,
};
use strata_components::filter_bar::prompt::{
    By, Candidate, Candidates, FooterPrompt, PromptIntent, PromptSpec, PromptState,
};
use strata_components::filter_bar::{Flash, flash};
use strata_kit::look::Look;

use crate::{Gallery, Scene};

/// The listing's key, and its focus stop's.
pub const SCOPE: &str = "c18-list";
pub const PANE: &str = "c18-pane";
const BAR: &str = "c18-filter";
const PROMPT: &str = "c18-prompt";
const CHORD: &str = "c18-chord";

/// The page's prompt kinds, as Strata's `Prompt` names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Find,
    FindBackward,
    Filter,
    Search,
    Go,
    Rename,
}

/// What a chord's second key does here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    First,
    Last,
    Place(&'static str),
    Pin(u8),
    TypePath,
    Sort(&'static str, bool),
    CopyPath,
    CopyName,
}

/// C18's demo state, held in `Gallery::c18`.
pub struct Demo {
    /// The folder's title (Documents, or Trash for the in-place scene).
    pub title: &'static str,
    /// The folder on "disk".
    pub disk: Vec<ListEntry>,
    /// What the listing shows: the folder, or the filter's results.
    pub listing: ListSource,
    pub coll: CollectionState,
    pub version: u64,
    pub filter: FilterState,
    pub prompt: PromptState<Kind>,
    pub chord: ChordState<Act>,
    /// C21's text menu over the filter field or the prompt's.
    pub fields: FieldMenuState<Field>,
    pub flash: Flash,
    pub tenxer: bool,
    /// The 10xer `f` filter's committed query, and `s`'s.
    pub listing_filter: String,
    pub search: String,
    /// The fake folder picker: answers due at a clock time.
    pub worker: Vec<(f64, Candidates)>,
    /// The entry `rename ›` renames.
    pub renaming: Option<EntryId>,
    pub next_id: u64,
    pub now: f64,
    pub log: String,
    /// A query waiting out the debounce: when it applies, and its text.
    pub pending_query: Option<(f64, String)>,
    /// Queries shown since the scene opened.
    pub queries: u32,
}

impl Default for Demo {
    fn default() -> Self {
        let disk = documents();
        let mut coll = CollectionState::new();
        coll.controllers.pointer = Some(Box::new(
            strata_components::collection::pointer::PointerIntent::new(),
        ));
        coll.controllers.keyboard =
            Some(Box::<strata_components::collection::keyboard::Keyboard>::default());
        Self {
            title: "Documents",
            listing: ListSource::new(disk.clone()),
            disk,
            coll,
            version: 1,
            filter: FilterState::new(),
            prompt: PromptState::new(),
            chord: ChordState::new(),
            fields: FieldMenuState::new(),
            flash: Flash::new(),
            tenxer: false,
            listing_filter: String::new(),
            search: String::new(),
            worker: Vec::new(),
            renaming: None,
            next_id: 100,
            now: 0.0,
            log: String::new(),
            pending_query: None,
            queries: 0,
        }
    }
}

/// The fake folder: folders first, then files, by name.
fn documents() -> Vec<ListEntry> {
    let folders = ["Projects", "Receipts", "reports"];
    let files = [
        ("budget.ods", "file"),
        ("duck.txt", "file_text"),
        ("ducks_in_the_pool.txt", "file_text"),
        ("holiday.jpg", "file_image"),
        ("main.rs", "file_code"),
        ("notes.md", "file_text"),
        ("report-2025.ods", "file"),
        ("report.pdf", "file"),
        ("scan.pdf", "file"),
    ];
    folders
        .iter()
        .map(|n| (*n, EntryKind::Folder, "folder"))
        .chain(files.iter().map(|(n, i)| (*n, EntryKind::File, *i)))
        .enumerate()
        .map(|(i, (n, k, icon))| ListEntry::new(EntryId::new(1 + i as u64), n, k, icon))
        .collect()
}

/// The folders `go ›` picks from, below the fake home.
const FOLDERS: &[&str] = &[
    "Desktop",
    "dev",
    "dev/strata",
    "dev/strata-components",
    "Documents",
    "Downloads",
    "Music",
    "Pictures",
    "Videos",
];

/// How long an edit's query waits for the next edit (Strata's `FILTER_DEBOUNCE_DELAY`).
const QUERY_DEBOUNCE: f64 = 0.04;

/// How long the fake folder picker takes to answer.
const PICKER_DELAY: f64 = 0.3;

fn matches(name: &str, query: &str) -> bool {
    name.to_lowercase().contains(&query.trim().to_lowercase())
}

fn results(g: &Gallery) -> Results<'_> {
    Results {
        source: &g.c18.listing,
        selection: &g.c18.coll.selection,
    }
}

fn filter_state(g: &Gallery) -> &FilterState {
    &g.c18.filter
}

fn filter_state_mut(g: &mut Gallery) -> &mut FilterState {
    &mut g.c18.filter
}

fn prompt_state(g: &Gallery) -> &PromptState<Kind> {
    &g.c18.prompt
}

fn prompt_state_mut(g: &mut Gallery) -> &mut PromptState<Kind> {
    &mut g.c18.prompt
}

fn bar(g: &Gallery) -> FilterBar<Gallery> {
    let n = g.c18.disk.len();
    FilterBar::new(BAR, filter_state, filter_state_mut)
        .labels(FilterLabels {
            field: "Filter list (Ctrl+F)".to_owned(),
            placeholder: format!("Filter {n} items\u{2026}"),
        })
        .results(results)
        .focus_return(PANE)
        .previews(|g: &Gallery| {
            let d = &g.c18;
            d.coll
                .selection
                .selected_ids(&d.listing)
                .iter()
                .filter_map(|id| d.listing.index_of(*id))
                .any(|i| d.listing.entry(i).kind == EntryKind::File)
        })
        .on_intent(|g: &mut Gallery, intent, tx| match intent {
            FilterIntent::FieldMenu { at } => field_menu().open(g, Field::Filter, at, tx),
            intent => g.c18.filtered(intent, tx),
        })
}

fn prompt() -> FooterPrompt<Gallery, Kind> {
    FooterPrompt::new(PROMPT, prompt_state, prompt_state_mut)
        .focus_return(PANE)
        .on_intent(|g: &mut Gallery, intent, tx| match intent {
            // The menu takes the keyboard: hold the prompt open until it gives it back.
            PromptIntent::FieldMenu { at } => {
                g.c18.prompt.hold_open(true);
                field_menu().open(g, Field::Prompt, at, tx);
                if !g.c18.fields.is_open() {
                    g.c18.prompt.hold_open(false);
                }
            }
            intent => g.c18.prompted(intent, tx),
        })
}

/// The fields C21's text menu opens over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Filter,
    Prompt,
}

fn field_edit_mut(g: &mut Gallery, f: Field) -> Option<&mut rune::TextEdit> {
    let d = &mut g.c18;
    match f {
        Field::Filter if d.filter.is_open() => Some(&mut d.filter.edit),
        Field::Prompt if d.prompt.is_open() => Some(&mut d.prompt.edit),
        _ => None,
    }
}

fn field_edit(g: &Gallery, f: Field) -> Option<&rune::TextEdit> {
    let d = &g.c18;
    match f {
        Field::Filter if d.filter.is_open() => Some(&d.filter.edit),
        Field::Prompt if d.prompt.is_open() => Some(&d.prompt.edit),
        _ => None,
    }
}

/// C21's text menu for the filter field and the prompt's (Cut, Copy, Paste, Delete, Select
/// All). An edit it makes reaches the bar or the prompt as typing does; closing it gives the
/// field the keyboard back and lets the prompt close on a blur again.
fn field_menu() -> FieldMenu<Gallery, Field> {
    FieldMenu::new(
        "c18-field-menu",
        |g: &Gallery| &g.c18.fields,
        |g: &mut Gallery| &mut g.c18.fields,
        field_edit_mut,
        field_edit,
    )
    .on_change(|g: &mut Gallery, f, tx| match f {
        Field::Filter => {
            let before = {
                let r = results(g);
                r.selection.position(r.source)
            };
            let polled = g.c18.filter.text_edited(before);
            bar(g).route(g, polled, tx);
        }
        Field::Prompt => {
            let polled = g.c18.prompt.text_edited();
            prompt().route(g, polled, tx);
        }
    })
    .on_close(|g: &mut Gallery, f| {
        if f == Field::Prompt {
            g.c18.prompt.hold_open(false);
        }
        g.c18.log = format!("{f:?} menu closed");
    })
}

fn collection(tenxer: bool) -> Collection<Gallery> {
    Collection::new(
        SCOPE,
        |g: &Gallery| (&g.c18.listing, &g.c18.coll),
        |g: &mut Gallery| (&g.c18.listing, &mut g.c18.coll),
        |g: &Gallery| g.look(),
        |g: &Gallery| g.c18.version,
    )
    .name("Files")
    .settings(if tenxer {
        KeySettings::tenxer()
    } else {
        KeySettings::default()
    })
    .keys_by_host()
    .marks(|g: &Gallery, id| {
        let mut m = RowMarks::default();
        m.enter_target = g.c18.filter.outlined(results(g)) == Some(id);
        m
    })
    .on_selection(|g: &mut Gallery, _| g.c18.filter.pick())
    .on_press(|g: &mut Gallery, _, _| g.c18.filter.results_scrolled())
    .on_intent(|g: &mut Gallery, intent, _| g.c18.heard(intent))
    .empty(|g: &Gallery| {
        g.c18
            .filter
            .is_open()
            .then(|| status(g.look(), "c18-status", "No matching files"))
    })
}

fn chords(map: &str) -> ChordMap<Act> {
    let c = |s: &str| Chord::parse(s).expect("a chord");
    let line = |cap: &str, label: &str, chord: &str, act: Act| {
        ChordOption::new(cap, label, vec![(c(chord), act)])
    };
    match map {
        "g" => ChordMap {
            mark: "g-".into(),
            name: "Keys after g".into(),
            options: vec![
                line("g", "First item", "g", Act::First),
                line(
                    "f",
                    "Follow search result",
                    "f",
                    Act::Place("the hit's folder"),
                ),
                line("h", "Home", "h", Act::Place("Home")),
                line("d", "Downloads", "d", Act::Place("Downloads")),
                line("c", "Config", "c", Act::Place("~/.config")),
                line("t", "Trash", "t", Act::Place("Trash")),
                line("n", "Network", "n", Act::Place("Network")),
                line("r", "Recent", "r", Act::Place("Recent")),
                line("k", "Documents", "k", Act::Place("Documents")),
                line("m", "Music", "m", Act::Place("Music")),
                line("p", "Pictures", "p", Act::Place("Pictures")),
                line("v", "Videos", "v", Act::Place("Videos")),
                ChordOption::new(
                    "1\u{2013}9",
                    "Pins",
                    (1..=9u8)
                        .map(|n| (c(&n.to_string()), Act::Pin(n)))
                        .collect(),
                ),
                ChordOption::new(
                    "+ / \u{2212}",
                    "Pin / unpin folder",
                    vec![
                        (c("plus"), Act::Place("pin")),
                        (c("minus"), Act::Place("unpin")),
                    ],
                ),
                line("Space", "Type a path", "space", Act::TypePath),
            ],
        },
        "," => {
            // Strata's sort keys (`src/ui/keys/bindings/tenxer.rs`); Shift reverses the
            // orders that have a direction.
            let sort = |cap: &'static str, label: &'static str, reverses: bool| {
                let mut chords = vec![(c(cap), Act::Sort(label, false))];
                if reverses {
                    chords.push((c(&format!("shift+{cap}")), Act::Sort(label, true)));
                }
                ChordOption::new(cap, label, chords)
            };
            ChordMap {
                mark: ",-".into(),
                name: "Keys after ,".into(),
                options: vec![
                    sort("a", "Name", true),
                    sort("m", "Modified", true),
                    sort("s", "Size", true),
                    sort("e", "Type", true),
                    sort("x", "Extension", true),
                    sort("b", "Created", true),
                    sort("u", "Unsorted", false),
                    sort("n", "Natural", false),
                    sort("l", "Alphabetical", false),
                    sort("c", "Case-sensitive names", false),
                    sort("f", "Folders first", false),
                    ChordOption::mention("Shift", "Reverse"),
                ],
            }
        }
        _ => ChordMap {
            mark: "c-".into(),
            name: "Keys after c".into(),
            options: vec![
                line("c", "Copy path", "c", Act::CopyPath),
                line("n", "Copy name", "n", Act::CopyName),
            ],
        },
    }
}

/// A file's stem, up to its last `.`; a folder's whole name (Strata's rule, C5's
/// `initial_selection` once merged).
fn stem(name: &str, folder: bool) -> std::ops::Range<usize> {
    match name.rfind('.') {
        Some(i) if !folder && i > 0 => 0..i,
        _ => 0..name.len(),
    }
}

impl Demo {
    fn name_of(&self, id: EntryId) -> String {
        self.listing
            .index_of(id)
            .map(|i| self.listing.entry(i).name.to_owned())
            .or_else(|| {
                self.disk
                    .iter()
                    .find(|e| e.id == id)
                    .map(|e| e.name.clone())
            })
            .unwrap_or_default()
    }

    /// Shows the folder's entries `keep` keeps, and brings the selection in step.
    fn show(&mut self, keep: impl Fn(&ListEntry) -> bool) {
        let shown: Vec<ListEntry> = self.disk.iter().filter(|e| keep(e)).cloned().collect();
        self.listing.replace(shown);
        self.version += 1;
        let _ = self.coll.selection.sync(&self.listing, SyncRule::default());
    }

    /// What the listing shows now: the Ctrl+F query, or the 10xer filter or search.
    fn refresh(&mut self) {
        let query = if self.filter.is_open() && !self.filter.text().trim().is_empty() {
            self.filter.text().to_owned()
        } else if !self.search.is_empty() {
            self.search.clone()
        } else {
            self.listing_filter.clone()
        };
        self.show(|e| query.trim().is_empty() || matches(&e.name, &query));
    }

    /// The filter bar's intents: the page filters, opens and previews.
    fn filtered(&mut self, intent: FilterIntent, tx: &mut Tx) {
        match intent {
            // Strata settles a query 40 ms after the last edit (`FILTER_DEBOUNCE_DELAY`).
            FilterIntent::Query { text } => {
                self.pending_query = Some((self.now + QUERY_DEBOUNCE, text));
                tx.wake_after(std::time::Duration::from_secs_f64(QUERY_DEBOUNCE));
            }
            FilterIntent::ShowListing { restore } => {
                self.pending_query = None;
                self.listing.replace(self.disk.clone());
                self.version += 1;
                if let Some(p) = restore {
                    let _ = self.coll.selection.restore(&p);
                }
                let _ = self.coll.selection.sync(&self.listing, SyncRule::default());
                if let Some(i) = self.coll.selection.cursor_index(&self.listing) {
                    self.coll.reveal(SCOPE, &self.listing, i, tx);
                }
            }
            FilterIntent::Closed => {
                self.pending_query = None;
                self.log = "Filter closed".into();
            }
            FilterIntent::Open { id } => self.log = format!("Opened {}", self.name_of(id)),
            FilterIntent::FocusResults { id } => {
                if let Some(i) = self.listing.index_of(id) {
                    let _ = self.coll.selection.select_only(&self.listing, i);
                    self.coll.keyboard_moved();
                    self.coll.reveal(SCOPE, &self.listing, i, tx);
                }
                self.log = format!("Down to {}", self.name_of(id));
            }
            FilterIntent::Preview => self.log = "Quick preview".into(),
            // C21's FieldMenu answers this in Strata; it isn't merged yet.
            _ => {}
        }
    }

    /// A settled query: the results show, and the bar hears which query they are for.
    fn apply_query(&mut self, text: &str, tx: &mut Tx) {
        self.show(|e| matches(&e.name, text));
        if self.filter.shown(text) {
            let _ = self.coll.selection.clear();
        }
        self.reveal_top(tx);
        self.queries += 1;
        let n = self.queries;
        self.log = format!("Query “{text}”, {n} shown");
    }

    fn reveal_top(&mut self, tx: &mut Tx) {
        let r = Results {
            source: &self.listing,
            selection: &self.coll.selection,
        };
        if let Some(i) = self.filter.take_reveal(r) {
            self.coll.reveal(SCOPE, &self.listing, i, tx);
        }
    }

    /// The collection's intents.
    fn heard(&mut self, intent: Intent) {
        match intent {
            Intent::Activate { id, .. } => self.log = format!("Opened {}", self.name_of(id)),
            Intent::ContextMenu { .. } => self.log = "Folder menu".into(),
            _ => {}
        }
    }

    fn open_prompt(
        &mut self,
        kind: Kind,
        spec: PromptSpec,
        text: &str,
        select: Option<std::ops::Range<usize>>,
        tx: &mut Tx,
    ) {
        self.chord.clear();
        let polled = self.prompt.open(kind, spec, text, select);
        self.route_prompt(polled, tx);
    }

    fn route_prompt(
        &mut self,
        polled: strata_components::filter_bar::prompt::Polled<Kind>,
        tx: &mut Tx,
    ) {
        for intent in polled.intents {
            self.prompted(intent, tx);
        }
    }

    /// The prompts' intents: the page finds, filters, searches, goes and renames.
    fn prompted(&mut self, intent: PromptIntent<Kind>, tx: &mut Tx) {
        match intent {
            PromptIntent::Edited { kind, text, .. } => match kind {
                Kind::Filter => {
                    self.listing_filter = text;
                    self.refresh();
                }
                Kind::Search => {
                    self.search = text;
                    self.refresh();
                }
                _ => {}
            },
            PromptIntent::Complete { query, .. } => {
                let typed = query.text.trim_start_matches("~/").to_lowercase();
                let items = FOLDERS
                    .iter()
                    // A typed folder ending in `/` comes first, then the folders below it.
                    .filter(|p| {
                        let p = p.to_lowercase();
                        typed.is_empty()
                            || p.starts_with(&typed)
                            || typed.strip_suffix('/') == Some(p.as_str())
                    })
                    .map(|p| {
                        let (parent, name) = p.rsplit_once('/').unwrap_or(("", p));
                        let hint = if parent.is_empty() {
                            "~".to_owned()
                        } else {
                            format!("~/{parent}")
                        };
                        let last = typed.rsplit('/').next().unwrap_or("");
                        let n = if name.to_lowercase().starts_with(last) {
                            last.len()
                        } else {
                            0
                        };
                        Candidate::folder(format!("~/{p}"), name, hint, format!("~/{p}/"))
                            .matched(std::iter::once(0..n))
                    })
                    .collect();
                self.worker.push((
                    self.now + PICKER_DELAY,
                    Candidates {
                        generation: query.generation,
                        items,
                        done: true,
                    },
                ));
            }
            PromptIntent::CancelCompletion => self.worker.clear(),
            PromptIntent::Step { delta, .. } => self.step_cursor(delta),
            submit @ PromptIntent::Submit { .. } => self.submitted(submit, tx),
            PromptIntent::Dismissed { kind, text, by } => {
                match kind {
                    Kind::Filter if by == By::Escape => {
                        self.listing_filter.clear();
                        self.refresh();
                    }
                    // A non-empty search keeps its hits; an empty one ends.
                    Kind::Search if text.is_empty() => {
                        self.search.clear();
                        self.refresh();
                    }
                    _ => {}
                }
                self.renaming = None;
                self.log = format!("Dismissed {kind:?} by {by:?}");
            }
            _ => {}
        }
    }

    fn submitted(&mut self, submit: PromptIntent<Kind>, tx: &mut Tx) {
        let PromptIntent::Submit {
            kind,
            text,
            chosen,
            stepped,
            settled,
            serial,
        } = submit
        else {
            return;
        };
        match kind {
            Kind::Find | Kind::FindBackward => {
                let _ = self.prompt.close();
                if text.is_empty() {
                    return;
                }
                self.find(&text, kind == Kind::FindBackward, tx);
            }
            Kind::Filter => {
                let _ = self.prompt.close();
                self.listing_filter = text;
                self.refresh();
                let _ = self
                    .coll
                    .selection
                    .move_cursor(&self.listing, 0, AnchorRule::Move);
                self.log = format!("Filter “{}”", self.listing_filter);
            }
            Kind::Search => {
                let _ = self.prompt.close();
                self.search = text;
                self.refresh();
                let _ = self
                    .coll
                    .selection
                    .move_cursor(&self.listing, 0, AnchorRule::Move);
                self.log = format!("Search “{}”", self.search);
            }
            Kind::Go => {
                if text.trim().is_empty() {
                    let _ = self.prompt.close();
                    return;
                }
                if !stepped && !settled && self.prompt.wait_until_settled(serial) {
                    self.log = "Waiting for the folders".into();
                    return;
                }
                let _ = self.prompt.close();
                self.log = format!("Go {}", chosen.unwrap_or(text));
            }
            Kind::Rename => self.rename(text, serial),
        }
    }

    fn find(&mut self, text: &str, backward: bool, tx: &mut Tx) {
        let n = self.listing.len();
        if n == 0 {
            return;
        }
        // From the cursor, wrapping; with no cursor, from the first entry (the last going
        // back) itself.
        let from = match self.coll.selection.cursor_index(&self.listing) {
            Some(i) => i,
            None if backward => 0,
            None => n - 1,
        };
        let hit = (1..=n)
            .map(|k| {
                if backward {
                    (from + n - k) % n
                } else {
                    (from + k) % n
                }
            })
            .find(|&i| matches(self.listing.entry(i).name, text));
        match hit {
            Some(i) => {
                let _ = self
                    .coll
                    .selection
                    .move_cursor(&self.listing, i, AnchorRule::Move);
                self.coll.keyboard_moved();
                self.log = format!("Found {}", self.listing.entry(i).name);
            }
            None => self.flash.show(format!("No matches for “{text}”"), tx),
        }
    }

    fn rename(&mut self, text: String, serial: u64) {
        let Some(id) = self.renaming else {
            let _ = self.prompt.close();
            return;
        };
        let old = self.name_of(id);
        if text.is_empty() || text == old {
            let _ = self.prompt.close();
            self.renaming = None;
            return;
        }
        if self.disk.iter().any(|e| e.name == text && e.id != id) {
            self.prompt
                .refuse(serial, format!("“{text}” already exists"));
            return;
        }
        if let Some(e) = self.disk.iter_mut().find(|e| e.id == id) {
            e.name = text.clone();
        }
        let _ = self.prompt.close();
        self.renaming = None;
        self.refresh();
        self.log = format!("Renamed {old} to {text}");
    }

    fn step_cursor(&mut self, delta: i32) {
        let n = self.listing.len();
        if n == 0 {
            return;
        }
        let at = self.coll.selection.cursor_index(&self.listing);
        let next = match at {
            None if delta > 0 => 0,
            None => n - 1,
            Some(i) => (i as i64 + i64::from(delta)).clamp(0, n as i64 - 1) as usize,
        };
        let _ = self
            .coll
            .selection
            .move_cursor(&self.listing, next, AnchorRule::Keep);
        self.coll.keyboard_moved();
        self.log = format!("Cursor on {}", self.listing.entry(next).name);
    }

    fn completed(&mut self, act: Act, tx: &mut Tx) {
        match act {
            Act::First => self.step_cursor(i32::MIN / 2),
            Act::Last => self.step_cursor(i32::MAX / 2),
            Act::TypePath => self.open_prompt(Kind::Go, PromptSpec::go(), "", None, tx),
            Act::Place(p) => self.log = format!("Go {p}"),
            Act::Pin(n) => self.flash.show(format!("No pin {n}"), tx),
            Act::Sort(by, reverse) => {
                self.log = format!("Sort by {by}{}", if reverse { ", reversed" } else { "" })
            }
            Act::CopyPath => self.log = "Copied the path".into(),
            Act::CopyName => self.log = "Copied the name".into(),
        }
    }

    fn set_tenxer(&mut self, on: bool, tx: &mut Tx) {
        self.tenxer = on;
        self.filter.set_tenxer(on);
        if !on {
            // Leaving the mode closes the prompts and chords, and ends its filters.
            let polled = self.prompt.close();
            self.route_prompt(polled, tx);
            self.chord.clear();
            self.listing_filter.clear();
            self.search.clear();
            self.refresh();
        }
        self.log = format!("10xer mode {}", if on { "on" } else { "off" });
    }

    /// Another program adds a file, or deletes the cursor's entry: the listing follows.
    fn disk_changed(&mut self, add: bool, tx: &mut Tx) {
        if add {
            let id = EntryId::new(self.next_id);
            self.next_id += 1;
            self.disk.push(ListEntry::new(
                id,
                "report-new.pdf",
                EntryKind::File,
                "file",
            ));
            self.disk.sort_by(|a, b| {
                (a.kind != EntryKind::Folder, a.name.to_lowercase())
                    .cmp(&(b.kind != EntryKind::Folder, b.name.to_lowercase()))
            });
            self.log = "Added report-new.pdf".into();
        } else if let Some(id) = self.coll.selection.cursor() {
            let name = self.name_of(id);
            self.coll
                .selection
                .before_removal(&self.listing, |e| e == id);
            self.disk.retain(|e| e.id != id);
            self.log = format!("Deleted {name}");
        }
        self.refresh();
        // A same-query update keeps the pick and the selection.
        if self.filter.is_open() {
            let text = self.filter.text().to_owned();
            let _ = self.filter.shown(&text);
            self.reveal_top(tx);
        }
    }
}

// Scenes.

fn scene(g: &mut Gallery, dark: bool) {
    g.open("c18", dark);
    g.c18 = Demo::default();
}

fn by_name(d: &Demo, name: &str) -> usize {
    (0..d.listing.len())
        .find(|&i| d.listing.entry(i).name == name)
        .unwrap_or_else(|| panic!("no {name}"))
}

/// The filter open on `query`, its results shown, the field focused.
fn filtering(g: &mut Gallery, dark: bool, query: &str) {
    scene(g, dark);
    let d = &mut g.c18;
    let _ = d.filter.open();
    d.filter.edit = rune::TextEdit::new(query);
    d.filter.edit.set_caret(query.len());
    d.filter.edit.focused = true;
    if !query.is_empty() {
        d.show(|e| matches(&e.name, query));
        let _ = d.filter.shown(query);
    }
}

fn filter_open(g: &mut Gallery, dark: bool) {
    filtering(g, dark, "");
}

fn filter_target(g: &mut Gallery, dark: bool) {
    filtering(g, dark, "rep");
}

fn filter_picked(g: &mut Gallery, dark: bool) {
    filtering(g, dark, "rep");
    let d = &mut g.c18;
    let i = by_name(d, "report.pdf");
    let _ = d.coll.selection.select_only(&d.listing, i);
    d.filter.pick();
}

fn filter_in_place(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    let d = &mut g.c18;
    d.title = "Trash";
    d.filter.set_shows(Shows::InPlace);
    let i = by_name(d, "notes.md");
    let _ = d.coll.selection.select_only(&d.listing, i);
    let _ = d.filter.open();
    d.filter.edit = rune::TextEdit::new("d");
    d.filter.edit.set_caret(1);
    d.filter.edit.focused = true;
    d.show(|e| matches(&e.name, "d"));
    let _ = d.filter.shown("d");
}

fn filter_empty(g: &mut Gallery, dark: bool) {
    filtering(g, dark, "zebra");
}

fn tenxer(g: &mut Gallery, dark: bool) {
    scene(g, dark);
    g.c18.set_tenxer(true, &mut Tx::default());
    g.c18.log.clear();
}

fn prompt_find(g: &mut Gallery, dark: bool) {
    tenxer(g, dark);
    g.c18.open_prompt(
        Kind::Find,
        PromptSpec::find(),
        "rep",
        None,
        &mut Tx::default(),
    );
}

fn prompt_filter(g: &mut Gallery, dark: bool) {
    tenxer(g, dark);
    let d = &mut g.c18;
    d.listing_filter = "rep".into();
    d.refresh();
    d.open_prompt(
        Kind::Filter,
        PromptSpec::filter(),
        "rep",
        None,
        &mut Tx::default(),
    );
}

fn prompt_search(g: &mut Gallery, dark: bool) {
    tenxer(g, dark);
    let d = &mut g.c18;
    d.open_prompt(
        Kind::Search,
        PromptSpec::search(),
        "duck",
        None,
        &mut Tx::default(),
    );
    d.search = "duck".into();
    d.refresh();
    let serial = d.prompt.serial();
    d.prompt.show_hint(serial, Some("2 hits".into()));
}

fn prompt_go(g: &mut Gallery, dark: bool) {
    tenxer(g, dark);
    let d = &mut g.c18;
    d.open_prompt(Kind::Go, PromptSpec::go(), "~/d", None, &mut Tx::default());
    // Deliver the picker's answer at once, so the golden shows the list.
    d.worker.iter_mut().for_each(|(at, _)| *at = 0.0);
}

fn rename_on(g: &mut Gallery, dark: bool, name: &str, text: &str) {
    tenxer(g, dark);
    let d = &mut g.c18;
    let i = by_name(d, name);
    let _ = d
        .coll
        .selection
        .move_cursor(&d.listing, i, AnchorRule::Move);
    let id = d.listing.id_at(i);
    d.renaming = Some(id);
    let folder = d.listing.entry(i).kind == EntryKind::Folder;
    let range = (text == name).then(|| stem(name, folder));
    d.open_prompt(
        Kind::Rename,
        PromptSpec::rename(),
        text,
        range,
        &mut Tx::default(),
    );
}

fn prompt_rename(g: &mut Gallery, dark: bool) {
    rename_on(g, dark, "report.pdf", "report.pdf");
}

fn prompt_refused(g: &mut Gallery, dark: bool) {
    rename_on(g, dark, "report.pdf", "notes.md");
    let d = &mut g.c18;
    let serial = d.prompt.serial();
    d.prompt.refuse(serial, "“notes.md” already exists");
}

fn chord_on(g: &mut Gallery, dark: bool, map: &str) {
    tenxer(g, dark);
    g.c18.chord.arm(chords(map));
}

fn flash_shown(g: &mut Gallery, dark: bool) {
    tenxer(g, dark);
    g.c18.flash.show("Unknown chord", &mut Tx::default());
}

/// Golden scenes: each gets an accessibility and a PNG golden.
pub const SCENES: &[Scene] = &[
    ("c18-filter-open-light", |g| filter_open(g, false)),
    ("c18-filter-open-dark", |g| filter_open(g, true)),
    ("c18-filter-target-light", |g| filter_target(g, false)),
    ("c18-filter-target-dark", |g| filter_target(g, true)),
    ("c18-filter-picked-light", |g| filter_picked(g, false)),
    ("c18-filter-picked-dark", |g| filter_picked(g, true)),
    ("c18-filter-in-place-light", |g| filter_in_place(g, false)),
    ("c18-filter-in-place-dark", |g| filter_in_place(g, true)),
    ("c18-filter-empty-light", |g| filter_empty(g, false)),
    ("c18-filter-empty-dark", |g| filter_empty(g, true)),
    ("c18-listing-light", |g| scene(g, false)),
    ("c18-listing-dark", |g| scene(g, true)),
    ("c18-tenxer-light", |g| tenxer(g, false)),
    ("c18-tenxer-dark", |g| tenxer(g, true)),
    ("c18-prompt-find-light", |g| prompt_find(g, false)),
    ("c18-prompt-find-dark", |g| prompt_find(g, true)),
    ("c18-prompt-filter-light", |g| prompt_filter(g, false)),
    ("c18-prompt-filter-dark", |g| prompt_filter(g, true)),
    ("c18-prompt-search-light", |g| prompt_search(g, false)),
    ("c18-prompt-search-dark", |g| prompt_search(g, true)),
    ("c18-prompt-go-light", |g| prompt_go(g, false)),
    ("c18-prompt-go-dark", |g| prompt_go(g, true)),
    ("c18-prompt-rename-light", |g| prompt_rename(g, false)),
    ("c18-prompt-rename-dark", |g| prompt_rename(g, true)),
    ("c18-prompt-refused-light", |g| prompt_refused(g, false)),
    ("c18-prompt-refused-dark", |g| prompt_refused(g, true)),
    ("c18-chord-go-light", |g| chord_on(g, false, "g")),
    ("c18-chord-go-dark", |g| chord_on(g, true, "g")),
    ("c18-chord-sort-light", |g| chord_on(g, false, ",")),
    ("c18-chord-sort-dark", |g| chord_on(g, true, ",")),
];

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &[
    ("c18-chord-copy-light", |g| chord_on(g, false, "c")),
    ("c18-chord-copy-dark", |g| chord_on(g, true, "c")),
    ("c18-flash-light", |g| flash_shown(g, false)),
    ("c18-flash-dark", |g| flash_shown(g, true)),
];

// The view.

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    cx.reduced_motion(!look.animations);
    let d = &g.c18;
    let k = &look.kit;
    let mut layer = Vec::new();
    let mut toggle = Button::new("Filter")
        .variant(ButtonVariant::Ghost)
        .on_press(|g: &mut Gallery| {
            let _ = g.c18.filter.open();
        })
        .build(k)
        .key("c18-filter-toggle")
        .focusable(false);
    if d.tenxer {
        // 10xer mode hides the pane's filter button.
        toggle = rune::el();
    }
    let header = row()
        .key("c18-header")
        .w_full()
        .h(look.frame.bar)
        .px(look.space.lg)
        .gap(look.space.sm)
        .items_center()
        .child(
            text(d.title)
                .key("c18-title")
                .font(k.ty.ui_medium)
                .color(k.c.text_base)
                .grow(1.0),
        )
        .child(toggle);
    let filter_bar = if d.tenxer {
        None
    } else {
        bar(g).build(g, look)
    };
    let mut pane = col()
        .key(PANE)
        .role("group")
        .label("Documents pane")
        .focusable(true)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .on_key(pane_key)
        .child(collection(d.tenxer).build(g));
    if let Some(path) = collection(d.tenxer).cursor_path(g) {
        pane = pane.active_descendant(path);
    }
    let footer = footer(g, look, cx, &mut layer);
    let menu = field_menu().build(g, look, &mut layer);
    let outlined = d
        .filter
        .outlined(results(g))
        .map_or("none".to_owned(), |id| d.name_of(id));
    let readouts = row()
        .w_full()
        .px(look.space.lg)
        .py(look.space.xs)
        .gap(look.space.lg)
        .child(
            text(format!("Outlined: {outlined}"))
                .key("c18-outlined")
                .font(k.ty.small)
                .color(k.c.text_muted),
        )
        .child(
            text(if d.log.is_empty() {
                "\u{2014}".to_owned()
            } else {
                d.log.clone()
            })
            .key("c18-log")
            .font(k.ty.small)
            .color(k.c.text_muted),
        );
    rune_kit::root(
        col()
            .key("c18")
            .w_full()
            .h_full()
            .fill(k.c.layer_01)
            .child(header)
            .children(filter_bar)
            .child(pane)
            .child(footer)
            .child(readouts)
            .child(menu),
        layer,
    )
}

/// The footer: the prompt while one is open, else the flash, the chord pill and the `10X`
/// stand-in (C19's), with the chord's panel above the pill.
fn footer(g: &Gallery, look: &Look, cx: &mut Cx<Gallery>, layer: &mut Vec<El>) -> El {
    let d = &g.c18;
    let k = &look.kit;
    let base = row()
        .key("c18-footer")
        .w_full()
        .h(look.frame.bar)
        .px(look.space.sm)
        .gap(look.space.sm)
        .items_center()
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_base);
    if let Some(bar) = prompt().build(g, look, cx, layer) {
        return base.child(bar);
    }
    let mut f = base.child(row().grow(1.0).children(flash(look, "c18-flash", &d.flash)));
    if let Some(p) = chord::pill(look, CHORD, &d.chord) {
        f = f.child(p);
    }
    layer.extend(chord::panel(look, CHORD, &d.chord));
    if d.tenxer {
        f = f.child(
            text("10X")
                .key("c18-10x")
                .font(k.ty.small_medium)
                .color(k.c.text_accent),
        );
    }
    f
}

fn plain(m: Mods) -> bool {
    !(m.ctrl || m.alt || m.cmd || m.logo)
}

fn is_mode_key(ev: &KeyEvent) -> bool {
    matches!(ev.key, Key::Command('m')) && ev.mods.shift
}

fn is_text(ev: &KeyEvent, c: &str) -> bool {
    matches!(&ev.key, Key::Text(t) if t == c) && plain(ev.mods)
}

/// The pane's keys: an armed chord first, then the page's commands, then the collection's.
fn pane_key(g: &mut Gallery, ev: &KeyEvent, tx: &mut Tx) -> bool {
    // Ctrl+Shift+M runs ahead of an armed chord, as Strata's capture handlers do.
    if is_mode_key(ev) {
        return page_command(g, ev, tx);
    }
    if let Some(outcome) = g.c18.chord.key(ev, tx) {
        match outcome {
            ChordOutcome::Pass => return false,
            ChordOutcome::Cancelled => {}
            ChordOutcome::Completed(act) => g.c18.completed(act, tx),
            ChordOutcome::Unknown => g.c18.flash.show("Unknown chord", tx),
        }
        return true;
    }
    if page_command(g, ev, tx) {
        return true;
    }
    let d = &mut g.c18;
    if d.filter.is_open() && !d.tenxer {
        if ev.key == Key::Escape && plain(ev.mods) {
            let polled = d.filter.close();
            bar(g).route(g, polled, tx);
            return true;
        }
        if ev.key == Key::Up
            && plain(ev.mods)
            && d.coll
                .selection
                .cursor_index(&d.listing)
                .is_none_or(|i| i == 0)
        {
            d.filter.focus_field();
            return true;
        }
    }
    // 10xer's Escape ladder, in part: the search's hits, then the `f` filter.
    if d.tenxer && ev.key == Key::Escape && plain(ev.mods) {
        if !d.search.is_empty() {
            d.search.clear();
        } else if !d.listing_filter.is_empty() {
            d.listing_filter.clear();
        } else {
            return collection(true).key(g, ev, tx);
        }
        d.refresh();
        return true;
    }
    let tenxer = g.c18.tenxer;
    if collection(tenxer).key(g, ev, tx) {
        if g.c18.filter.is_open() {
            g.c18.filter.pick();
        }
        return true;
    }
    // Type to search: a printable key the listing didn't take opens the filter with it.
    let d = &mut g.c18;
    if !d.tenxer
        && let Key::Text(t) = &ev.key
        && plain(ev.mods)
        && !t.trim().is_empty()
    {
        let before = d.coll.selection.position(&d.listing);
        let polled = d.filter.open_with(t, before);
        bar(g).route(g, polled, tx);
        return true;
    }
    false
}

/// The page's commands (Strata's key map), from the pane or from nothing focused.
fn page_command(g: &mut Gallery, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let d = &mut g.c18;
    if is_mode_key(ev) {
        let on = !d.tenxer;
        d.set_tenxer(on, tx);
        return true;
    }
    match ev.key {
        Key::Function(6) => {
            d.disk_changed(true, tx);
            return true;
        }
        Key::Function(7) => {
            d.disk_changed(false, tx);
            return true;
        }
        _ => {}
    }
    if d.tenxer {
        let spec = if is_text(ev, "/") {
            Some((Kind::Find, PromptSpec::find(), String::new()))
        } else if is_text(ev, "?") {
            Some((
                Kind::FindBackward,
                PromptSpec::find_backward(),
                String::new(),
            ))
        } else if is_text(ev, "f") {
            Some((Kind::Filter, PromptSpec::filter(), d.listing_filter.clone()))
        } else if is_text(ev, "s") {
            Some((Kind::Search, PromptSpec::search(), d.search.clone()))
        } else {
            None
        };
        if let Some((kind, spec, text)) = spec {
            d.open_prompt(kind, spec, &text, None, tx);
            return true;
        }
        if is_text(ev, "r") || ev.key == Key::Function(2) {
            let Some(i) = d.coll.selection.cursor_index(&d.listing) else {
                d.flash.show("Nothing to rename", tx);
                return true;
            };
            let e = d.listing.entry(i);
            let name = e.name.to_owned();
            let range = stem(&name, e.kind == EntryKind::Folder);
            d.renaming = Some(d.listing.id_at(i));
            d.open_prompt(Kind::Rename, PromptSpec::rename(), &name, Some(range), tx);
            return true;
        }
        for map in ["g", ",", "c"] {
            if is_text(ev, map) {
                d.chord.arm(chords(map));
                return true;
            }
        }
        return false;
    }
    let ctrl_f = matches!(ev.key, Key::Command('f')) && !ev.mods.shift;
    if ctrl_f || is_text(ev, "/") {
        let polled = d.filter.open();
        bar(g).route(g, polled, tx);
        return true;
    }
    false
}

/// Keys no focused element took while this page is open: with nothing focused (a scene's
/// first frames), the pane's keys, as if it had the keyboard; from the filter field and the
/// prompt, the page's commands (Ctrl+Shift+M, F6, F7). True consumes the key.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    // C21's field menu passes on the keys it doesn't use; while it is open they are the menu's,
    // not the page's key map (no type-to-search, prompt or chord under it).
    if g.c18.fields.is_open() {
        return true;
    }
    if g.c18.filter.field_focused() || g.c18.prompt.focused() {
        return page_command(g, key, tx);
    }
    pane_key(g, key, tx)
}

/// Runs at the start of every frame while this page is open: the bar's and the prompt's
/// ticks, the flash, and the fake picker's due answers, asking for a wake when the next one
/// is due. True when state changed.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    g.c18.now = t;
    let mut changed = false;
    let polled = g.c18.filter.tick();
    changed |= polled.changed;
    bar(g).route(g, polled, tx);
    let polled = g.c18.prompt.tick();
    changed |= polled.changed;
    g.c18.route_prompt(polled, tx);
    changed |= g.c18.flash.tick(t, tx);
    // A menu whose field closed (Escape in the filter's results, a submitted prompt) closes.
    if field_menu().check(g) {
        changed = true;
    }
    let d = &mut g.c18;
    if let Some((at, text)) = d.pending_query.clone() {
        if at <= t + 1e-6 {
            d.pending_query = None;
            d.apply_query(&text, tx);
            changed = true;
        } else {
            tx.wake_after(std::time::Duration::from_secs_f64(at - t));
        }
    }
    let (due, later): (Vec<_>, Vec<_>) = d.worker.drain(..).partition(|(at, _)| *at <= t);
    d.worker = later;
    for (_, answer) in due {
        let polled = d.prompt.deliver(answer);
        changed |= polled.changed;
        d.route_prompt(polled, tx);
    }
    if let Some(next) = d.worker.iter().map(|(at, _)| *at).reduce(f64::min) {
        tx.wake_after(std::time::Duration::from_secs_f64((next - t).max(0.001)));
    }
    changed
}
