//! The gallery page for C11b SheetGrid: its scenes, demo state, view and key and poll hooks.
//! Scene names start with `c11b-` and end in `-light` or `-dark`.
//!
//! No work package doc of its own: split from C11's spreadsheet half
//! (`docs/work/phase-2-composites/C11-document-view.md`).
//!
//! A sheet in C9's drawer beside a fake listing. Each fixture is what Strata's sandboxed
//! reader would hand over: rows of text, the first the titles. The page stands in for
//! Strata's policy: a workbook's other sheets "load" from a fake reader queue on the virtual
//! clock (its first read of Inventory fails, so Try again reads it), and closing the drawer
//! ends a range drag. Two lines under the files read what the
//! scripts check: the last intent (`c11b-last-intent`) and what Ctrl+C would copy from the
//! range (`c11b-range`), each "None" while empty.

use rune::{Cx, El, Key, KeyEvent, Tx, col, el, text};
use strata_components::preview_drawer::{
    DocumentView, DrawerIntent, DrawerState, HeaderInfo, PreviewDrawer,
};
use strata_components::sheet_grid::{
    CellPos, CellRange, Sheet, SheetGrid, SheetGridState, SheetIntent, SortDir,
};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use crate::{Gallery, Scene};

/// How long the fake reader takes to read a sheet.
const READ_SECONDS: f64 = 0.3;
/// The workbook sheet whose first read fails (Inventory), and what the reader says.
const FAILS_ONCE: usize = 3;
const FAILURE: &str = "The sandboxed preview renderer failed";

/// Which fixture the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fixture {
    /// `people.csv`: names with case ties, numbers, text and empty cells.
    #[default]
    People,
    /// `data.csv`: Strata's own QA table for secondary sorts (`table-preview-keeps-secondary-sorts`).
    Ties,
    /// `budget.xlsx`: four sheets, read one at a time.
    Workbook,
    /// `notes.csv`: cells over 512 bytes and on several lines, and Strata's budget notice.
    Limits,
    /// `log.csv`: 100,000 rows of four columns.
    Large,
    /// `wide.xlsx`: 390 rows of 256 columns, the most Strata's budget lets through.
    Wide,
}

/// C11b's demo state, held in `Gallery::c11b`.
pub struct Demo {
    pub fixture: Fixture,
    pub drawer: DrawerState,
    pub grid: SheetGridState,
    /// The fake reader's queue: each sheet and the clock time it arrives.
    pub reads: Vec<(usize, f64)>,
    /// A sheet asked for at the next poll, when the clock is known.
    pub asked: Vec<usize>,
    /// The workbook's sheets, handed over as they're "read".
    pub book: Vec<Sheet>,
    /// The last intent, as the page's status line reads it ("Load 2", "Closed").
    pub last: String,
    /// What Ctrl+C would copy from the range, read from the page's poll.
    pub range: String,
    /// The sheet and range `range` was read from, so the poll reads it again only when it
    /// changes.
    pub range_at: Option<(usize, CellRange)>,
    /// Whether the reader has failed its one failing read.
    pub failed_once: bool,
}

impl Default for Demo {
    fn default() -> Self {
        Demo {
            fixture: Fixture::default(),
            drawer: DrawerState::default(),
            grid: SheetGridState::new("sheet"),
            reads: Vec::new(),
            asked: Vec::new(),
            book: Vec::new(),
            last: String::new(),
            range: String::new(),
            range_at: None,
            failed_once: false,
        }
    }
}

fn rows(lines: &[&[&str]]) -> Vec<Vec<String>> {
    lines
        .iter()
        .map(|r| r.iter().map(|c| (*c).to_owned()).collect())
        .collect()
}

pub fn people() -> Sheet {
    let mut r = rows(&[
        &["name", "group", "score", "city", "joined", "notes"],
        &["ada", "b", "92", "London", "2021-03-04", "first"],
        &["Ada", "a", "88.5", "Paris", "2020-11-30", ""],
        &["bob", "a", "", "Berlin", "2022-01-15", "late"],
        &["Cleo", "c", "-3", "", "2019-06-01", "moved"],
        &["dev", "b", "1e3", "Oslo", "2023-02-28", ""],
        &["", "c", "17", "Rome", "", "no name"],
        &["eve", "a", "n/a", "Madrid", "2021-08-09", ""],
        &["Finn", "b", "42", "Lisbon", "2020-05-05", "ragged"],
    ]);
    // A ragged row: Strata's reader keeps rows short when their file does.
    r.push(vec!["gus".into(), "c".into()]);
    for i in 0..30 {
        r.push(vec![
            format!("user{i:02}"),
            ["a", "b", "c"][i % 3].into(),
            format!("{}", (i * 37) % 101),
            ["Vienna", "Prague", "Dublin", "Athens"][i % 4].into(),
            format!("2024-{:02}-{:02}", 1 + i % 12, 1 + i % 28),
            String::new(),
        ]);
    }
    Sheet::new("people.csv", r)
}

/// Strata's QA scenario's table, verbatim.
pub fn ties() -> Sheet {
    Sheet::new(
        "data.csv",
        rows(&[
            &["name", "group"],
            &["b", "1"],
            &["a", "1"],
            &["A", "1"],
            &["a", "2"],
            &["b", "2"],
        ]),
    )
}

fn workbook() -> Vec<Sheet> {
    let months = ["Jan", "Feb", "Mar"];
    let mut sales = vec![vec![
        "region".to_owned(),
        "month".into(),
        "units".into(),
        "revenue".into(),
        "rep".into(),
    ]];
    for i in 0..36 {
        sales.push(vec![
            ["North", "South", "East", "West"][i % 4].into(),
            months[(i / 4) % 3].into(),
            format!("{}", 10 + (i * 13) % 90),
            format!("{:.2}", 120.0 + ((i * 29) % 97) as f64 * 7.5),
            ["Kim", "Lee", "Ray"][i % 3].into(),
        ]);
    }
    let mut stock = vec![(0..40).map(|j| format!("bin {j}")).collect::<Vec<_>>()];
    for i in 0..24 {
        stock.push(
            (0..40)
                .map(|j| format!("{}", (i * 7 + j * 3) % 50))
                .collect(),
        );
    }
    vec![
        Sheet::new(
            "Summary",
            rows(&[
                &["item", "total"],
                &["units", "1,912"],
                &["revenue", "31,404.50"],
            ]),
        ),
        Sheet::new("Q1 sales", sales),
        Sheet::new("Empty", Vec::new()),
        Sheet::new("Inventory", stock),
    ]
}

fn limits() -> Sheet {
    let long = "lorem ipsum dolor sit amet ".repeat(30);
    Sheet::new(
        "notes.csv",
        vec![
            vec!["id".into(), "note".into(), "status".into()],
            vec!["1".into(), long.clone(), "open".into()],
            vec![
                "2".into(),
                "first line\nsecond line\nthird".into(),
                "done".into(),
            ],
            vec!["3".into(), "é".repeat(300), "open".into()],
            vec!["4".into(), "short".into(), "".into()],
        ],
    )
    .truncated(true)
}

pub fn large() -> Sheet {
    let mut r = vec![vec![
        "seq".to_owned(),
        "level".into(),
        "message".into(),
        "ms".into(),
    ]];
    for i in 0..100_000usize {
        r.push(vec![
            format!("{i}"),
            ["info", "warn", "error", "debug"][(i * 7) % 4].into(),
            format!("event {}", (i * 7919) % 100_000),
            format!("{}", (i * 31) % 997),
        ]);
    }
    Sheet::new("log.csv", r)
}

pub fn wide() -> Sheet {
    let mut r = vec![(0..256).map(|j| format!("col {j}")).collect::<Vec<_>>()];
    for i in 0..390usize {
        r.push(
            (0..256)
                .map(|j| format!("{}", (i * 31 + j * 17) % 1000))
                .collect(),
        );
    }
    Sheet::new("wide.xlsx", r)
}

pub fn setup(g: &mut Gallery, dark: bool, fixture: Fixture) {
    g.open("c11b", dark);
    let mut drawer = DrawerState::default();
    drawer.open_at_once();
    let mut grid = SheetGridState::new("sheet");
    let mut book = Vec::new();
    match fixture {
        Fixture::Workbook => {
            book = workbook();
            grid.set_workbook(book.iter().map(|s| s.name.clone()));
            grid.set_sheet(0, book[0].clone());
        }
        Fixture::Wide => grid.set_single(wide()),
        f => {
            grid.set_single(match f {
                Fixture::Ties => ties(),
                Fixture::Limits => limits(),
                Fixture::Large => large(),
                _ => people(),
            });
            // CSV and TSV have a Source view.
            grid.set_switch(Some(DocumentView::Rendered));
        }
    }
    g.c11b = Demo {
        fixture,
        drawer,
        grid,
        book,
        ..Demo::default()
    };
}

/// The workbook on its second sheet, sorted by month then region descending.
fn workbook_sorted(g: &mut Gallery, dark: bool) {
    setup(g, dark, Fixture::Workbook);
    let d = &mut g.c11b;
    d.grid.set_sheet(1, d.book[1].clone());
    d.grid.set_active(1);
    d.grid
        .set_sort(vec![(1, SortDir::Ascending), (0, SortDir::Descending)]);
}

/// People with a 3-by-3 range, the cursor at its far corner.
fn range(g: &mut Gallery, dark: bool) {
    setup(g, dark, Fixture::People);
    let d = &mut g.c11b;
    d.grid.select(CellPos::new(1, 1), CellPos::new(3, 3));
}

/// The workbook on its empty sheet.
fn empty_sheet(g: &mut Gallery, dark: bool) {
    setup(g, dark, Fixture::Workbook);
    let d = &mut g.c11b;
    d.grid.set_sheet(2, d.book[2].clone());
    d.grid.set_active(2);
}

/// The workbook on Inventory, which the reader couldn't read.
fn failed_sheet(g: &mut Gallery, dark: bool) {
    setup(g, dark, Fixture::Workbook);
    let d = &mut g.c11b;
    d.grid.set_active(FAILS_ONCE);
    d.grid.set_sheet_failed(FAILS_ONCE, FAILURE);
    d.failed_once = true;
}

/// Golden scenes: each gets an accessibility and a PNG golden. Each calls
/// `g.open("c11b", dark)`, then fills in `g.c11b`.
pub const SCENES: &[Scene] = &[
    ("c11b-csv-light", |g| setup(g, false, Fixture::People)),
    ("c11b-csv-dark", |g| setup(g, true, Fixture::People)),
    ("c11b-workbook-light", |g| workbook_sorted(g, false)),
    ("c11b-workbook-dark", |g| workbook_sorted(g, true)),
    ("c11b-range-light", |g| range(g, false)),
    ("c11b-range-dark", |g| range(g, true)),
    ("c11b-limits-light", |g| setup(g, false, Fixture::Limits)),
    ("c11b-limits-dark", |g| setup(g, true, Fixture::Limits)),
    ("c11b-empty-sheet-light", |g| empty_sheet(g, false)),
    ("c11b-empty-sheet-dark", |g| empty_sheet(g, true)),
    ("c11b-failed-light", |g| failed_sheet(g, false)),
    ("c11b-failed-dark", |g| failed_sheet(g, true)),
];

/// Scenes for review with `--scene` only, kept out of the goldens: Strata's QA table for the
/// sort scripts, the workbook on its first sheet, and the large and wide sheets.
pub const REVIEW: &[Scene] = &[
    ("c11b-ties-light", |g| setup(g, false, Fixture::Ties)),
    ("c11b-ties-dark", |g| setup(g, true, Fixture::Ties)),
    ("c11b-book-light", |g| setup(g, false, Fixture::Workbook)),
    ("c11b-book-dark", |g| setup(g, true, Fixture::Workbook)),
    ("c11b-100k-light", |g| setup(g, false, Fixture::Large)),
    ("c11b-100k-dark", |g| setup(g, true, Fixture::Large)),
    ("c11b-wide-light", |g| setup(g, false, Fixture::Wide)),
    ("c11b-wide-dark", |g| setup(g, true, Fixture::Wide)),
];

pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let d = &g.c11b;
    let look = g.look();
    let (name, kind) = match d.fixture {
        Fixture::People => ("people.csv", "CSV document"),
        Fixture::Ties => ("data.csv", "CSV document"),
        Fixture::Workbook => ("budget.xlsx", "Excel spreadsheet"),
        Fixture::Limits => ("notes.csv", "CSV document"),
        Fixture::Large => ("log.csv", "CSV document"),
        Fixture::Wide => ("wide.xlsx", "Excel spreadsheet"),
    };
    let content = SheetGrid::new(
        |g: &Gallery| &g.c11b.grid,
        |g: &mut Gallery| &mut g.c11b.grid,
    )
    .on_intent(intent)
    .build(g, look, cx);
    PreviewDrawer::new(&d.drawer, |g: &mut Gallery| &mut g.c11b.drawer)
        .header(
            HeaderInfo::new(name)
                .icon(look.icon(IconName::FileText))
                .details(["18 KB", "Today 09:41", kind]),
        )
        .hosted(&d.grid, |g: &mut Gallery| &mut g.c11b.grid)
        .content(content)
        .on_intent(drawer_intent)
        .build(files(look, &d.last, &d.range), look, cx)
}

fn intent(g: &mut Gallery, intent: SheetIntent, _tx: &mut Tx) {
    let d = &mut g.c11b;
    if let SheetIntent::Load(i) = intent {
        d.last = format!("Load {i}");
        d.asked.push(i);
    }
}

fn drawer_intent(g: &mut Gallery, intent: DrawerIntent, _tx: &mut Tx) {
    let d = &mut g.c11b;
    match intent {
        DrawerIntent::ShowView(form) => d.last = format!("Show {form:?}"),
        DrawerIntent::Closed => {
            // Strata's preview closing: the range drag and its autoscroll end with it.
            d.grid.stop();
            d.last = "Closed".into();
        }
        _ => {}
    }
}

/// The fake listing beside the drawer, its focus group keyed `files`, and the status lines.
fn files(look: &Look, last: &str, range: &str) -> El {
    let k = &look.kit;
    let line = |key: &str, s: &str| {
        let name = if s.is_empty() { "None" } else { s };
        text(name.replace('\n', "⏎").replace('\t', "⇥"))
            .key(key.to_owned())
            .label(name.to_owned())
            .wrap()
            .w_full()
            .font(k.ty.small)
            .color(k.c.text_muted)
            .px(look.space.md)
            .py(look.space.xs)
    };
    let list = col()
        .key("files")
        .role("listbox")
        .label("Files")
        .focusable(true)
        .w_full()
        .grow(1.0)
        .padding(look.space.xs)
        .children(["people.csv", "budget.xlsx", "notes.csv"].iter().map(|n| {
            el().role("option")
                .label(*n)
                .w_full()
                .h(look.row.list)
                .px(look.space.md)
                .items_center()
                .child(text(*n).font(k.ty.ui).color(k.c.text_base))
        }));
    col()
        .w_full()
        .h_full()
        .child(list)
        .child(line("c11b-last-intent", last))
        .child(line("c11b-range", range))
}

/// Keys no focused element took while this page is open: F9 closes the preview, standing in
/// for however Strata closes it, and so ends a range drag. (Not Space, Strata's Quick Preview
/// key: Rune keeps Space from `on_key` while a pointer is held, for its Space-drag pan.) False
/// lets the gallery's Escape go back to the index.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    let plain = !(key.mods.cmd || key.mods.ctrl || key.mods.alt || key.mods.shift);
    if plain && key.key == Key::Function(9) && g.c11b.drawer.is_open() {
        let look = g.look.clone();
        let d = &mut g.c11b;
        d.drawer.close(&look, tx);
        d.grid.stop();
        d.last = "Closed".into();
        return true;
    }
    false
}

/// Runs at the start of every frame while this page is open: hands over the sheets the fake
/// reader has "read" by now, drives the range drag's autoscroll and reads the range, as
/// Strata's poll would. True when state changed.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    let d = &mut g.c11b;
    let mut changed = false;
    for i in d.asked.drain(..) {
        d.reads.push((i, t + READ_SECONDS));
    }
    let (due, later): (Vec<_>, Vec<_>) = d.reads.drain(..).partition(|&(_, at)| at <= t);
    d.reads = later;
    for (i, _) in due {
        if i == FAILS_ONCE && !d.failed_once {
            d.failed_once = true;
            d.grid.set_sheet_failed(i, FAILURE);
            changed = true;
        } else if let Some(sheet) = d.book.get(i) {
            d.grid.set_sheet(i, sheet.clone());
            changed = true;
        }
    }
    if let Some(&(_, at)) = d.reads.iter().min_by(|a, b| a.1.total_cmp(&b.1)) {
        tx.wake_after(std::time::Duration::from_secs_f64((at - t).max(0.0)));
    }
    changed |= d.grid.tick(t, &g.look, tx);
    // Read the range's text only when the range changes: a 100,000-row range is megabytes.
    // A sort or a sheet arriving clears the range, so the same range has the same text.
    let at = d.grid.range().map(|r| (d.grid.active(), r));
    if at != d.range_at {
        d.range_at = at;
        let range = d.grid.copy_range().unwrap_or_default();
        changed |= range != d.range;
        d.range = range;
    }
    changed
}
