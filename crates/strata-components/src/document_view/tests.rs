//! Unit tests for the document view: the model's conversions, what a unit shows and copies,
//! find, and, on Rune's headless runtime, selection, copy, links, Copy code and code colours.

mod perf;

use std::sync::Arc;
use std::time::Duration;

use rune::{Cx, El, Env, Key, KeyEvent, Mods, PhysicalInput, Pixels, Runtime, Tx, clipboard};
use strata_kit::look::Look;
use strata_kit::theme::syntax::syntect_theme;

use super::find::{Point, match_ranges};
use super::shown::{Insert, Shown, math_inserts};
use super::*;
use crate::code_view::highlight::{CodeBlocks, Highlighter};
use crate::preview_drawer::{FindStatus, HostedPreview};

// --- The model ------------------------------------------------------------------------------

#[test]
fn char_spans_become_byte_spans() {
    let u = Unit::new(UnitKind::Paragraph, "café au lait")
        .from_char_spans([(5..7, SpanStyle::Bold), (8..99, SpanStyle::Italic)]);
    // "café " is 6 bytes: the é takes two.
    assert_eq!(u.spans[0].range, 6..8);
    assert_eq!(&u.text[u.spans[0].range.clone()], "au");
    assert_eq!(
        u.spans[1].range,
        9..u.text.len(),
        "a range past the text ends at it"
    );
}

#[test]
fn an_inline_equation_shows_its_source_and_copies_its_dollars() {
    let text = "Area \u{fffc} m.";
    let at = text.find('\u{fffc}').unwrap();
    let spans = vec![
        Span::new(0..4, SpanStyle::Bold),
        Span::new(at..at + 3, SpanStyle::Math("\\pi r^2".into())),
        Span::new(at + 3..text.len(), SpanStyle::Italic),
    ];
    let shown = Shown::with(text, &spans, math_inserts(text, &spans));
    assert_eq!(shown.text, "Area \\pi r^2 m.");
    let math = &shown.spans[1];
    assert_eq!(&shown.text[math.range.clone()], "\\pi r^2");
    assert_eq!(&shown.text[shown.spans[2].range.clone()], " m.");
    // Any part of the equation copies the whole of it, with its delimiters.
    assert_eq!(shown.copy(0..shown.text.len()), "Area $\\pi r^2$ m.");
    assert_eq!(shown.copy(7..9), "$\\pi r^2$");
    assert_eq!(shown.copy(0..4), "Area");
}

#[test]
fn the_copy_button_room_draws_but_never_copies() {
    let text = "fn main() {\n}";
    let shown = Shown::with(
        text,
        &[],
        vec![Insert {
            old: 11..11,
            new: "\u{2007}\u{2007}".into(),
            copy: String::new(),
        }],
    );
    assert_eq!(shown.text, "fn main() {\u{2007}\u{2007}\n}");
    assert_eq!(shown.copy(0..shown.text.len()), text);
    // A colour ending at the line's end stays clear of the room; the next line moves past it.
    assert_eq!(shown.map(11), 11);
    assert_eq!(shown.map(12), 12 + 6);
}

// --- Find -----------------------------------------------------------------------------------

#[test]
fn find_folds_case_and_never_overlaps() {
    assert_eq!(match_ranges("Un ÉCLAIR", "éclair"), vec![3..10]);
    assert_eq!(match_ranges("aaa", "aa").len(), 1);
    assert!(match_ranges("text", "").is_empty());
}

fn doc(units: Vec<Unit>) -> DocumentState {
    DocumentState::new(Arc::new(Document::new(units)))
}

fn table(rows: &[&[&str]]) -> Unit {
    let rows: Vec<Vec<Cell>> = rows
        .iter()
        .enumerate()
        .map(|(r, cells)| {
            cells
                .iter()
                .map(|c| {
                    if r == 0 {
                        Cell::header(*c)
                    } else {
                        Cell::new(*c)
                    }
                })
                .collect()
        })
        .collect();
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
fn find_reads_across_units_and_skips_tables() {
    let mut d = doc(vec![
        Unit::new(UnitKind::Paragraph, "the end of one"),
        table(&[&["one two"], &["one"]]),
        Unit::new(UnitKind::Paragraph, "two starts here"),
    ]);
    d.find("one\ntwo");
    assert_eq!(
        d.matches()
            .iter()
            .map(|m| (m.start, m.end))
            .collect::<Vec<_>>(),
        [(
            Point {
                unit: 0,
                offset: 11
            },
            Point { unit: 2, offset: 3 }
        )],
        "a match spans the paragraphs and not the table between"
    );
    d.find("one");
    assert_eq!(
        d.find_status().map(|s| s.total),
        Some(1),
        "table cells aren't searched"
    );
}

#[test]
fn a_table_only_document_is_not_searchable() {
    assert!(!doc(vec![table(&[&["a"], &["b"]])]).searchable());
    assert!(doc(vec![Unit::new(UnitKind::Paragraph, "a")]).searchable());
}

#[test]
fn find_steps_wrap_and_an_edit_clears_the_selection() {
    let mut d = doc(vec![Unit::new(UnitKind::Paragraph, "a b a b a")]);
    d.find("a");
    assert_eq!(
        d.find_status(),
        Some(FindStatus {
            current: Some(1),
            total: 3
        })
    );
    d.find_step(false);
    assert_eq!(
        d.find_status().and_then(|s| s.current),
        Some(3),
        "wraps back to the last"
    );
    d.find_step(true);
    assert_eq!(
        d.find_status().and_then(|s| s.current),
        Some(1),
        "wraps on to the first"
    );
    d.find("zzz");
    assert_eq!(
        d.find_status(),
        Some(FindStatus {
            current: None,
            total: 0
        })
    );
    assert_eq!(
        d.pending().last(),
        Some(&state::Request::Clear),
        "with no match, nothing stays selected"
    );
}

#[test]
fn strata_language_ids_find_a_grammar() {
    let set = two_face::syntax::extra_newlines();
    let missing: Vec<&str> = lang::STRATA_IDS
        .iter()
        .copied()
        .filter(|id| set.find_syntax_by_token(grammar_token(id)).is_none())
        .collect();
    // Vala has no grammar in two-face's set: its blocks draw plain.
    assert_eq!(missing, ["vala"], "ids with no grammar");
}

// --- On Rune's runtime ----------------------------------------------------------------------

struct Host {
    doc: DocumentState,
    look: Look,
    blocks: Option<CodeBlocks>,
    intents: Vec<DocumentIntent>,
}

fn host_view(h: &Host, cx: &mut Cx<Host>) -> El {
    let mut v = DocumentView::new(|h: &Host| &h.doc, |h: &mut Host| &mut h.doc)
        .on_intent(|h: &mut Host, i, _tx: &mut Tx| h.intents.push(i));
    if let Some(b) = &h.blocks {
        v = v.blocks(b);
    }
    v.build(h, &h.look, cx)
}

fn runtime(units: Vec<Unit>) -> Runtime<Host> {
    runtime_with(doc(units), None)
}

fn runtime_with(doc: DocumentState, blocks: Option<CodeBlocks>) -> Runtime<Host> {
    let host = Host {
        doc,
        look: Look::fallback(false),
        blocks,
        intents: Vec::new(),
    };
    let mut rt = Runtime::new(host, host_view, (700.0, 500.0), Env::new(false, 1.0));
    rt.frame(0.0);
    settle(&mut rt);
    rt
}

fn settle(rt: &mut Runtime<Host>) {
    for _ in 0..60 {
        if !rt.frame(1.0 / 60.0) {
            break;
        }
    }
}

/// Whether `path` is element `leaf` of row `row` (`doc-u3/t`): rows hold unkeyed boxes
/// between the row and its texts.
fn is(path: &str, row_leaf: &str) -> bool {
    let (row, leaf) = row_leaf.split_once('/').expect("row/leaf");
    path.split('/').any(|s| s == row) && path.ends_with(&format!("/{leaf}"))
}

fn rect(rt: &Runtime<Host>, row_leaf: &str) -> [f32; 4] {
    rt.elements()
        .into_iter()
        .find(|e| is(&e.path, row_leaf))
        .unwrap_or_else(|| panic!("no element {row_leaf}"))
        .rect
}

fn command(rt: &mut Runtime<Host>, c: char) {
    rt.key(KeyEvent {
        key: Key::Command(c),
        mods: Mods {
            cmd: true,
            ..Mods::default()
        },
    });
    settle(rt);
}

fn drag(rt: &mut Runtime<Host>, from: (f32, f32), to: (f32, f32)) {
    rt.pointer_move(from.0, from.1);
    rt.pointer_down(from.0, from.1);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(to.0, to.1);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(to.0, to.1);
    settle(rt);
}

fn image(px: u32) -> Pixels {
    let p = Pixels::new();
    p.set(px, px, vec![128; (px * px * 4) as usize]);
    p
}

fn mixed() -> Vec<Unit> {
    vec![
        Unit::new(UnitKind::Heading(1), "Title"),
        Unit::new(UnitKind::Paragraph, "Alpha beta."),
        Unit::new(
            UnitKind::Media {
                list_depth: None,
                media: Media::Image,
            },
            "pic",
        )
        .copy_text("[Image: pic]\n"),
        Unit::new(
            UnitKind::Code {
                list_depth: None,
                language: None,
            },
            "x = 1\ny = 2",
        ),
        Unit::new(UnitKind::Paragraph, "Omega end."),
    ]
}

#[test]
fn a_drag_across_blocks_copies_strata_text_with_the_image_between() {
    let mut d = doc(mixed());
    d.set_media(2, MediaState::Ready(image(40)));
    let mut rt = runtime_with(d, None);
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    let first = rect(&rt, "doc-u0/t");
    let last = rect(&rt, "doc-u4/t");
    drag(
        &mut rt,
        (first[0] + 1.0, first[1] + first[3] / 2.0),
        (last[0] + last[2] - 1.0, last[1] + last[3] / 2.0),
    );
    command(&mut rt, 'c');
    assert_eq!(
        clip.text().as_deref().map(String::as_str),
        Some("Title\nAlpha beta.\n[Image: pic]\nx = 1\ny = 2\nOmega end.\n")
    );
}

#[test]
fn a_selection_ending_inside_a_unit_copies_its_part_then_the_newline_between() {
    // Paragraphs that wrap, so the middle of a first line is inside the text.
    let one = "alpha ".repeat(60);
    let two = "omega ".repeat(60);
    let mut rt = runtime(vec![
        Unit::new(UnitKind::Paragraph, one.trim_end()),
        Unit::new(UnitKind::Paragraph, two.trim_end()),
    ]);
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    let a = rect(&rt, "doc-u0/t");
    let b = rect(&rt, "doc-u1/t");
    let line = 8.0;
    drag(
        &mut rt,
        (a[0] + a[2] * 0.5, a[1] + line),
        (b[0] + b[2] * 0.5, b[1] + line),
    );
    command(&mut rt, 'c');
    let got = clip.text().map(|t| t.to_string()).unwrap_or_default();
    let (head, tail) = got.split_once('\n').unwrap_or_else(|| panic!("{got:?}"));
    assert!(
        one.trim_end().ends_with(head),
        "the first unit's end: {head:?}"
    );
    assert!(head.len() < one.trim_end().len(), "a part, not all");
    assert!(
        two.starts_with(tail) && !tail.is_empty(),
        "the second unit's start: {tail:?}"
    );
    assert!(tail.len() < two.trim_end().len(), "a part, not all");
}

#[test]
fn ctrl_a_then_ctrl_c_copies_the_whole_document_past_runes_row_cap() {
    // Rune collects at most 10,000 unbuilt rows for a copy (R4).
    let units: Vec<Unit> = (0..12_000)
        .map(|i| Unit::new(UnitKind::Paragraph, format!("unit {i}")))
        .collect();
    let want: String = units.iter().map(|u| u.copy_text.as_str()).collect();
    let mut rt = runtime(units);
    // The host polls, as Strata's does: select-all's change comes before its next poll.
    rt.poll = Some(host_poll);
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    let t = rect(&rt, "doc-u0/t");
    rt.pointer_move(t[0] + 2.0, t[1] + 4.0);
    rt.pointer_down(t[0] + 2.0, t[1] + 4.0);
    rt.pointer_up(t[0] + 2.0, t[1] + 4.0);
    settle(&mut rt);
    command(&mut rt, 'a');
    command(&mut rt, 'c');
    let got = clip.text().map(|t| t.to_string()).unwrap_or_default();
    assert_eq!(
        got.len(),
        want.len(),
        "copied {} of {} bytes",
        got.len(),
        want.len()
    );
    assert!(got == want);
    // A click after that ends it: the next copy is the selection's again.
    rt.update(|h, _| assert!(h.doc.all_selected()));
    let t = rect(&rt, "doc-u0/t");
    drag(
        &mut rt,
        (t[0] + 1.0, t[1] + 4.0),
        (t[0] + t[2] - 1.0, t[1] + 4.0),
    );
    rt.update(|h, _| assert!(!h.doc.all_selected()));
}

#[test]
fn a_link_click_asks_the_host_to_open_it() {
    let text = "Read the guide online.";
    let at = text.find("guide").unwrap();
    let mut rt = runtime(vec![Unit::new(UnitKind::Paragraph, text).spans([
        Span::new(
            at..at + 5,
            SpanStyle::Link("https://example.org/guide".into()),
        ),
    ])]);
    let link = rt
        .elements()
        .into_iter()
        .find(|e| e.role == "link")
        .expect("the link")
        .rect;
    let (x, y) = (link[0] + link[2] / 2.0, link[1] + link[3] / 2.0);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y);
    settle(&mut rt);
    rt.update(|h, _| {
        assert_eq!(
            h.intents,
            [DocumentIntent::OpenLink("https://example.org/guide".into())]
        )
    });
}

#[test]
fn a_shift_click_extends_the_selection_into_the_next_block() {
    let mut rt = runtime(vec![
        Unit::new(UnitKind::Paragraph, "Alpha beta"),
        Unit::new(UnitKind::Paragraph, "Gamma delta"),
    ]);
    let a = rect(&rt, "doc-u0/t");
    let b = rect(&rt, "doc-u1/t");
    let at = (a[0] + 1.0, a[1] + a[3] / 2.0);
    rt.pointer_move(at.0, at.1);
    rt.pointer_down(at.0, at.1);
    rt.pointer_up(at.0, at.1);
    settle(&mut rt);
    rt.physical_input(PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    let to = (b[0] + b[2] - 1.0, b[1] + b[3] / 2.0);
    rt.pointer_move(to.0, to.1);
    rt.pointer_down(to.0, to.1);
    rt.pointer_up(to.0, to.1);
    rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
    settle(&mut rt);
    assert_eq!(
        rt.selected_text().as_deref(),
        Some("Alpha beta\nGamma delta")
    );
}

#[test]
fn find_selects_and_reveals_its_match_and_find_end_keeps_it() {
    let mut units: Vec<Unit> = (0..300)
        .map(|i| Unit::new(UnitKind::Paragraph, format!("filler {i}")))
        .collect();
    units.push(Unit::new(UnitKind::Paragraph, "The NEEDLE is here."));
    let mut rt = runtime(units);
    rt.update(|h, tx| {
        h.doc.find("needle");
        h.doc.flush(tx);
    });
    settle(&mut rt);
    assert_eq!(rt.selected_text().as_deref(), Some("NEEDLE"));
    assert!(
        rt.elements().iter().any(|e| is(&e.path, "doc-u300/t")),
        "the match's row was scrolled to"
    );
    rt.update(|h, tx| {
        h.doc.find_end();
        h.doc.flush(tx);
    });
    settle(&mut rt);
    assert_eq!(rt.selected_text().as_deref(), Some("NEEDLE"));
}

#[test]
fn copy_code_copies_the_block_then_turns_back_after_two_seconds() {
    let mut rt = runtime(vec![Unit::new(
        UnitKind::Code {
            list_depth: None,
            language: None,
        },
        "let a = 1;\nlet b = 2;",
    )]);
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    let label = |rt: &Runtime<Host>| {
        rt.elements()
            .into_iter()
            .find(|e| is(&e.path, "doc-u0/copy"))
            .and_then(|e| e.label)
    };
    assert_eq!(label(&rt).as_deref(), Some("Copy code"));
    let b = rect(&rt, "doc-u0/copy");
    let (x, y) = (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y);
    settle(&mut rt);
    assert_eq!(
        clip.text().as_deref().map(String::as_str),
        Some("let a = 1;\nlet b = 2;")
    );
    assert_eq!(label(&rt).as_deref(), Some("Code copied"));
    rt.update(|h, _| {
        // The press stamped the check with the last poll's clock (none ran here: 0).
        assert!(!h.doc.tick(1.9), "still copied before 2 s");
        assert!(h.doc.tick(2.0));
    });
    settle(&mut rt);
    assert_eq!(label(&rt).as_deref(), Some("Copy code"));
}

#[test]
fn plain_left_asks_to_leave_the_document() {
    let mut rt = runtime(vec![Unit::new(UnitKind::Paragraph, "Some text")]);
    let t = rect(&rt, "doc-u0/t");
    rt.pointer_move(t[0] + 2.0, t[1] + 4.0);
    rt.pointer_down(t[0] + 2.0, t[1] + 4.0);
    rt.pointer_up(t[0] + 2.0, t[1] + 4.0);
    settle(&mut rt);
    rt.key(KeyEvent {
        key: Key::Left,
        mods: Mods::default(),
    });
    settle(&mut rt);
    rt.update(|h, _| assert_eq!(h.intents, [DocumentIntent::Leave]));
}

#[test]
fn code_colours_are_asked_for_near_the_viewport_only() {
    // 3,000 units with a coloured block every 30: only those near the viewport are asked
    // for, and a copy of the whole document (which builds every row) asks for none.
    let units: Vec<Unit> = (0..3000)
        .map(|i| {
            if i % 30 == 0 {
                Unit::new(
                    UnitKind::Code {
                        list_depth: None,
                        language: Some("rust".into()),
                    },
                    format!("let block_{i} = {i};"),
                )
            } else {
                Unit::new(UnitKind::Paragraph, format!("paragraph {i}"))
            }
        })
        .collect();
    let look = Look::fallback(false);
    let h = Highlighter::new(Arc::new(syntect_theme(&look)));
    let blocks = h.blocks();
    let mut rt = runtime_with(doc(units), Some(blocks.clone()));
    let asked = blocks.entry_count();
    assert!(
        (1..=10).contains(&asked),
        "{asked} blocks asked for on open"
    );
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    let t = rect(&rt, "doc-u1/t");
    rt.pointer_move(t[0] + 2.0, t[1] + 4.0);
    rt.pointer_down(t[0] + 2.0, t[1] + 4.0);
    rt.pointer_up(t[0] + 2.0, t[1] + 4.0);
    settle(&mut rt);
    command(&mut rt, 'a');
    rt.update(|h, _| h.doc.select_all_off_for_test());
    command(&mut rt, 'c');
    assert!(clip.text().is_some_and(|t| t.len() > 10_000));
    assert_eq!(blocks.entry_count(), asked, "the copy asked for colours");
}

#[test]
fn a_block_in_parts_draws_plain_with_copy_code_on_its_first_part_only() {
    let part = |first, last, t: &str| {
        Unit::new(
            UnitKind::Code {
                list_depth: None,
                language: Some("rust".into()),
            },
            t,
        )
        .part(first, last, true)
    };
    let look = Look::fallback(false);
    let h = Highlighter::new(Arc::new(syntect_theme(&look)));
    let mut blocks = h.blocks();
    blocks.wait_on_miss(Some(Duration::from_secs(30)));
    let rt = runtime_with(
        doc(vec![
            part(true, false, "let a = 1;"),
            part(false, true, "let b = 2;"),
        ]),
        Some(blocks.clone()),
    );
    assert_eq!(blocks.entry_count(), 0, "a split block isn't coloured");
    let copies: Vec<String> = rt
        .elements()
        .into_iter()
        .filter(|e| e.path.ends_with("/copy"))
        .map(|e| e.path)
        .collect();
    assert_eq!(copies.len(), 1, "{copies:?}");
    assert!(is(&copies[0], "doc-u0/copy"));
}

#[test]
fn find_maps_folded_matches_back_through_characters_that_change_length() {
    // "İ" lowercases to two characters (i and a combining dot), "ẞ" to "ß": a match after,
    // across or inside them still lands on the original text's characters.
    let text = "aİb ẞtraße x";
    let r = match_ranges(text, "b");
    assert_eq!(
        r.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(),
        ["b"]
    );
    let r = match_ranges(text, "ßtraße");
    assert_eq!(
        r.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(),
        ["ẞtraße"]
    );
    let r = match_ranges(text, "x");
    assert_eq!(
        r.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(),
        ["x"]
    );
    // A query matching only the first half of İ's lowercase takes the whole character.
    let r = match_ranges(text, "ai");
    assert_eq!(
        r.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(),
        ["aİ"]
    );
}

/// The host's poll, as Strata's: the document's clock and its queued requests.
fn host_poll(h: &mut Host, t: f64, tx: &mut Tx) -> bool {
    let a = h.doc.tick(t);
    let b = h.doc.flush(tx);
    a | b
}

/// Runs the window as a shell that sleeps between wakes: only to each wake deadline, and no
/// further once there is none.
fn rest(rt: &mut Runtime<Host>) {
    for _ in 0..10 {
        let Some(due) = rt.wake_deadline() else {
            return;
        };
        rt.frame((due - rt.clock).max(1.0 / 60.0));
        settle(rt);
    }
}

#[test]
fn copy_code_turns_back_on_a_window_at_rest() {
    // Review 1, B1: the 2 s ran from the first poll after the press, and the press's only
    // wake came 2 s after the press, before it: a resting window kept "Code copied".
    let mut rt = runtime(vec![Unit::new(
        UnitKind::Code {
            list_depth: None,
            language: None,
        },
        "let a = 1;",
    )]);
    rt.poll = Some(host_poll);
    settle(&mut rt);
    let label = |rt: &Runtime<Host>| {
        rt.elements()
            .into_iter()
            .find(|e| is(&e.path, "doc-u0/copy"))
            .and_then(|e| e.label)
    };
    let b = rect(&rt, "doc-u0/copy");
    let (x, y) = (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.frame(1.0 / 60.0);
    rt.pointer_up(x, y);
    settle(&mut rt);
    assert_eq!(label(&rt).as_deref(), Some("Code copied"));
    rest(&mut rt);
    assert_eq!(label(&rt).as_deref(), Some("Copy code"));
}

fn link_runtime() -> (Runtime<Host>, (f32, f32)) {
    let text = "Read the guide online.";
    let at = text.find("guide").unwrap();
    let mut rt = runtime(vec![
        Unit::new(UnitKind::Paragraph, "Before the link."),
        Unit::new(UnitKind::Paragraph, text).spans([Span::new(
            at..at + 5,
            SpanStyle::Link("https://example.org/guide".into()),
        )]),
    ]);
    rt.poll = Some(host_poll);
    settle(&mut rt);
    let link = rt
        .elements()
        .into_iter()
        .find(|e| e.role == "link")
        .expect("the link")
        .rect;
    (rt, (link[0] + link[2] / 2.0, link[1] + link[3] / 2.0))
}

fn click(rt: &mut Runtime<Host>, (x, y): (f32, f32)) {
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(1.0 / 60.0);
}

#[test]
fn a_shift_click_on_a_link_does_not_open_it() {
    // Strata opens a link on a plain single click only (`presses == 1 && !extend`).
    let (mut rt, at) = link_runtime();
    let before = rect(&rt, "doc-u0/t");
    click(&mut rt, (before[0] + 1.0, before[1] + before[3] / 2.0));
    settle(&mut rt);
    rt.frame(1.0);
    rt.physical_input(PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    click(&mut rt, at);
    rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
    settle(&mut rt);
    rt.update(|h, _| assert_eq!(h.intents, [], "a Shift-click opened the link"));
}

#[test]
fn a_double_click_on_a_link_opens_it_once() {
    let (mut rt, at) = link_runtime();
    click(&mut rt, at);
    click(&mut rt, at);
    settle(&mut rt);
    rt.update(|h, _| {
        assert_eq!(
            h.intents,
            [DocumentIntent::OpenLink("https://example.org/guide".into())]
        )
    });
    // Past the double-click time, a click opens it again.
    rt.frame(1.0);
    settle(&mut rt);
    click(&mut rt, at);
    settle(&mut rt);
    rt.update(|h, _| assert_eq!(h.intents.len(), 2));
}

#[test]
fn a_find_queued_before_a_document_swap_selects_nothing_in_the_new_one() {
    // Review 1, should-fix 3: the queued selection kept the old match's unit and bytes, and
    // selected whatever the new document had there.
    let mut rt = runtime(vec![
        Unit::new(UnitKind::Paragraph, "zz"),
        Unit::new(UnitKind::Paragraph, "the second line"),
    ]);
    rt.update(|h, _| h.doc.find("line"));
    rt.update(|h, tx| {
        h.doc.set_document(Arc::new(Document::new(vec![
            Unit::new(UnitKind::Paragraph, "zz"),
            Unit::new(UnitKind::Paragraph, "ÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉÉ"),
        ])));
        h.doc.flush(tx);
    });
    settle(&mut rt);
    assert_eq!(rt.selected_text(), None);
    rt.update(|h, _| {
        assert_eq!(
            h.doc.find_status(),
            Some(FindStatus {
                current: None,
                total: 0
            })
        )
    });
}

#[test]
fn ctrl_a_over_a_full_selection_does_not_make_a_later_shift_click_copy_everything() {
    // Review 1: with everything already selected, Ctrl+A changes no selection, so its pending
    // flag waited for the next change, and a Shift-click that narrowed the selection then
    // copied the whole document.
    let mut rt = runtime(vec![
        Unit::new(UnitKind::Paragraph, "Alpha beta"),
        Unit::new(UnitKind::Paragraph, "Gamma delta"),
    ]);
    rt.poll = Some(host_poll);
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    let a = rect(&rt, "doc-u0/t");
    let b = rect(&rt, "doc-u1/t");
    drag(
        &mut rt,
        (a[0] + 0.5, a[1] + a[3] / 2.0),
        (b[0] + b[2] - 1.0, b[1] + b[3] / 2.0),
    );
    assert_eq!(
        rt.selected_text().as_deref(),
        Some("Alpha beta\nGamma delta")
    );
    command(&mut rt, 'a');
    rt.frame(1.0);
    settle(&mut rt);
    rt.physical_input(PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    click(&mut rt, (b[0] + 20.0, b[1] + b[3] / 2.0));
    rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
    settle(&mut rt);
    let narrowed = rt.selected_text().expect("a selection");
    assert!(
        narrowed.len() < "Alpha beta\nGamma delta".len(),
        "{narrowed:?}"
    );
    command(&mut rt, 'c');
    assert_eq!(clip.text().as_deref(), Some(&narrowed));
}

#[test]
fn copy_source_copies_a_display_equation_source() {
    let mut d = doc(vec![Unit::new(
        UnitKind::Media {
            list_depth: None,
            media: Media::Equation {
                source: "e^{i\\pi} + 1 = 0".into(),
                display: true,
            },
        },
        "",
    )]);
    d.set_media(0, MediaState::Ready(image(40)));
    let mut rt = runtime_with(d, None);
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    let b = rect(&rt, "doc-u0/copy");
    click(&mut rt, (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0));
    settle(&mut rt);
    assert_eq!(
        clip.text().as_deref().map(String::as_str),
        Some("e^{i\\pi} + 1 = 0")
    );
}

#[test]
fn find_skips_text_in_tables_images_and_rules() {
    // D8: these units carry text (a table's, should Strata's converter give it one, an
    // image's alt, a rule's line), and find still reads only the text blocks.
    let mut d = doc(vec![
        Unit::new(UnitKind::Paragraph, "a needle in prose"),
        table(&[&["needle"], &["needle"]]).copy_text("needle\nneedle\n"),
        Unit::new(
            UnitKind::Table {
                list_depth: None,
                rows: Vec::new(),
            },
            "needle",
        ),
        Unit::new(
            UnitKind::Media {
                list_depth: None,
                media: Media::Image,
            },
            "needle alt",
        ),
        Unit::new(UnitKind::Rule { list_depth: None }, "needle ──"),
    ]);
    d.find("needle");
    assert_eq!(d.find_status().map(|s| s.total), Some(1));
    assert!(d.matches().iter().all(|m| m.start.unit == 0));
}

#[test]
fn a_coloured_block_coming_into_reach_asks_for_a_build() {
    let units: Vec<Unit> = (0..200)
        .map(|i| {
            if i == 100 {
                Unit::new(
                    UnitKind::Code {
                        list_depth: None,
                        language: Some("rust".into()),
                    },
                    "let x = 1;",
                )
            } else {
                Unit::new(UnitKind::Paragraph, format!("p {i}"))
            }
        })
        .collect();
    let mut d = doc(units);
    assert!(!d.set_visible(0..10), "nothing that colours is near");
    // Reach is twice the visible rows (here 20) either side.
    assert!(d.set_visible(80..90), "the block at 100 came within reach");
    assert!(!d.set_visible(81..91), "still within reach: nothing new");
    assert!(d.set_visible(150..160), "it left reach");
}

/// Frames as a shell does: one, then more only while something animates or input is
/// queued. The number of frames run.
fn shell(rt: &mut Runtime<Host>) -> u32 {
    let mut n = 0;
    loop {
        let moving = rt.frame(1.0 / 60.0);
        n += 1;
        if !(moving || rt.has_pending_input()) || n > 200 {
            return n;
        }
    }
}

#[test]
fn ctrl_a_over_a_full_selection_then_a_shift_click_on_a_resting_window_copies_the_click() {
    // Review 2, SF-A: Ctrl+A over a selection that already held everything changed nothing
    // and queued no frame, and the modifier presses frame nothing either, so the Shift-click's
    // own change came while the poll countdown still waited for select-all's: it turned
    // copy-everything on, and Ctrl+C copied the whole document.
    let mut rt = runtime(vec![
        Unit::new(UnitKind::Paragraph, "Alpha beta"),
        Unit::new(UnitKind::Paragraph, "Gamma delta"),
    ]);
    rt.poll = Some(host_poll);
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    let a = rect(&rt, "doc-u0/t");
    let b = rect(&rt, "doc-u1/t");
    drag(
        &mut rt,
        (a[0] + 0.5, a[1] + a[3] / 2.0),
        (b[0] + b[2] - 1.0, b[1] + b[3] / 2.0),
    );
    let at = (b[0] + 20.0, b[1] + b[3] / 2.0);
    rt.pointer_move(at.0, at.1);
    shell(&mut rt);
    rt.frame(1.0);
    shell(&mut rt);
    let command = KeyEvent {
        key: Key::Command('a'),
        mods: Mods {
            cmd: true,
            ..Mods::default()
        },
    };
    rt.key(command);
    shell(&mut rt);
    rt.physical_input(PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    rt.pointer_down(at.0, at.1);
    rt.pointer_up(at.0, at.1);
    rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
    shell(&mut rt);
    let narrowed = rt.selected_text().expect("a selection");
    assert!(
        narrowed.len() < "Alpha beta\nGamma delta".len(),
        "{narrowed:?}"
    );
    rt.key(KeyEvent {
        key: Key::Command('c'),
        mods: Mods {
            cmd: true,
            ..Mods::default()
        },
    });
    shell(&mut rt);
    assert_eq!(clip.text().as_deref(), Some(&narrowed));
}

#[test]
fn past_runes_cap_a_shift_click_after_ctrl_a_copies_the_click_not_the_document() {
    // SF-A where Rune's report is cut at its 10,000-row cap: a second Ctrl+A changes nothing,
    // and the Shift-click that narrows the selection after it must still end copy-everything.
    let units: Vec<Unit> = (0..12_000)
        .map(|i| Unit::new(UnitKind::Paragraph, format!("unit {i}")))
        .collect();
    let mut rt = runtime(units);
    rt.poll = Some(host_poll);
    let clip = clipboard::memory();
    rt.use_clipboard(&clip);
    let t = rect(&rt, "doc-u0/t");
    click(&mut rt, (t[0] + 2.0, t[1] + 4.0));
    shell(&mut rt);
    let ctrl_a = KeyEvent {
        key: Key::Command('a'),
        mods: Mods {
            cmd: true,
            ..Mods::default()
        },
    };
    rt.key(ctrl_a.clone());
    shell(&mut rt);
    rt.update(|h, _| assert!(h.doc.all_selected(), "Ctrl+A selects the document"));
    rt.key(ctrl_a);
    shell(&mut rt);
    rt.update(|h, _| assert!(h.doc.all_selected(), "a second Ctrl+A keeps it"));
    let u3 = rect(&rt, "doc-u3/t");
    rt.physical_input(PhysicalInput::Modifiers(Mods {
        shift: true,
        ..Mods::default()
    }));
    rt.pointer_down(u3[0] + 2.0, u3[1] + 4.0);
    rt.pointer_up(u3[0] + 2.0, u3[1] + 4.0);
    rt.physical_input(PhysicalInput::Modifiers(Mods::default()));
    shell(&mut rt);
    rt.update(|h, _| assert!(!h.doc.all_selected(), "the Shift-click ended it"));
    command(&mut rt, 'c');
    let got = clip.text().map(|t| t.to_string()).unwrap_or_default();
    assert!(
        got.starts_with("unit 0\nunit 1\nunit 2\n") && got.len() < 40,
        "{got:?}"
    );
}

#[test]
fn a_second_copy_press_keeps_the_check_two_seconds_from_itself() {
    // Each press asks for its own wake: the first one's comes while the second's check still
    // has time left, and the second's turns it back, on a window at rest.
    let mut rt = runtime(vec![Unit::new(
        UnitKind::Code {
            list_depth: None,
            language: None,
        },
        "let a = 1;",
    )]);
    rt.poll = Some(host_poll);
    settle(&mut rt);
    let label = |rt: &Runtime<Host>| {
        rt.elements()
            .into_iter()
            .find(|e| is(&e.path, "doc-u0/copy"))
            .and_then(|e| e.label)
    };
    let b = rect(&rt, "doc-u0/copy");
    let at = (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0);
    click(&mut rt, at);
    shell(&mut rt);
    rt.frame(1.5);
    shell(&mut rt);
    click(&mut rt, at);
    shell(&mut rt);
    let second = rt.clock;
    let mut back = None;
    for _ in 0..10 {
        let Some(due) = rt.wake_deadline() else { break };
        rt.frame((due - rt.clock).max(1.0 / 60.0));
        shell(&mut rt);
        if label(&rt).as_deref() == Some("Copy code") {
            back = Some(rt.clock - second);
            break;
        }
    }
    let back = back.expect("the check turned back on its own");
    assert!(
        back >= 1.95,
        "turned back {back:.3} s after the second press"
    );
}

#[test]
fn the_double_click_time_is_the_hosts() {
    // Two clicks on one link `gap` apart open it twice when the host's double-click time is
    // shorter than the gap, once when it is longer (neither holds at the 0.5 s default).
    for (interval, gap, opens) in [(0.2, 0.3, 2), (1.0, 0.6, 1)] {
        let (mut rt, at) = link_runtime();
        rt.update(|h, _| h.doc.set_double_click_interval(interval));
        click(&mut rt, at);
        rt.frame(gap);
        click(&mut rt, at);
        settle(&mut rt);
        rt.update(|h, _| {
            assert_eq!(
                h.intents.len(),
                opens,
                "interval {interval} s, clicks {gap} s apart"
            )
        });
    }
}
