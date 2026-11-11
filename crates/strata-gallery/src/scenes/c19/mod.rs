//! The gallery page for C19 Footer and ShortcutReference: its scenes, demo state, view and key and
//! poll hooks. Scene names start with `c19-` and end in `-light` or `-dark`.
//!
//! Work package: `docs/work/phase-2-composites/C19-footer-shortcut-reference.md`.
//!
//! The page draws a stand-in window: a row of buttons that change what the footer counts and
//! shows (scripts tap them by key), a location field, a five-entry listing, the footer, and the
//! reference over it. The page's key hook stands in for Strata's dispatch: F1 (and `~` in 10xer
//! mode) toggles the reference, Ctrl+1–3 switch the view, Ctrl+Shift+M the mode, and Delete,
//! Ctrl+V and Ctrl+, write what they would do to the log, so scripts see that nothing ran under
//! the reference. The footer prompt (`r`, or the Footer prompt button: a `rename ›` prompt), the
//! `g` chord's pill and panel and the flash are C18's; the prompt's field menu is C21's, and the
//! page's key hook stands down while it is open (C18's host rule). C16's jobs element is a
//! stand-in until C16 lands.

mod fixture;

use rune::{Chord, Cx, El, Key, KeyEvent, TextEdit, Tx, col, row, text, text_input};
use rune_kit::{Button, ButtonVariant, ControlSize};
use strata_components::collection::keyboard::RangeKind;
use strata_components::context_menu::{FieldMenu, FieldMenuState};
use strata_components::filter_bar::chord::{self, ChordMap, ChordOption, ChordOutcome, ChordState};
use strata_components::filter_bar::prompt::{FooterPrompt, PromptIntent, PromptSpec, PromptState};
use strata_components::filter_bar::{Flash, flash};
use strata_components::footer::reference::{
    ReferenceKeys, ReferenceLabels, ReferenceState, ShortcutData, ShortcutReference, chord_label,
};
use strata_components::footer::{
    Count, CountWords, FilterCounts, FilterMark, Footer, Listing, Selected, SelectionDetails,
    reference, summary,
};
use strata_kit::look::Look;

pub use fixture::{Scope, View};

use crate::{Gallery, Scene};

/// The reference's key prefix.
pub const REFERENCE: &str = "c19-reference";

/// A filter or search over the listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub search: bool,
    pub query: String,
    /// Its results are on screen (false: a typed search hasn't reported back).
    pub displayed: bool,
}

/// What the page's poll changes at a time on the virtual clock (the live-data script).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Tenxer(bool),
    View(View),
    UnbindSearch,
}

/// C19's demo state, held in `Gallery::c19`.
pub struct Demo {
    pub reference: ReferenceState,
    pub scope: Scope,
    /// Lists 5,000 rows instead of the catalog.
    pub large: bool,
    pub data: ShortcutData,
    revision: u64,
    /// Show F1 Shortcuts button off.
    pub hide_button: bool,
    /// No listing is active, so there is no count.
    pub no_listing: bool,
    pub clipboard: bool,
    pub selected: Vec<usize>,
    pub filter: Option<Filter>,
    pub hit: usize,
    pub range: Option<RangeKind>,
    /// C18's footer prompt, its `g` chord and its flash, and C21's menu over the prompt's field.
    pub prompt: PromptState<Kind>,
    pub chord: ChordState<Act>,
    pub flash: Flash,
    pub fields: FieldMenuState<Field>,
    pub jobs: bool,
    pub location: TextEdit,
    pub log: String,
    pub schedule: Vec<(f64, Change)>,
    /// The reference's compact threshold, when not the component's.
    pub compact_below: Option<f32>,
    /// The categories key is Ctrl+G.
    pub rebound: bool,
    /// Every key that reached the page's key hook while the reference was open: none should.
    pub leaks: Vec<String>,
}

impl Default for Demo {
    fn default() -> Self {
        let mut d = Self {
            reference: ReferenceState::new(),
            scope: Scope::default(),
            large: false,
            data: ShortcutData::default(),
            revision: 0,
            hide_button: false,
            no_listing: false,
            clipboard: false,
            selected: Vec::new(),
            filter: None,
            hit: 0,
            range: None,
            prompt: PromptState::new(),
            chord: ChordState::new(),
            flash: Flash::new(),
            fields: FieldMenuState::new(),
            jobs: false,
            location: TextEdit::default(),
            log: String::new(),
            schedule: Vec::new(),
            compact_below: None,
            rebound: false,
            leaks: Vec::new(),
        };
        d.refresh();
        d
    }
}

impl Demo {
    /// Rebuilds the catalog for the scope, at a new revision.
    pub fn refresh(&mut self) {
        self.revision += 1;
        self.data = if self.large {
            fixture::large(5_000, self.revision)
        } else {
            fixture::data(self.scope, self.revision)
        };
    }

    /// The keys the reference answers in this scope.
    pub fn keys(&self) -> ReferenceKeys {
        let mut keys = if self.scope.tenxer {
            ReferenceKeys::tenxer()
        } else {
            ReferenceKeys::default()
        };
        if self.rebound {
            keys.categories = vec![rune::Chord::parse("ctrl+g").expect("a chord")];
        }
        keys
    }

    /// The footer's listing.
    pub fn listing(&self) -> Listing {
        let entries = &fixture::ENTRIES;
        let selected = (!self.selected.is_empty()).then(|| {
            let chosen = self.selected.iter().map(|&i| &entries[i]);
            let mut d = SelectionDetails::default();
            for e in chosen {
                if e.folder {
                    d.folders += 1;
                } else {
                    d.files += 1;
                    if let Some(size) = e.size {
                        d.known_files += 1;
                        d.known_bytes += size;
                    }
                }
            }
            Selected {
                count: self.selected.len(),
                details: Some(d),
            }
        });
        let filter = self.filter.as_ref().map(|f| {
            let (files, folders) = if f.search {
                (fixture::HITS.len(), 0)
            } else {
                let shown = entries.iter().filter(|e| e.name.contains(f.query.as_str()));
                shown.fold((0, 0), |(files, folders), e| {
                    if e.folder {
                        (files, folders + 1)
                    } else {
                        (files + 1, folders)
                    }
                })
            };
            FilterCounts {
                files,
                folders,
                displayed: f.displayed,
            }
        });
        Listing {
            total: entries.len(),
            files: entries.iter().filter(|e| !e.folder).count(),
            folders: entries.iter().filter(|e| e.folder).count(),
            selected,
            filter,
        }
    }

    fn count(&self) -> Option<Count> {
        (!self.no_listing).then(|| summary(&self.listing(), &CountWords::default()))
    }

    fn mark(&self) -> Option<FilterMark> {
        let f = self.filter.as_ref()?;
        Some(if f.search {
            FilterMark::Search(f.query.clone())
        } else {
            FilterMark::Filter(f.query.clone())
        })
    }

    fn log(&mut self, words: &str) {
        self.log = words.to_owned();
    }

    /// Opens the `rename ›` prompt on `name`, its stem selected.
    fn open_rename(&mut self, name: &str) -> strata_components::filter_bar::prompt::Polled<Kind> {
        let stem = name.rfind('.').filter(|&i| i > 0).unwrap_or(name.len());
        self.chord.clear();
        self.prompt
            .open(Kind::Rename, PromptSpec::rename(), name, Some(0..stem))
    }

    /// What the prompt asks of the page.
    fn prompted(&mut self, intent: PromptIntent<Kind>) {
        match intent {
            PromptIntent::Submit { text, .. } => {
                self.log(&format!("Renamed to {text}"));
                let _ = self.prompt.close();
            }
            PromptIntent::Dismissed { .. } => self.log("Prompt dismissed"),
            _ => {}
        }
    }
}

/// The page's prompt kinds: only `rename ›`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Rename,
}

/// What the `g` chord's second keys do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    First,
    Home,
    Downloads,
}

/// The fields C21's text menu opens over: the prompt's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Prompt,
}

/// The `g` chord (a short copy of Strata's).
pub fn go_chord() -> ChordMap<Act> {
    let line = |cap: &str, label: &str, chord: &str, act| {
        ChordOption::new(
            cap,
            label,
            vec![(Chord::parse(chord).expect("a chord"), act)],
        )
    };
    ChordMap {
        mark: "g-".to_owned(),
        name: "Keys after g".to_owned(),
        options: vec![
            line("g", "First item", "g", Act::First),
            line("h", "Home folder", "h", Act::Home),
            line("d", "Downloads", "d", Act::Downloads),
        ],
    }
}

fn prompt() -> FooterPrompt<Gallery, Kind> {
    FooterPrompt::new(
        PROMPT,
        |g: &Gallery| &g.c19.prompt,
        |g: &mut Gallery| &mut g.c19.prompt,
    )
    .focus_return("c19-list")
    .on_intent(|g: &mut Gallery, intent, tx| match intent {
        // C21's menu takes the keyboard; the poll holds the prompt open while it does.
        PromptIntent::FieldMenu { at } => field_menu().open(g, Field::Prompt, at, tx),
        intent => g.c19.prompted(intent),
    })
}

fn field_menu() -> FieldMenu<Gallery, Field> {
    FieldMenu::new(
        "c19-field-menu",
        |g: &Gallery| &g.c19.fields,
        |g: &mut Gallery| &mut g.c19.fields,
        |g: &mut Gallery, _| g.c19.prompt.is_open().then_some(&mut g.c19.prompt.edit),
        |g: &Gallery, _| g.c19.prompt.is_open().then_some(&g.c19.prompt.edit),
    )
    .on_change(|g: &mut Gallery, _, tx| {
        let polled = g.c19.prompt.text_edited();
        prompt().route(g, polled, tx);
    })
}

/// The prompt's key prefix: its field is `c19-prompt-field`.
pub const PROMPT: &str = "c19-prompt";

/// The chord's key prefix: `c19-chord-pill`, `c19-chord-panel`.
pub const CHORD: &str = "c19-chord";

macro_rules! twin {
    ($light:literal, $dark:literal, $f:expr) => {
        [
            (
                $light,
                (|g: &mut Gallery| setup(g, false, $f)) as fn(&mut Gallery),
            ),
            (
                $dark,
                (|g: &mut Gallery| setup(g, true, $f)) as fn(&mut Gallery),
            ),
        ]
    };
}

fn setup(g: &mut Gallery, dark: bool, f: fn(&mut Demo)) {
    g.open("c19", dark);
    g.c19 = Demo::default();
    f(&mut g.c19);
    g.c19.refresh();
}

const GOLDEN: [[Scene; 2]; 10] = [
    twin!("c19-footer-light", "c19-footer-dark", |_| {}),
    twin!(
        "c19-footer-selection-light",
        "c19-footer-selection-dark",
        |d| {
            d.selected = vec![0, 2, 3];
            d.clipboard = true;
            d.jobs = true;
        }
    ),
    twin!("c19-footer-tenxer-light", "c19-footer-tenxer-dark", |d| {
        d.scope.tenxer = true;
        d.filter = Some(Filter {
            search: false,
            query: "o".to_owned(),
            displayed: true,
        });
        d.range = Some(RangeKind::Select);
        d.chord.arm(go_chord());
    }),
    twin!("c19-footer-search-light", "c19-footer-search-dark", |d| {
        d.scope.tenxer = true;
        d.filter = Some(Filter {
            search: true,
            query: "read".to_owned(),
            displayed: true,
        });
        d.clipboard = true;
    }),
    twin!("c19-footer-prompt-light", "c19-footer-prompt-dark", |d| {
        d.scope.tenxer = true;
        let _ = d.open_rename("minutes.txt");
    }),
    twin!("c19-reference-light", "c19-reference-dark", |d| {
        d.scope.arrows = true;
        d.reference.open();
    }),
    twin!(
        "c19-reference-search-light",
        "c19-reference-search-dark",
        |d| {
            d.reference.set_query("tab");
            d.reference.open();
        }
    ),
    twin!(
        "c19-reference-category-light",
        "c19-reference-category-dark",
        |d| {
            d.reference
                .set_category(Some("Files and selection".to_owned()));
            d.reference.open();
        }
    ),
    twin!(
        "c19-reference-empty-light",
        "c19-reference-empty-dark",
        |d| {
            d.reference.set_query("zzz");
            d.reference.open();
        }
    ),
    twin!(
        "c19-reference-tenxer-light",
        "c19-reference-tenxer-dark",
        |d| {
            d.scope.tenxer = true;
            d.reference.set_category(Some("Places".to_owned()));
            d.reference.open();
        }
    ),
];

const REVIEWS: [[Scene; 2]; 7] = [
    twin!("c19-reference-wide-light", "c19-reference-wide-dark", |d| {
        d.compact_below = Some(600.0);
        d.reference.open();
    }),
    twin!(
        "c19-reference-large-light",
        "c19-reference-large-dark",
        |d| {
            d.large = true;
            d.reference.open();
        }
    ),
    twin!(
        "c19-reference-german-light",
        "c19-reference-german-dark",
        |d| {
            d.scope.german = true;
            d.reference.open();
        }
    ),
    twin!("c19-reference-live-light", "c19-reference-live-dark", |d| {
        d.reference.open();
        d.schedule = vec![
            (0.5, Change::View(View::Icons)),
            (0.8, Change::UnbindSearch),
        ];
    }),
    twin!(
        "c19-reference-live-mode-light",
        "c19-reference-live-mode-dark",
        |d| {
            d.reference.open();
            d.schedule = vec![(0.5, Change::Tenxer(true))];
        }
    ),
    twin!(
        "c19-reference-rebound-light",
        "c19-reference-rebound-dark",
        |d| {
            d.rebound = true;
            d.reference.open();
        }
    ),
    twin!("c19-tenxer-light", "c19-tenxer-dark", |d| {
        d.scope.tenxer = true;
    }),
];

const fn flatten<const N: usize, const M: usize>(pairs: [[Scene; 2]; N]) -> [Scene; M] {
    let mut out: [Scene; M] = [pairs[0][0]; M];
    let mut i = 0;
    while i < N {
        out[2 * i] = pairs[i][0];
        out[2 * i + 1] = pairs[i][1];
        i += 1;
    }
    out
}

const GOLDEN_FLAT: [Scene; 20] = flatten(GOLDEN);
const REVIEW_FLAT: [Scene; 14] = flatten(REVIEWS);

/// Golden scenes: each gets an accessibility and a PNG golden.
pub const SCENES: &[Scene] = &GOLDEN_FLAT;

/// Scenes for review with `--scene` only, kept out of the goldens.
pub const REVIEW: &[Scene] = &REVIEW_FLAT;

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let look = g.look();
    let d = &g.c19;
    let k = &look.kit;
    let mut layer = Vec::new();
    let keys = d.keys();
    let labels = ReferenceLabels {
        note: reference::note(&keys, chord_label),
        ..ReferenceLabels::default()
    };
    let mut panel = ShortcutReference::new(
        REFERENCE,
        |g: &Gallery| &g.c19.reference,
        |g: &mut Gallery| &mut g.c19.reference,
        |g: &Gallery| &g.c19.data,
    )
    .keys(keys)
    .labels(labels)
    .reduced_motion(cx.is_reduced_motion());
    if let Some(w) = d.compact_below {
        panel = panel.compact_below(w);
    }
    // The prompt's candidates, the chord's panel and the field menu go onto the layer first:
    // the reference, a modal, goes last, above them all.
    let footer = footer(g, look, cx, &mut layer);
    let menu = field_menu().build(g, look, &mut layer);
    panel.build(g, look, &mut layer);
    rune_kit::root(
        col()
            .key("c19")
            .w_full()
            .h_full()
            .fill(k.c.layer_01)
            .child(controls(look))
            .child(location(g, look))
            .child(listing(g, look))
            .child(
                text(if d.log.is_empty() {
                    "\u{2014}".to_owned()
                } else {
                    d.log.clone()
                })
                .key("c19-log")
                .font(k.ty.small)
                .color(k.c.text_muted)
                .px(look.space.lg)
                .py(look.space.xs),
            )
            .children(footer)
            .child(menu),
        layer,
    )
}

/// The buttons scripts tap to change what the footer shows.
fn controls(look: &Look) -> El {
    type Act = fn(&mut Demo);
    let buttons: [(&str, &str, Act); 16] = [
        ("c19-select-two", "Select 2 files", |d| {
            d.selected = vec![2, 3]
        }),
        ("c19-select-mixed", "Select 1 folder, 2 files", |d| {
            d.selected = vec![0, 2, 4]
        }),
        ("c19-select-unknown", "Select the unknown file", |d| {
            d.selected = vec![4]
        }),
        ("c19-select-all", "Select all", |d| {
            d.selected = (0..5).collect()
        }),
        ("c19-clear", "Clear selection", |d| d.selected.clear()),
        ("c19-copy", "Copy files", |d| d.clipboard = true),
        ("c19-clipboard-clear", "Clipboard: text", |d| {
            d.clipboard = false
        }),
        ("c19-filter", "Filter “o”", |d| {
            d.filter = Some(Filter {
                search: false,
                query: "o".to_owned(),
                displayed: true,
            })
        }),
        ("c19-filter-miss", "Filter “zz”", |d| {
            d.filter = Some(Filter {
                search: false,
                query: "zz".to_owned(),
                displayed: true,
            })
        }),
        ("c19-search", "Search “read”", |d| {
            d.filter = Some(Filter {
                search: true,
                query: "read".to_owned(),
                displayed: true,
            });
            d.hit = 0;
        }),
        ("c19-pending", "Type a search", |d| {
            d.filter = Some(Filter {
                search: true,
                query: "minu".to_owned(),
                displayed: false,
            })
        }),
        ("c19-next-hit", "Next hit", |d| {
            d.hit = (d.hit + 1) % fixture::HITS.len()
        }),
        ("c19-range", "Range v", |d| {
            d.range = match d.range {
                None => Some(RangeKind::Select),
                Some(RangeKind::Select) => Some(RangeKind::Unset),
                Some(RangeKind::Unset) => None,
            }
        }),
        ("c19-hide-button", "Hide F1 button", |d| {
            d.hide_button = !d.hide_button
        }),
        ("c19-no-listing", "No listing", |d| {
            d.no_listing = !d.no_listing
        }),
        ("c19-prompt", "Footer prompt", |d| {
            let _ = d.open_rename("notes.txt");
        }),
    ];
    row()
        .key("c19-controls")
        .w_full()
        .flex_wrap()
        .gap(look.space.xs)
        .px(look.space.lg)
        .py(look.space.sm)
        .children(buttons.into_iter().map(|(key, label, act)| {
            Button::new(label)
                .variant(ButtonVariant::Outline)
                .control_size(ControlSize::Small)
                .on_press(move |g: &mut Gallery| act(&mut g.c19))
                .build(&look.kit)
                .key(key)
        }))
}

/// A text field outside the reference: the location bar's stand-in.
fn location(g: &Gallery, look: &Look) -> El {
    let k = &look.kit;
    row()
        .w_full()
        .px(look.space.lg)
        .py(look.space.xs)
        .gap(look.space.sm)
        .items_center()
        .child(text("Location").font(k.ty.small).color(k.c.text_muted))
        .child(
            row()
                .h(look.frame.control)
                .grow(1.0)
                .px(look.space.sm)
                .items_center()
                .radius(k.radius.md)
                .fill(k.c.bg_base)
                .border(1.0, k.c.border_base)
                .child(
                    text_input(&g.c19.location, k.ty.ui)
                        .single_line()
                        .key("c19-location")
                        .label("Location")
                        .color(k.c.text_base)
                        .grow(1.0)
                        .on_edit(|g: &mut Gallery| &mut g.c19.location),
                ),
        )
}

/// The five-entry listing: a focus stop with its selection drawn.
fn listing(g: &Gallery, look: &Look) -> El {
    let k = &look.kit;
    let d = &g.c19;
    col()
        .key("c19-list")
        .role("list")
        .label("Files")
        .focusable(true)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .px(look.space.lg)
        .children(fixture::ENTRIES.iter().enumerate().map(|(i, e)| {
            let selected = d.selected.contains(&i);
            let r = row()
                .key(format!("c19-entry-{i}"))
                .role("listitem")
                .w_full()
                .h(look.row.list)
                .px(look.space.sm)
                .items_center()
                .radius(k.radius.sm)
                .child(
                    text(if e.folder {
                        format!("{}/", e.name)
                    } else {
                        e.name.to_owned()
                    })
                    .font(k.ty.ui)
                    .color(k.c.text_base),
                );
            if selected {
                r.fill(look.fill.highlight)
            } else {
                r
            }
        }))
}

/// The footer, with C18's prompt, flash and chord pill (its panel goes onto `layer`), and C16's
/// stand-in.
fn footer(g: &Gallery, look: &Look, cx: &mut Cx<Gallery>, layer: &mut Vec<El>) -> Option<El> {
    let d = &g.c19;
    let k = &look.kit;
    let prompt = prompt().build(g, look, cx, layer);
    layer.extend(chord::panel(look, CHORD, &d.chord));
    let hit = d
        .filter
        .as_ref()
        .filter(|f| f.search && f.displayed)
        .map(|_| fixture::HITS[d.hit].to_owned());
    let chord = chord::pill(look, CHORD, &d.chord);
    let jobs = d.jobs.then(|| {
        Button::new("1 job")
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Small)
            .build(k)
            .key("c19-jobs")
    });
    let shortcuts = (!d.hide_button).then_some((
        (|g: &Gallery| &g.c19.reference) as fn(&Gallery) -> &ReferenceState,
        (|g: &mut Gallery| &mut g.c19.reference) as fn(&mut Gallery) -> &mut ReferenceState,
    ));
    Footer::new("c19-footer")
        .shortcuts(shortcuts)
        .count(d.count())
        .clipboard(d.clipboard)
        .tenxer(d.scope.tenxer)
        .mark(if d.scope.tenxer { d.mark() } else { None })
        .current_hit(if d.scope.tenxer { hit } else { None })
        .range(d.range)
        .flash(flash(look, "c19-flash", &d.flash))
        .chord(chord)
        .jobs(jobs)
        .prompt(prompt)
        .build(g, look)
}

fn ctrl(ev: &KeyEvent, c: char) -> bool {
    ev.key == Key::Command(c) && !ev.mods.alt && !ev.mods.logo
}

/// The page's dispatch, standing in for Strata's: keys no focused element took.
pub fn on_key(g: &mut Gallery, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let d = &mut g.c19;
    // C21's field menu passes on the keys it doesn't use: while it is open they are the menu's,
    // not the page's (C18's host rule).
    if d.fields.is_open() {
        return true;
    }
    // The reference takes every key it doesn't use: one that gets here is a leak. Logged, so a
    // script that closes the reference after sees it, and taken.
    if d.reference.is_open() {
        let leak = format!("{:?} {:?}", ev.key, ev.mods);
        d.log(&format!("Leaked {leak}"));
        d.leaks.push(leak);
        return true;
    }
    // The reference's keys run before an armed chord (Strata's `input_owner.rs`): F1 or `~`
    // after `g` opens the reference. The poll then cancels the chord, as it does whichever way
    // the reference opened.
    if d.keys().toggles(ev, tx) {
        d.reference.toggle();
        return true;
    }
    // An armed chord hears the next key first.
    if let Some(outcome) = d.chord.key(ev, tx) {
        match outcome {
            ChordOutcome::Pass => return false,
            ChordOutcome::Cancelled => {}
            ChordOutcome::Completed(act) => d.log(&format!("{act:?}")),
            ChordOutcome::Unknown => d.flash.show("Unknown chord", tx),
        }
        return true;
    }
    // The prompt's field passes on what it doesn't use: no browsing command runs from it, and
    // Rune's own fallbacks (Shift+F10 and the Menu key open the field's menu) still do.
    if d.prompt.focused() {
        return ctrl(ev, 'm') && ev.mods.shift && toggle_mode(d);
    }
    let typed = |c: &str| {
        matches!(&ev.key, Key::Text(t) if t == c)
            && !(ev.mods.cmd || ev.mods.ctrl || ev.mods.alt || ev.mods.logo)
    };
    if d.scope.tenxer && typed("g") {
        d.chord.arm(go_chord());
        return true;
    }
    if d.scope.tenxer && typed("r") {
        let polled = d.open_rename("notes.txt");
        prompt().route(g, polled, tx);
        return true;
    }
    if ctrl(ev, 'm') && ev.mods.shift {
        return toggle_mode(d);
    }
    for (c, view) in [('1', View::Columns), ('2', View::Icons), ('3', View::List)] {
        if ctrl(ev, c) && !ev.mods.shift {
            d.scope.view = view;
            d.refresh();
            d.log(&format!("Switched to {view:?}"));
            return true;
        }
    }
    if ctrl(ev, 'v') && !ev.mods.shift {
        d.log("Pasted");
        return true;
    }
    if ctrl(ev, ',') {
        d.log("Opened Settings");
        return true;
    }
    if ev.key == Key::Delete {
        d.log("Moved to Trash");
        return true;
    }
    if ev.key == Key::Escape && d.filter.is_some() {
        d.filter = None;
        return true;
    }
    false
}

fn toggle_mode(d: &mut Demo) -> bool {
    // Leaving 10xer mode closes its prompt and chord (`integration/10xer-mode`).
    d.scope.tenxer = !d.scope.tenxer;
    if !d.scope.tenxer {
        d.chord.clear();
        let _ = d.prompt.close();
    }
    d.refresh();
    true
}

/// Applies the scheduled changes as the virtual clock reaches them.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    // The prompt stays open while the reference or the field menu has the keyboard (F1 from
    // the prompt, as Strata's `abandoned` does); it closes on a blur otherwise.
    let held = g.c19.reference.is_open() || g.c19.fields.is_open();
    g.c19.prompt.hold_open(held);
    // While the reference is open no chord stays armed, however it opened (the F1 button
    // included): Strata cancels the chord on every key under the reference.
    let mut changed = g.c19.reference.is_open() && g.c19.chord.clear();
    let polled = g.c19.prompt.tick();
    changed |= polled.changed;
    prompt().route(g, polled, tx);
    changed |= g.c19.flash.tick(t, tx);
    changed |= field_menu().check(g);
    let d = &mut g.c19;
    // Wake at the next change, so it lands in a resting window too.
    if let Some(next) = d.schedule.iter().map(|(at, _)| *at).reduce(f64::min)
        && next > t
    {
        tx.wake_after(std::time::Duration::from_secs_f64(next - t));
    }
    let due: Vec<Change> = d
        .schedule
        .iter()
        .filter(|(at, _)| *at <= t)
        .map(|(_, c)| *c)
        .collect();
    if due.is_empty() {
        return changed;
    }
    d.schedule.retain(|(at, _)| *at > t);
    for change in due {
        match change {
            Change::Tenxer(on) => d.scope.tenxer = on,
            Change::View(v) => d.scope.view = v,
            Change::UnbindSearch => d.scope.search_unbound = true,
        }
    }
    d.refresh();
    true
}
