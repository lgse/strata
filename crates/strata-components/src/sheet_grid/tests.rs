//! Unit tests for the sheet grid: sorting, what a cell shows and copies, the column
//! arithmetic and widths, and on Rune's headless runtime (Inter, `Runtime::new`) the pointer,
//! keys, copy, autoscroll, tabs and what assistive technology hears.

mod perf;

use rune::{Cx, El, Env, Key, KeyEvent, Mods, PhysicalInput, Runtime, Tx, clipboard};
use strata_kit::look::Look;

use super::copy::{all_text, range_text};
use super::range::Columns;
use super::sort::{self, ColumnKeys, SortKey};
use super::widths;
use super::*;
use crate::preview_drawer::HostedPreview;

fn sheet(lines: &[&[&str]]) -> Sheet {
    Sheet::new(
        "t",
        lines
            .iter()
            .map(|r| r.iter().map(|c| (*c).to_owned()).collect())
            .collect(),
    )
}

/// The first column's text in display order.
fn firsts(st: &SheetGridState) -> Vec<String> {
    let body = &st.sheet(st.active()).unwrap().rows[1..];
    st.order()
        .iter()
        .map(|&r| body[r as usize].join(" "))
        .collect()
}

// --- Sorting --------------------------------------------------------------------------------

#[test]
fn a_cell_sorts_as_a_finite_number_else_lowercased_text_else_empty() {
    assert_eq!(SortKey::new(" 3 "), SortKey::Number(3.0));
    assert_eq!(SortKey::new("1e3"), SortKey::Number(1000.0));
    assert_eq!(SortKey::new("-0.5"), SortKey::Number(-0.5));
    assert_eq!(SortKey::new("NaN"), SortKey::Text("nan".into()));
    assert_eq!(SortKey::new("inf"), SortKey::Text("inf".into()));
    assert_eq!(SortKey::new("Émile"), SortKey::Text("émile".into()));
    assert_eq!(SortKey::new("   "), SortKey::Empty);
}

#[test]
fn ascending_puts_numbers_then_text_then_empty_cells_and_descending_reverses_it() {
    let mut st = SheetGridState::new("g");
    st.set_single(sheet(&[
        &["v"],
        &["beta"],
        &[""],
        &["10"],
        &["Alpha"],
        &["2"],
        &["alpha"],
    ]));
    // A sheet opens sorted ascending by its first column; "Alpha" and "alpha" tie, source order.
    assert_eq!(firsts(&st), ["2", "10", "Alpha", "alpha", "beta", ""]);
    st.set_sort(vec![(0, SortDir::Descending)]);
    assert_eq!(firsts(&st), ["", "beta", "Alpha", "alpha", "10", "2"]);
}

/// Strata's own QA scenario, `table-preview-keeps-secondary-sorts`, click by click.
#[test]
fn title_clicks_keep_earlier_columns_as_tie_breakers_as_stratas_scenario_does() {
    let mut st = SheetGridState::new("g");
    st.set_single(sheet(&[
        &["name", "group"],
        &["b", "1"],
        &["a", "1"],
        &["A", "1"],
        &["a", "2"],
        &["b", "2"],
    ]));
    let click = |st: &mut SheetGridState, j| {
        let spec = sort::clicked(st.sort(), j);
        st.set_sort(spec);
        firsts(st)
    };
    assert_eq!(click(&mut st, 1), ["a 1", "A 1", "b 1", "a 2", "b 2"]);
    assert_eq!(click(&mut st, 1), ["a 2", "b 2", "a 1", "A 1", "b 1"]);
    assert_eq!(click(&mut st, 0), ["a 2", "a 1", "A 1", "b 2", "b 1"]);
    assert_eq!(click(&mut st, 0), ["b 2", "b 1", "a 2", "a 1", "A 1"]);
    assert_eq!(click(&mut st, 0), ["a 2", "a 1", "A 1", "b 2", "b 1"]);
    assert_eq!(click(&mut st, 0), ["b 2", "b 1", "a 2", "a 1", "A 1"]);
    assert_eq!(
        st.sort(),
        [(0, SortDir::Descending), (1, SortDir::Descending)]
    );
}

/// A small deterministic generator, so the property runs the same every time.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

#[test]
fn flipping_the_primary_by_reversing_groups_gives_what_a_full_sort_gives() {
    for seed in 1..=20 {
        let mut g = Lcg(seed);
        let words = ["a", "A", "b", "", "3", "10", "x", "B"];
        let rows: Vec<Vec<String>> = std::iter::once(vec!["p".into(), "q".into(), "r".into()])
            .chain((0..200).map(|_| {
                let n = 1 + g.next(3) as usize;
                (0..n)
                    .map(|_| words[g.next(words.len() as u64) as usize].to_owned())
                    .collect()
            }))
            .collect();
        let data = Sheet::new("t", rows);
        let mut st = SheetGridState::new("g");
        st.set_single(data.clone());
        let mut spec = st.sort().to_vec();
        for _ in 0..12 {
            spec = sort::clicked(&spec, g.next(3) as usize);
            st.set_sort(spec.clone());
            let keys: Vec<ColumnKeys> = (0..3).map(|c| ColumnKeys::new(data.body(), c)).collect();
            let full: Vec<(&ColumnKeys, SortDir)> =
                spec.iter().map(|&(c, d)| (&keys[c], d)).collect();
            assert_eq!(
                st.order(),
                sort::order(data.body().len(), &full),
                "seed {seed}, spec {spec:?}"
            );
        }
    }
}

#[test]
fn a_sparse_column_orders_as_a_dense_one_would() {
    // Most rows stop after the first cell, so the second column is kept sparse.
    let mut lines: Vec<Vec<String>> = (0..30).map(|i| vec![format!("{i}")]).collect();
    for (i, v) in [(3, "b"), (17, "a"), (8, "10"), (25, "")] {
        lines[i].push(v.into());
    }
    let sparse = ColumnKeys::new(&lines, 1);
    assert!(matches!(sparse, ColumnKeys::Sparse(_)));
    let dense = ColumnKeys::Dense(
        lines
            .iter()
            .map(|r| r.get(1).map_or(SortKey::Empty, |t| SortKey::new(t)))
            .collect(),
    );
    for dir in [SortDir::Ascending, SortDir::Descending] {
        assert_eq!(
            sort::order(30, &[(&sparse, dir)]),
            sort::order(30, &[(&dense, dir)])
        );
    }
}

// --- What a cell shows and copies ---------------------------------------------------------

#[test]
fn columns_are_lettered_as_spreadsheets_letter_them() {
    let names: Vec<String> = [0, 25, 26, 51, 53, 255, 701, 702]
        .into_iter()
        .map(column_letters)
        .collect();
    assert_eq!(names, ["A", "Z", "AA", "AZ", "BB", "IV", "ZZ", "AAA"]);
}

#[test]
fn a_cell_shows_its_first_line_cut_at_512_bytes_on_a_character_boundary() {
    let (s, cut) = model::shown("short");
    assert_eq!((s.as_ref(), cut), ("short", false));
    let long = "x".repeat(600);
    let (s, cut) = model::shown(&long);
    assert!(cut);
    assert_eq!(s.len(), CELL_DISPLAY_BYTES + "…".len());
    // Two-byte characters: 512 bytes is 256 of them.
    let accents = "é".repeat(300);
    let (s, _) = model::shown(&accents);
    assert_eq!(s.chars().filter(|&c| c == 'é').count(), 256);
    assert_eq!(model::shown("one\r\ntwo").0, "one…");
    assert_eq!(model::shown("one\n\n").0, "one…", "an empty second line");
}

#[test]
fn a_lone_line_ending_at_the_end_is_not_another_line() {
    for t in ["one\n", "one\r\n", "one\r"] {
        let (s, cut) = model::shown(t);
        assert_eq!((s.as_ref(), cut), ("one", false), "{t:?}");
    }
}

#[test]
fn an_empty_first_row_is_no_title_row_as_in_strata() {
    let s = Sheet::new("t", vec![vec![], vec!["b".into()], vec!["a".into()]]);
    assert!(!s.header);
    assert_eq!(s.title(0), None);
    let mut st = SheetGridState::new("g");
    st.set_single(s);
    // Every row sorts, the empty one last (its cell is empty).
    assert_eq!(st.order(), [2, 1, 0]);
    assert!(Sheet::new("t", vec![vec!["id".into()]]).header);
}

#[test]
fn a_column_title_shows_stratas_128_character_cut_marked_with_an_ellipsis() {
    let title = "t".repeat(300);
    let rt = single(sheet(&[&[&title, "b"], &["1", "2"]]));
    let name = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("g-h0"))
        .and_then(|e| e.label)
        .expect("the first title");
    assert_eq!(name, format!("{}…", "t".repeat(128)));
}

#[test]
fn copying_a_range_keeps_sort_order_whole_cells_and_ragged_rows_columns() {
    let s = sheet(&[
        &["h1", "h2", "h3"],
        &["a", "b", "c"],
        &["d"],
        &["g", "h\tx", "i"],
    ]);
    let order = [2, 0, 1];
    let r = CellRange::spanning(CellPos::new(0, 1), CellPos::new(2, 2));
    assert_eq!(range_text(&s, &order, r), "h\tx\ti\nb\tc\n\t");
    assert_eq!(all_text(&s, &order), "h1\th2\th3\ng\th\tx\ti\na\tb\tc\nd\n");
    assert_eq!(
        all_text(&s.clone().header(false), &[3, 0, 1, 2])[..10],
        *"g\th\tx\ti\nh1"
    );
    // A cell shown shortened copies whole.
    let long = "y".repeat(700);
    let s = sheet(&[&["h"], &[&long]]);
    let r = CellRange::spanning(CellPos::new(0, 0), CellPos::new(0, 0));
    assert_eq!(range_text(&s, &[0], r), long);
    assert_eq!(all_text(&s, &[0]), format!("h\n{long}\n"));
}

// --- Columns and widths ---------------------------------------------------------------------

#[test]
fn a_point_falls_in_the_column_under_it_clamped_to_the_ends() {
    let c = Columns::new(40.0, &[100.0, 50.0, 80.0]);
    assert_eq!(c.width(), 270.0);
    let at: Vec<usize> = [0.0, 40.0, 139.9, 140.0, 189.0, 190.0, 269.0, 900.0]
        .into_iter()
        .map(|x| c.at(x))
        .collect();
    assert_eq!(at, [0, 0, 0, 1, 1, 2, 2, 2]);
}

#[test]
fn columns_fit_the_viewport_sharing_spare_room_by_what_each_wants_past_160() {
    let want = [100.0, 400.0, 260.0];
    let w = widths::fit(&want, 1000.0);
    assert_eq!(w.iter().sum::<f32>(), 1000.0);
    // Past their floors (100, 160, 160) the wider two share by 240:100.
    assert_eq!(w[0], 101.0);
    assert!(w[1] > w[2] && w[2] > 160.0, "{w:?}");
    // Narrower than the floors: each keeps its floor, and the rows scroll sideways.
    assert_eq!(widths::fit(&want, 300.0), [100.0, 160.0, 160.0]);
    let s = sheet(&[&["id", &"w".repeat(400)], &["1", "x"]]);
    let wanted = widths::wanted(&s, 10.0, |t, _| t.chars().count() as f32 * 8.0);
    assert_eq!(
        wanted,
        [72.0, 640.0],
        "between 72 and 640, from 128 characters at most"
    );
}

// --- On Rune's runtime ----------------------------------------------------------------------

const W: f32 = 800.0;
const H: f32 = 500.0;

struct Host {
    grid: SheetGridState,
    look: Look,
    intents: Vec<SheetIntent>,
    now: f64,
}

fn host_view(h: &Host, cx: &mut Cx<Host>) -> El {
    SheetGrid::new(|h: &Host| &h.grid, |h: &mut Host| &mut h.grid)
        .on_intent(|h: &mut Host, i, _tx: &mut Tx| h.intents.push(i))
        .build(h, &h.look, cx)
}

fn runtime(grid: SheetGridState) -> Runtime<Host> {
    let host = Host {
        grid,
        look: Look::fallback(false),
        intents: Vec::new(),
        now: 0.0,
    };
    let mut rt = Runtime::new(host, host_view, (W, H), Env::new(false, 1.0));
    rt.frame(0.0);
    settle(&mut rt);
    rt
}

fn single(s: Sheet) -> Runtime<Host> {
    let mut g = SheetGridState::new("g");
    g.set_single(s);
    runtime(g)
}

/// Frames as a host's poll runs them, the grid's autoscroll ticking on the virtual clock.
fn step(rt: &mut Runtime<Host>) -> bool {
    rt.update_if(|h, tx| {
        h.now += 1.0 / 60.0;
        let (now, look) = (h.now, h.look.clone());
        h.grid.tick(now, &look, tx)
    });
    rt.frame(1.0 / 60.0)
}

fn settle(rt: &mut Runtime<Host>) {
    for _ in 0..60 {
        if !step(rt) {
            break;
        }
    }
}

/// `people` rows: a number, a name and a group; 100 of them, the first row the titles.
fn people(n: usize) -> Sheet {
    let mut rows = vec![vec!["id".to_owned(), "name".into(), "group".into()]];
    for i in 0..n {
        rows.push(vec![
            format!("{}", n - i),
            format!("n{i:03}"),
            ["x", "y"][i % 2].into(),
        ]);
    }
    Sheet::new("people", rows)
}

fn rect(rt: &Runtime<Host>, end: &str) -> [f32; 4] {
    rt.elements()
        .into_iter()
        .find(|e| e.path.ends_with(end))
        .unwrap_or_else(|| panic!("no element ending {end}"))
        .rect
}

fn mid(r: [f32; 4]) -> (f32, f32) {
    (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
}

/// Display row `i`'s cell `j`.
fn cell(rt: &Runtime<Host>, i: usize, j: usize) -> (f32, f32) {
    mid(rect(rt, &format!("g-r{i}-c{j}")))
}

fn tap(at: (f32, f32), rt: &mut Runtime<Host>) {
    rt.pointer_move(at.0, at.1);
    rt.pointer_down(at.0, at.1);
    step(rt);
    rt.pointer_up(at.0, at.1);
    settle(rt);
}

fn drag(from: (f32, f32), to: (f32, f32), rt: &mut Runtime<Host>) {
    rt.pointer_move(from.0, from.1);
    rt.pointer_down(from.0, from.1);
    step(rt);
    rt.pointer_move(to.0, to.1);
    step(rt);
    rt.pointer_up(to.0, to.1);
    settle(rt);
}

fn key(rt: &mut Runtime<Host>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    settle(rt);
}

const CMD: Mods = Mods {
    cmd: true,
    ctrl: true,
    shift: false,
    alt: false,
    logo: false,
};

const SHIFT: Mods = Mods {
    shift: true,
    cmd: false,
    ctrl: false,
    alt: false,
    logo: false,
};

#[test]
fn a_drag_across_cells_copies_its_rectangle_as_tab_separated_rows_in_sort_order() {
    let mut rt = single(people(100));
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    // Sorted by id ascending, the last source row (id 1) comes first.
    drag(cell(&rt, 0, 1), cell(&rt, 2, 2), &mut rt);
    assert_eq!(
        rt.state.grid.range(),
        Some(CellRange::spanning(CellPos::new(0, 1), CellPos::new(2, 2)))
    );
    key(&mut rt, Key::Command('c'), CMD);
    assert_eq!(
        clip.text().as_deref().map(String::as_str),
        Some("n099\ty\nn098\tx\nn097\ty")
    );
}

#[test]
fn without_a_range_ctrl_c_copies_the_titles_and_every_row_offscreen_ones_too() {
    let mut rt = single(people(100));
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    tap(cell(&rt, 1, 1), &mut rt);
    key(&mut rt, Key::Command('c'), CMD);
    let text = clip.text().expect("copied").to_string();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 101);
    assert_eq!(lines[0], "id\tname\tgroup");
    assert_eq!(lines[1], "1\tn099\ty");
    assert_eq!(lines[100], "100\tn000\tx", "the last row, far offscreen");
}

#[test]
fn ctrl_c_with_shift_alt_or_super_copies_nothing() {
    let mut rt = single(people(10));
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    drag(cell(&rt, 0, 0), cell(&rt, 1, 1), &mut rt);
    for extra in [
        Mods { shift: true, ..CMD },
        Mods { alt: true, ..CMD },
        Mods { logo: true, ..CMD },
    ] {
        key(&mut rt, Key::Command('c'), extra);
        assert_eq!(clip.text(), None, "{extra:?}");
    }
    key(&mut rt, Key::Command('c'), CMD);
    assert!(clip.text().is_some(), "the plain chord still copies");
}

#[test]
fn shift_with_the_arrows_grows_the_range_from_its_anchor_and_a_plain_arrow_drops_it() {
    let mut rt = single(people(20));
    tap(cell(&rt, 2, 0), &mut rt);
    assert_eq!(rt.state.grid.cursor(), Some(CellPos::new(2, 0)));
    assert_eq!(
        rt.state.grid.range(),
        None,
        "a click is a cursor, not a range"
    );
    key(&mut rt, Key::Down, SHIFT);
    key(&mut rt, Key::Right, SHIFT);
    assert_eq!(
        rt.state.grid.range(),
        Some(CellRange::spanning(CellPos::new(2, 0), CellPos::new(3, 1)))
    );
    // A Shift-click grows it from the same anchor.
    rt.physical_input(PhysicalInput::Modifiers(SHIFT));
    tap(cell(&rt, 5, 2), &mut rt);
    rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
    assert_eq!(
        rt.state.grid.range(),
        Some(CellRange::spanning(CellPos::new(2, 0), CellPos::new(5, 2)))
    );
    key(&mut rt, Key::Up, Mods::default());
    assert_eq!(rt.state.grid.range(), None);
    assert_eq!(rt.state.grid.cursor(), Some(CellPos::new(4, 2)));
}

#[test]
fn a_title_click_sorts_clears_the_range_and_goes_back_to_the_top() {
    let mut rt = single(people(200));
    drag(cell(&rt, 0, 0), cell(&rt, 1, 1), &mut rt);
    rt.wheel(W / 2.0, H / 2.0, -900.0);
    settle(&mut rt);
    assert!(rt.state.grid.feed.get().offset[1] > 0.0);
    tap(mid(rect(&rt, "g-h1")), &mut rt);
    assert_eq!(
        rt.state.grid.sort(),
        [(1, SortDir::Ascending), (0, SortDir::Ascending)]
    );
    assert_eq!(rt.state.grid.range(), None);
    assert_eq!(rt.state.grid.feed.get().offset[1], 0.0);
    tap(mid(rect(&rt, "g-h1")), &mut rt);
    assert_eq!(rt.state.grid.sort()[0], (1, SortDir::Descending));
}

#[test]
fn dragging_a_divider_resizes_its_column_without_sorting_and_a_double_click_fits_it_back() {
    let mut rt = single(people(10));
    let before = rt.state.grid.column_width(2).unwrap();
    let fitted = rt.state.grid.column_width(1).unwrap();
    // The last column's divider too.
    let d = mid(rect(&rt, "g-h2-resize"));
    drag(d, (d.0 + 60.0, d.1), &mut rt);
    assert_eq!(rt.state.grid.column_width(2), Some(before + 60.0));
    assert_eq!(rt.state.grid.sort(), [(0, SortDir::Ascending)], "no sort");
    let d = mid(rect(&rt, "g-h1-resize"));
    drag(d, (d.0 - 30.0, d.1), &mut rt);
    assert_eq!(rt.state.grid.column_width(1), Some(fitted - 30.0));
    let d = mid(rect(&rt, "g-h1-resize"));
    tap(d, &mut rt);
    tap(d, &mut rt);
    assert_eq!(rt.state.grid.column_width(1), Some(fitted));
    assert_eq!(rt.state.grid.sort(), [(0, SortDir::Ascending)]);
}

#[test]
fn holding_a_range_drag_below_the_rows_scrolls_and_stop_ends_it() {
    let mut rt = single(people(400));
    let from = cell(&rt, 0, 0);
    rt.pointer_move(from.0, from.1);
    rt.pointer_down(from.0, from.1);
    step(&mut rt);
    rt.pointer_move(from.0, H - 4.0);
    for _ in 0..30 {
        step(&mut rt);
    }
    let y = rt.state.grid.feed.get().offset[1];
    assert!(y > 100.0, "it scrolled down: {y}");
    let row = rt.state.grid.cursor().unwrap().row;
    assert!(row > 15, "the range followed: {row}");
    // The host closes the preview mid-drag: nothing scrolls after.
    rt.update(|h, _| h.grid.stop());
    for _ in 0..5 {
        step(&mut rt);
    }
    let still = rt.state.grid.feed.get().offset[1];
    for _ in 0..20 {
        step(&mut rt);
    }
    assert_eq!(rt.state.grid.feed.get().offset[1], still);
    assert!(!rt.state.grid.dragging());
    rt.pointer_up(from.0, H - 4.0);
}

#[test]
fn a_press_on_empty_space_below_the_rows_clears_the_range() {
    let mut rt = single(people(4));
    drag(cell(&rt, 0, 0), cell(&rt, 2, 1), &mut rt);
    assert!(rt.state.grid.range().is_some());
    tap((W / 2.0, H - 20.0), &mut rt);
    assert_eq!(rt.state.grid.range(), None);
}

#[test]
fn the_cursor_cell_is_the_active_descendant_and_the_range_reads_selected() {
    let mut rt = single(people(30));
    drag(cell(&rt, 1, 0), cell(&rt, 2, 1), &mut rt);
    let tree = rt.accessibility_json().to_string();
    assert!(tree.contains(r#""activeDescendant":"g-r2-c1""#), "{tree}");
    let t = rt.accessibility_tree();
    let present = |a| t.nodes.iter().any(|(id, _)| *id == a);
    assert!(
        t.nodes
            .iter()
            .filter_map(|(_, n)| n.active_descendant())
            .all(present)
    );
    let selected = rt
        .elements()
        .into_iter()
        .filter(|e| e.role == "gridcell" && e.item_selected)
        .count();
    assert_eq!(selected, 4);
    // Far down by keys: the cursor's row is built and named.
    key(&mut rt, Key::End, CMD);
    let t = rt.accessibility_tree();
    assert!(
        t.nodes
            .iter()
            .filter_map(|(_, n)| n.active_descendant())
            .all(|a| t.nodes.iter().any(|(id, _)| *id == a))
    );
    assert_eq!(rt.state.grid.cursor(), Some(CellPos::new(29, 2)));
}

fn book() -> SheetGridState {
    let mut g = SheetGridState::new("g");
    g.set_workbook(["One".into(), "Two".into(), "Three".into()]);
    g.set_sheet(0, people(200));
    g
}

#[test]
fn a_tab_whose_sheet_hasnt_come_asks_once_then_shows_it_and_each_sheet_keeps_its_place() {
    let mut rt = runtime(book());
    rt.wheel(W / 2.0, H / 2.0, -600.0);
    settle(&mut rt);
    let kept = rt.state.grid.feed.get().offset[1];
    tap(mid(rect(&rt, "g-sheet1")), &mut rt);
    assert_eq!(rt.state.grid.active(), 1);
    assert!(rt.state.grid.is_loading(1));
    assert!(
        rect(&rt, "g-loading")[2] > 0.0,
        "a loading line in the grid's place"
    );
    tap(mid(rect(&rt, "g-sheet0")), &mut rt);
    tap(mid(rect(&rt, "g-sheet1")), &mut rt);
    assert_eq!(rt.state.intents, [SheetIntent::Load(1)], "asked once");
    rt.update(|h, _| h.grid.set_sheet(1, people(5)));
    settle(&mut rt);
    assert!(rect(&rt, "g-h0")[2] > 0.0);
    tap(mid(rect(&rt, "g-sheet0")), &mut rt);
    for _ in 0..30 {
        step(&mut rt);
    }
    assert_eq!(
        rt.state.grid.feed.get().offset[1],
        kept,
        "back where it was"
    );
}

#[test]
fn ctrl_page_down_shows_the_next_sheet_and_asks_for_it() {
    let mut rt = runtime(book());
    tap(cell(&rt, 0, 0), &mut rt);
    key(&mut rt, Key::PageDown, CMD);
    assert_eq!(rt.state.grid.active(), 1);
    assert_eq!(rt.state.intents, [SheetIntent::Load(1)]);
    key(&mut rt, Key::PageUp, CMD);
    assert_eq!(rt.state.grid.active(), 0);
}

/// 256 columns, the widest Strata's budget lets through.
fn wide(rows: usize) -> Sheet {
    let mut r = vec![(0..256).map(|j| format!("col {j}")).collect::<Vec<_>>()];
    for i in 0..rows {
        r.push((0..256).map(|j| format!("{i}.{j}")).collect());
    }
    Sheet::new("wide", r)
}

#[test]
fn a_wide_sheet_builds_the_columns_near_what_shows_and_follows_a_sideways_scroll() {
    let mut rt = single(wide(40));
    let titles = |rt: &Runtime<Host>| {
        rt.elements()
            .into_iter()
            .filter(|e| e.role == "columnheader" && e.key.as_deref() != Some("g-corner"))
            .count()
    };
    let few = titles(&rt);
    assert!((1..60).contains(&few), "{few} of 256 titles built");
    for _ in 0..40 {
        rt.wheel_xy(W / 2.0, H / 2.0, -400.0, 0.0);
        step(&mut rt);
    }
    settle(&mut rt);
    let x = rt.state.grid.feed.get().offset[0];
    assert!(x > 8000.0, "scrolled sideways: {x}");
    // The column at the viewport's middle is built, its title over its cells.
    let laid = rt.state.grid.laid.borrow().clone();
    let j = laid.at(x + W / 2.0);
    let title = rect(&rt, &format!("g-h{j}"));
    let cell = rect(&rt, &format!("g-r0-c{j}"));
    assert_eq!(title[0], cell[0]);
    assert!(titles(&rt) < 60);
    // Keys reach a column outside what's built: its cell is built for the active descendant.
    tap(mid(rect(&rt, &format!("g-r1-c{j}"))), &mut rt);
    key(&mut rt, Key::Home, Mods::default());
    assert_eq!(rt.state.grid.cursor(), Some(CellPos::new(1, 0)));
    assert!(
        rt.elements()
            .iter()
            .any(|e| e.key.as_deref() == Some("g-r1-c0"))
    );
    assert_eq!(
        rt.state.grid.feed.get().offset[0],
        0.0,
        "scrolled back to the first column"
    );
}

#[test]
fn switching_sheets_drops_the_range_and_keeps_the_cursor() {
    let mut rt = runtime(book());
    drag(cell(&rt, 0, 0), cell(&rt, 2, 1), &mut rt);
    assert!(rt.state.grid.range().is_some());
    tap(mid(rect(&rt, "g-sheet1")), &mut rt);
    tap(mid(rect(&rt, "g-sheet0")), &mut rt);
    assert_eq!(rt.state.grid.range(), None);
    assert_eq!(rt.state.grid.cursor(), Some(CellPos::new(2, 1)));
    // By keys too.
    drag(cell(&rt, 0, 0), cell(&rt, 2, 1), &mut rt);
    key(&mut rt, Key::PageDown, CMD);
    key(&mut rt, Key::PageUp, CMD);
    assert_eq!(rt.state.grid.range(), None);
    // And by the host.
    drag(cell(&rt, 0, 0), cell(&rt, 2, 1), &mut rt);
    rt.update(|h, _| h.grid.set_active(1));
    settle(&mut rt);
    rt.update(|h, _| h.grid.set_active(0));
    settle(&mut rt);
    assert_eq!(rt.state.grid.range(), None);
    assert_eq!(rt.state.grid.cursor(), Some(CellPos::new(2, 1)));
}

/// Rune takes Escape while a drag is held and ends the drag cancelled; the key reaches no
/// handler. Strata's capture-phase key controller clears the range on it, so this does too.
#[test]
fn escape_during_a_range_drag_drops_the_range() {
    let mut rt = single(people(20));
    let (from, to) = (cell(&rt, 1, 0), cell(&rt, 3, 1));
    rt.pointer_move(from.0, from.1);
    rt.pointer_down(from.0, from.1);
    step(&mut rt);
    rt.pointer_move(to.0, to.1);
    step(&mut rt);
    assert_eq!(
        rt.state.grid.range(),
        Some(CellRange::spanning(CellPos::new(1, 0), CellPos::new(3, 1)))
    );
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    step(&mut rt);
    let past = cell(&rt, 5, 2);
    rt.pointer_move(past.0, past.1);
    step(&mut rt);
    rt.pointer_up(past.0, past.1);
    settle(&mut rt);
    assert_eq!(rt.state.grid.range(), None);
    assert!(!rt.state.grid.dragging());
}

/// A press on the sticky row numbers, at the start and scrolled far sideways: it picks no
/// cell and starts no drag, so holding it scrolls nothing.
#[test]
fn a_press_on_a_row_number_is_on_no_cell_at_any_sideways_offset() {
    let mut rt = single(wide(40));
    let hold = |rt: &mut Runtime<Host>| {
        let at = mid(rect(rt, "g-r1-n"));
        let x = rt.state.grid.feed.get().offset[0];
        rt.pointer_move(at.0, at.1);
        rt.pointer_down(at.0, at.1);
        for _ in 0..20 {
            step(rt);
        }
        assert_eq!(rt.state.grid.cursor(), None, "at offset {x}");
        assert!(!rt.state.grid.dragging(), "at offset {x}");
        assert_eq!(rt.state.grid.feed.get().offset[0], x, "no autoscroll");
        rt.pointer_up(at.0, at.1);
        settle(rt);
    };
    hold(&mut rt);
    for _ in 0..12 {
        rt.wheel_xy(W / 2.0, H / 2.0, -400.0, 0.0);
        step(&mut rt);
    }
    settle(&mut rt);
    assert!(rt.state.grid.feed.get().offset[0] > 4000.0);
    hold(&mut rt);
}

#[test]
fn a_sheet_that_failed_says_why_is_not_busy_and_asks_again_on_retry() {
    let mut rt = runtime(book());
    tap(mid(rect(&rt, "g-sheet2")), &mut rt);
    assert_eq!(rt.state.intents, [SheetIntent::Load(2)]);
    rt.update(|h, _| {
        h.grid
            .set_sheet_failed(2, "The sandboxed preview renderer failed")
    });
    settle(&mut rt);
    assert!(!rt.state.grid.is_loading(2));
    assert_eq!(rt.state.grid.busy(), None);
    assert_eq!(
        rt.state.grid.failure(2),
        Some("The sandboxed preview renderer failed")
    );
    let t = rt.accessibility_json().to_string();
    assert!(t.contains("Preview unavailable"), "{t}");
    assert!(t.contains("The sandboxed preview renderer failed"), "{t}");
    // No rowless grid around the reason and the button.
    let root = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("g"))
        .expect("the grid's root");
    assert_eq!(root.role, "group");
    // Its tab again asks nothing; the retry button asks once more.
    tap(mid(rect(&rt, "g-sheet0")), &mut rt);
    tap(mid(rect(&rt, "g-sheet2")), &mut rt);
    assert_eq!(rt.state.intents, [SheetIntent::Load(2)]);
    tap(mid(rect(&rt, "g-retry")), &mut rt);
    assert_eq!(
        rt.state.intents,
        [SheetIntent::Load(2), SheetIntent::Load(2)]
    );
    assert!(rt.state.grid.is_loading(2));
    assert_eq!(rt.state.grid.failure(2), None);
    assert!(rect(&rt, "g-loading")[2] > 0.0);
    rt.update(|h, _| h.grid.set_sheet(2, people(3)));
    settle(&mut rt);
    assert!(rect(&rt, "g-h0")[2] > 0.0);
}
