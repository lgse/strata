// SPDX-License-Identifier: MIT

//! Behavioral tests for interactive Markdown/document tables inside [`DocumentView`]:
//! header sorting (ascending -> descending -> unsorted, multi-key, Shift-click, rich inline
//! spans & links), display-order copy, column resizing & auto-fit, per-table state reset on
//! `set_document`, horizontal overflow scrolling, and ragged/empty/find edge cases.

use std::cmp::Ordering;
use std::sync::Arc;

use rune::{Cursor, Key, KeyEvent, Mods, PhysicalInput, Runtime, clipboard};

use super::super::table::sort::SortKey;
use super::*;

fn mid(r: [f32; 4]) -> (f32, f32) {
    (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0)
}

fn table_scroll_rect(rt: &Runtime<Host>, unit: usize) -> [f32; 4] {
    let row = format!("doc-u{unit}");
    let prefix = format!("{row}-tscroll-");
    rt.elements()
        .into_iter()
        .find(|e| {
            e.path.split('/').any(|s| s == row)
                && e.path
                    .rsplit('/')
                    .next()
                    .is_some_and(|leaf| leaf.starts_with(&prefix))
        })
        .unwrap_or_else(|| panic!("no table scroll viewport for unit {unit}"))
        .rect
}

fn wide_table(tag: &str) -> Unit {
    let row_a = [
        format!("{tag} zeta alpha"),
        format!("{tag} value beta"),
        format!("{tag} value gamma"),
        format!("{tag} value delta"),
        format!("{tag} value epsilon"),
        format!("{tag} value zeta"),
        format!("{tag} value eta"),
        format!("{tag} value theta"),
    ];
    let row_b = [
        format!("{tag} alpha one"),
        format!("{tag} second two"),
        format!("{tag} third three"),
        format!("{tag} fourth four"),
        format!("{tag} fifth five"),
        format!("{tag} sixth six"),
        format!("{tag} seventh seven"),
        format!("{tag} eighth eight"),
    ];
    let refs_a: Vec<&str> = row_a.iter().map(String::as_str).collect();
    let refs_b: Vec<&str> = row_b.iter().map(String::as_str).collect();
    table(&[
        &[
            "Col1", "Col2", "Col3", "Col4", "Col5", "Col6", "Col7", "Col8",
        ],
        &refs_a,
        &refs_b,
    ])
}

fn header_click(rt: &mut Runtime<Host>, unit: usize, col: usize) {
    let r = rect(rt, &format!("doc-u{unit}/doc-u{unit}-th{col}"));
    // Click near the left-center of the header, clear of the trailing resize handle.
    click(rt, (r[0] + 12.0, r[1] + r[3] / 2.0));
    settle(rt);
}

fn shift_header_click(rt: &mut Runtime<Host>, unit: usize, col: usize) {
    let r = rect(rt, &format!("doc-u{unit}/doc-u{unit}-th{col}"));
    let at = (r[0] + 12.0, r[1] + r[3] / 2.0);
    rt.physical_input(PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    click(rt, at);
    rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
    settle(rt);
}

fn has_sort_icon(rt: &Runtime<Host>, unit: usize, col: usize) -> bool {
    let header = format!("doc-u{unit}-th{col}");
    rt.elements()
        .into_iter()
        .any(|e| e.path.split('/').any(|s| s == header) && e.path.ends_with("/sort"))
}

fn table_with_cells(rows: Vec<Vec<Cell>>) -> Unit {
    let copy: String = rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|c| c.text.as_str())
                .collect::<Vec<_>>()
                .join("\t")
                + "\n"
        })
        .collect();
    Unit::new(
        UnitKind::Table {
            list_depth: None,
            rows,
        },
        "",
    )
    .copy_text(copy)
}

#[test]
fn clicking_header_sorts_ascending_descending_and_restores_original_order() {
    let unit = table_with_cells(vec![
        vec![Cell::header("Item"), Cell::header("Score")],
        vec![
            Cell::new("Beta item").spans([Span::new(0..4, SpanStyle::Bold)]),
            Cell::new("10"),
        ],
        vec![
            Cell::new("alpha link").spans([Span::new(
                6..10,
                SpanStyle::Link("https://example.org/alpha".into()),
            )]),
            Cell::new("2"),
        ],
        vec![
            Cell::new("code_row").spans([Span::new(0..8, SpanStyle::Mono)]),
            Cell::new("-3"),
        ],
        vec![
            Cell::new("empty_score").spans([Span::new(0..5, SpanStyle::Strike)]),
            Cell::new(""),
        ],
        vec![
            Cell::new("Text score").spans([Span::new(0..4, SpanStyle::Italic)]),
            Cell::new("n/a"),
        ],
    ]);
    let mut rt = runtime(vec![unit]);

    // Initial order is the original Markdown document order (unsorted).
    assert_eq!(rt.state.doc.table_sort(0), []);
    assert_eq!(rt.state.doc.table_row_order(0), Some(vec![1, 2, 3, 4, 5]));
    assert!(!has_sort_icon(&rt, 0, 1));

    // 1st click on Score -> ascending: numbers (-3, 2, 10) < text ("n/a") < empty ("").
    header_click(&mut rt, 0, 1);
    assert_eq!(rt.state.doc.table_sort(0), [(1, SortDir::Ascending)]);
    assert_eq!(rt.state.doc.table_row_order(0), Some(vec![3, 2, 1, 5, 4]));
    assert!(has_sort_icon(&rt, 0, 1));

    // Inline link inside a sorted row stays intact and clickable.
    let link = rt
        .elements()
        .into_iter()
        .find(|e| e.role == "link")
        .expect("inline link inside sorted table cell")
        .rect;
    click(&mut rt, mid(link));
    assert_eq!(
        rt.state.intents,
        [DocumentIntent::OpenLink("https://example.org/alpha".into())]
    );

    // 2nd click on Score -> descending: empty, text, 10, 2, -3.
    header_click(&mut rt, 0, 1);
    assert_eq!(rt.state.doc.table_sort(0), [(1, SortDir::Descending)]);
    assert_eq!(rt.state.doc.table_row_order(0), Some(vec![4, 5, 1, 2, 3]));
    assert!(has_sort_icon(&rt, 0, 1));

    // 3rd click on Score -> unsorted (back to original document order).
    header_click(&mut rt, 0, 1);
    assert_eq!(rt.state.doc.table_sort(0), []);
    assert_eq!(rt.state.doc.table_row_order(0), Some(vec![1, 2, 3, 4, 5]));
    assert!(!has_sort_icon(&rt, 0, 1));
}

#[test]
fn secondary_sort_preserves_ties_on_click_and_shift_click() {
    let unit = table(&[
        &["name", "group"],
        &["b", "1"],
        &["a", "1"],
        &["A", "1"],
        &["a", "2"],
        &["b", "2"],
    ]);
    let mut rt = runtime(vec![unit]);

    // Click name asc, then group asc -> group primary asc, name asc tie-breaker.
    header_click(&mut rt, 0, 0);
    header_click(&mut rt, 0, 1);
    assert_eq!(
        rt.state.doc.table_sort(0),
        [(1, SortDir::Ascending), (0, SortDir::Ascending)]
    );
    assert_eq!(rt.state.doc.table_row_order(0), Some(vec![2, 3, 1, 4, 5]));

    // Flip group to descending -> ties still ascending by name (a, A, b) and stable.
    header_click(&mut rt, 0, 1);
    assert_eq!(
        rt.state.doc.table_sort(0),
        [(1, SortDir::Descending), (0, SortDir::Ascending)]
    );
    assert_eq!(rt.state.doc.table_row_order(0), Some(vec![4, 5, 2, 3, 1]));

    // 3rd click on primary group -> returns to original unsorted order.
    header_click(&mut rt, 0, 1);
    assert_eq!(rt.state.doc.table_sort(0), []);
    assert_eq!(rt.state.doc.table_row_order(0), Some(vec![1, 2, 3, 4, 5]));

    // Shift-click adds explicit secondary sort key and cycles it Asc -> Desc -> removed.
    header_click(&mut rt, 0, 1);
    shift_header_click(&mut rt, 0, 0);
    assert_eq!(
        rt.state.doc.table_sort(0),
        [(1, SortDir::Ascending), (0, SortDir::Ascending)]
    );
    assert_eq!(rt.state.doc.table_row_order(0), Some(vec![2, 3, 1, 4, 5]));

    shift_header_click(&mut rt, 0, 0);
    assert_eq!(
        rt.state.doc.table_sort(0),
        [(1, SortDir::Ascending), (0, SortDir::Descending)]
    );
    assert_eq!(rt.state.doc.table_row_order(0), Some(vec![1, 2, 3, 5, 4]));

    shift_header_click(&mut rt, 0, 0);
    assert_eq!(rt.state.doc.table_sort(0), [(1, SortDir::Ascending)]);
}

#[test]
fn copy_outputs_sorted_rows_for_ctrl_a_cross_block_and_cell_selection() {
    let mut rt = runtime(vec![
        Unit::new(UnitKind::Paragraph, "Intro line"),
        table(&[
            &["Name", "Val"],
            &["zeta", "30"],
            &["alpha", "5"],
            &["mid", "12"],
        ]),
        Unit::new(UnitKind::Paragraph, "Outro line"),
    ]);
    rt.poll = Some(host_poll);
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);

    // Sort by Val ascending: alpha (5), mid (12), zeta (30).
    header_click(&mut rt, 1, 1);
    let expected_asc = "Intro line\nName\tVal\nalpha\t5\nmid\t12\nzeta\t30\nOutro line\n";

    // Ctrl+A -> Ctrl+C copies the whole document with sorted table rows.
    let intro = rect(&rt, "doc-u0/t");
    click(&mut rt, (intro[0] + 2.0, intro[1] + intro[3] / 2.0));
    command(&mut rt, 'a');
    command(&mut rt, 'c');
    assert_eq!(
        clip.text().as_deref().map(String::as_str),
        Some(expected_asc)
    );

    // Cross-block drag from Intro to Outro also copies the sorted table rows.
    let intro = rect(&rt, "doc-u0/t");
    let outro = rect(&rt, "doc-u2/t");
    drag(
        &mut rt,
        (intro[0] + 1.0, intro[1] + intro[3] / 2.0),
        (outro[0] + outro[2] - 1.0, outro[1] + outro[3] / 2.0),
    );
    command(&mut rt, 'c');
    assert_eq!(
        clip.text().as_deref().map(String::as_str),
        Some(expected_asc)
    );

    // Drag across the first two sorted body rows (alpha 5, mid 12) copies displayed order.
    let first_cell = rect(&rt, "doc-u1/r0c0");
    let second_row_val = rect(&rt, "doc-u1/r1c1");
    drag(
        &mut rt,
        (first_cell[0] + 1.0, first_cell[1] + first_cell[3] / 2.0),
        (
            second_row_val[0] + second_row_val[2] - 1.0,
            second_row_val[1] + second_row_val[3] / 2.0,
        ),
    );
    command(&mut rt, 'c');
    assert_eq!(
        clip.text().as_deref().map(String::as_str),
        Some("alpha\t5\nmid\t12")
    );

    // Re-sorting clears the active selection and updates copy_all to descending order.
    header_click(&mut rt, 1, 1);
    assert_eq!(rt.selected_text(), None);
    assert_eq!(
        rt.state.doc.copy_all(),
        "Intro line\nName\tVal\nzeta\t30\nmid\t12\nalpha\t5\nOutro line\n"
    );
}

#[test]
fn dragging_resize_handle_clamps_persists_across_sort_and_double_click_autofits() {
    let unit = table(&[
        &["Name", "Kind", "Notes"],
        &["zebra", "file", "short"],
        &["apple", "dir", "also short"],
    ]);
    let mut rt = runtime(vec![unit]);

    let initial = rt
        .state
        .doc
        .table_column_widths(0)
        .expect("initial column widths");
    assert_eq!(initial.len(), 3);

    // Hovering a resize handle switches cursor to ResizeColumn.
    let handle0 = mid(rect(&rt, "doc-u0/doc-u0-th0-resize"));
    rt.pointer_move(handle0.0, handle0.1);
    settle(&mut rt);
    assert_eq!(rt.cursor(), Cursor::ResizeColumn);

    // Zero-delta tap on the divider must not freeze responsive auto-fitting.
    click(&mut rt, handle0);
    settle(&mut rt);
    assert!(
        !rt.state.doc.table_is_manually_sized(0),
        "zero-delta tap on divider keeps responsive auto-fitting"
    );
    assert_eq!(rt.state.doc.table_column_widths(0).unwrap(), initial);

    // Dragging and cancelling with Escape restores auto-fitting without writing manual widths.
    rt.pointer_move(handle0.0, handle0.1);
    rt.pointer_down(handle0.0, handle0.1);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(handle0.0 + 45.0, handle0.1);
    rt.frame(1.0 / 60.0);
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    rt.pointer_up(handle0.0 + 45.0, handle0.1);
    settle(&mut rt);
    assert!(
        !rt.state.doc.table_is_manually_sized(0),
        "Escape-cancelled drag keeps responsive auto-fitting"
    );
    assert_eq!(rt.state.doc.table_column_widths(0).unwrap(), initial);

    // Drag the last column narrower past the floor -> clamps to TABLE_MIN_WIDTH.
    let handle_last = mid(rect(&rt, "doc-u0/doc-u0-th2-resize"));
    drag(&mut rt, handle_last, (handle_last.0 - 500.0, handle_last.1));
    let clamped = rt.state.doc.table_column_widths(0).unwrap();
    assert_eq!(clamped[2], TABLE_MIN_WIDTH);
    assert_eq!(
        rt.state.doc.table_sort(0),
        [],
        "dragging last divider must not sort"
    );

    // Drag column 0 wider without triggering a sort.
    let handle0 = mid(rect(&rt, "doc-u0/doc-u0-th0-resize"));
    drag(&mut rt, handle0, (handle0.0 + 70.0, handle0.1));
    let widened = rt.state.doc.table_column_widths(0).unwrap();
    assert_eq!(widened[0], initial[0] + 70.0);
    assert_eq!(widened[2], TABLE_MIN_WIDTH);
    assert_eq!(
        rt.state.doc.table_sort(0),
        [],
        "dragging divider must not sort"
    );

    // Resized widths persist across sort toggles.
    header_click(&mut rt, 0, 0);
    assert_eq!(rt.state.doc.table_sort(0), [(0, SortDir::Ascending)]);
    assert_eq!(rt.state.doc.table_column_widths(0).unwrap(), widened);
    header_click(&mut rt, 0, 0);
    assert_eq!(rt.state.doc.table_sort(0), [(0, SortDir::Descending)]);
    assert_eq!(rt.state.doc.table_column_widths(0).unwrap(), widened);

    // Double-clicking a resized divider auto-fits that column to its content width.
    let handle0 = mid(rect(&rt, "doc-u0/doc-u0-th0-resize"));
    click(&mut rt, handle0);
    click(&mut rt, handle0);
    let fitted = rt.state.doc.table_column_widths(0).unwrap();
    assert!(
        fitted[0] < widened[0],
        "double-click auto-fit shrinks overly widened column back toward content"
    );
    assert_eq!(
        rt.state.doc.table_sort(0),
        [(0, SortDir::Descending)],
        "double-clicking divider must not toggle sort"
    );
}

#[test]
fn set_document_resets_per_table_sort_widths_and_scroll_state() {
    let mut rt = runtime(vec![wide_table("first")]);

    // Mutate sort, manual width, and horizontal scroll on Document A's wide table.
    header_click(&mut rt, 0, 0);
    let handle = mid(rect(&rt, "doc-u0/doc-u0-th0-resize"));
    drag(&mut rt, handle, (handle.0 + 80.0, handle.1));
    assert_eq!(rt.state.doc.table_sort(0), [(0, SortDir::Ascending)]);
    let manual = rt.state.doc.table_column_widths(0).unwrap();

    let scroll_at = mid(table_scroll_rect(&rt, 0));
    rt.pointer_move(scroll_at.0, scroll_at.1);
    rt.wheel_xy(scroll_at.0, scroll_at.1, -180.0, 0.0);
    settle(&mut rt);
    assert!(
        rt.state.doc.table_bar(0).expect("first bar").offset()[0] > 0.0,
        "first table scrolled horizontally before swap"
    );

    // Swap document via set_document: sort, manual widths, and horizontal scroll must reset.
    rt.update(|h, _| {
        h.doc
            .set_document(Arc::new(Document::new(vec![wide_table("second")])))
    });
    settle(&mut rt);

    assert_eq!(rt.state.doc.table_sort(0), []);
    assert_eq!(rt.state.doc.table_row_order(0), Some(vec![1, 2]));
    assert!(!rt.state.doc.table_is_manually_sized(0));
    let reset = rt.state.doc.table_column_widths(0).unwrap();
    assert_ne!(
        reset[0], manual[0],
        "manual width from previous document must not carry over"
    );
    assert_eq!(
        rt.state.doc.table_bar(0).expect("second bar").offset()[0],
        0.0,
        "horizontal scroll offset must reset after set_document"
    );

    // Scroll Document B horizontally, then replace with a fresh DocumentState using the same key
    // (mirroring RenderedDocument::new): horizontal scroll offset must still start at 0.
    let scroll_at = mid(table_scroll_rect(&rt, 0));
    rt.pointer_move(scroll_at.0, scroll_at.1);
    rt.wheel_xy(scroll_at.0, scroll_at.1, -180.0, 0.0);
    settle(&mut rt);
    assert!(
        rt.state.doc.table_bar(0).expect("second bar").offset()[0] > 0.0,
        "second table scrolled horizontally before fresh DocumentState swap"
    );

    rt.update(|h, _| {
        h.doc = DocumentState::new(Arc::new(Document::new(vec![wide_table("third")]))).key("doc");
    });
    settle(&mut rt);
    assert_eq!(
        rt.state.doc.table_bar(0).expect("third bar").offset()[0],
        0.0,
        "horizontal scroll offset must not leak across same-key DocumentState instances"
    );
}

#[test]
fn wide_table_overflows_horizontally_while_narrow_table_hides_scrollbar() {
    let narrow = table(&[&["A", "B"], &["1", "2"]]);
    let wide = wide_table("wide");
    let mut units = vec![narrow, wide];
    units.extend((0..30).map(|i| Unit::new(UnitKind::Paragraph, format!("Line {i}"))));
    let mut rt = runtime(units);

    assert!(
        !rt.state.doc.table_bar(0).expect("narrow bar").overflows(),
        "fitted narrow table does not show horizontal scrollbar"
    );
    assert!(
        rt.state.doc.table_bar(1).expect("wide bar").overflows(),
        "wide table overflows horizontally and activates C24 horizontal bar"
    );

    // Horizontal wheel over the wide table scrolls its viewport sideways.
    let wide_scroll = table_scroll_rect(&rt, 1);
    let at = mid(wide_scroll);
    rt.pointer_move(at.0, at.1);
    rt.wheel_xy(at.0, at.1, -120.0, 0.0);
    settle(&mut rt);
    let scrolled_x = rt.state.doc.table_bar(1).expect("wide bar").offset()[0];
    assert!(
        scrolled_x > 0.0,
        "wide table scrolled sideways: {scrolled_x}"
    );

    // Vertical wheel over the wide table passes through to the outer document scroll.
    rt.wheel_xy(at.0, at.1, 0.0, -120.0);
    settle(&mut rt);
    let doc_y = rt.state.doc.bar.offset()[1];
    assert!(
        doc_y > 0.0,
        "outer document still scrolls vertically over wide table: {doc_y}"
    );
}

#[test]
fn mixed_sort_keys_form_a_total_order() {
    let samples = [
        "-10", "-0", "0", "1", "10", "-inf", "inf", "NaN", "alpha", "é", "É", "", "   ",
    ];
    let keys: Vec<SortKey> = samples.iter().map(|s| SortKey::new(s)).collect();

    // Non-finite floats fall back to text; accented letters fold case-insensitively.
    assert!(matches!(SortKey::new("NaN"), SortKey::Text(_)));
    assert!(matches!(SortKey::new("inf"), SortKey::Text(_)));
    assert!(matches!(SortKey::new("-inf"), SortKey::Text(_)));
    assert_eq!(
        SortKey::new("é").compare(&SortKey::new("É")),
        Ordering::Equal
    );
    assert_eq!(
        SortKey::new("").compare(&SortKey::new("   ")),
        Ordering::Equal
    );

    // Reflexivity, antisymmetry, and transitivity across every triple.
    for (i, a) in keys.iter().enumerate() {
        assert_eq!(a.compare(a), Ordering::Equal, "reflexive: {}", samples[i]);
        for (j, b) in keys.iter().enumerate() {
            let ab = a.compare(b);
            let ba = b.compare(a);
            assert_eq!(
                ab,
                ba.reverse(),
                "antisymmetric: {} vs {}",
                samples[i],
                samples[j]
            );
            for (k, c) in keys.iter().enumerate() {
                if ab != Ordering::Greater && b.compare(c) != Ordering::Greater {
                    assert_ne!(
                        a.compare(c),
                        Ordering::Greater,
                        "transitive: {} <= {} <= {}",
                        samples[i],
                        samples[j],
                        samples[k]
                    );
                }
            }
        }
    }

    // Finite numbers sort strictly before non-finite/text, which sort strictly before empty.
    assert_eq!(
        SortKey::new("10").compare(&SortKey::new("-inf")),
        Ordering::Less
    );
    assert_eq!(
        SortKey::new("10").compare(&SortKey::new("NaN")),
        Ordering::Less
    );
    assert_eq!(SortKey::new("é").compare(&SortKey::new("")), Ordering::Less);
}

#[test]
fn empty_header_only_headerless_ragged_tables_and_active_find_never_panic() {
    let empty = table_with_cells(Vec::new());
    let zero_cols = table_with_cells(vec![Vec::new()]);
    let header_only = table(&[&["OnlyA", "OnlyB"]]);
    let headerless = table_with_cells(vec![
        vec![Cell::new("b"), Cell::new("2")],
        vec![Cell::new("a"), Cell::new("1")],
    ]);
    let ragged = table_with_cells(vec![
        vec![Cell::header("H1"), Cell::header("H2")],
        vec![Cell::new("r1"), Cell::new("20"), Cell::new("extra")],
        vec![Cell::new("r2")],
        vec![Cell::new("r3"), Cell::new("5"), Cell::new("alpha")],
    ]);
    let mut rt = runtime(vec![
        Unit::new(UnitKind::Paragraph, "First needle here."),
        empty,
        zero_cols,
        header_only,
        headerless,
        ragged,
        Unit::new(UnitKind::Paragraph, "Second needle below."),
    ]);

    // Start an active find on prose while sorting edge-case tables.
    rt.update(|h, tx| {
        h.doc.find("needle");
        h.doc.flush(tx);
    });
    settle(&mut rt);
    assert_eq!(rt.state.doc.matches().len(), 2);

    // Header-only table can toggle sort without panicking.
    header_click(&mut rt, 3, 0);
    assert_eq!(rt.state.doc.table_sort(3), [(0, SortDir::Ascending)]);
    assert_eq!(rt.state.doc.table_row_order(3), Some(vec![]));

    // Headerless table sorts all rows (including row 0) and uses fallback Column labels.
    header_click(&mut rt, 4, 1);
    assert_eq!(rt.state.doc.table_row_order(4), Some(vec![1, 0]));
    assert!(rt.state.doc.copy_all().contains("a\t1\nb\t2\n"));

    // Ragged table sorts sparse column 2 (with fallback "Column 3" header) cleanly.
    header_click(&mut rt, 5, 2);
    assert_eq!(rt.state.doc.table_sort(5), [(2, SortDir::Ascending)]);
    // "alpha" (row 3) < "extra" (row 1) < missing/empty (row 2).
    assert_eq!(rt.state.doc.table_row_order(5), Some(vec![3, 1, 2]));

    // Stepping find after table sorts still navigates prose matches cleanly.
    rt.update(|h, tx| {
        h.doc.find_step(true);
        h.doc.flush(tx);
    });
    settle(&mut rt);
    assert_eq!(rt.selected_text().as_deref(), Some("needle"));
}
