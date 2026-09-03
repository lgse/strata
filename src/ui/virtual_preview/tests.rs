// SPDX-License-Identifier: GPL-3.0-or-later

use std::{rc::Rc, sync::Arc};

use gtk::prelude::*;
use sourceview5::prelude::*;

use super::{
    DocumentSelection, PreviewUnit, SelectionPoint, SourceUnit, VirtualPreviewState,
    bind_document_row, bind_document_table_row, bind_source_row, bounded_text_prefix,
    code_block_copy_text, document_tag_table, document_text_view, drag_threshold_crossed,
    highlighted_code_language, local_selection, matching_link, plain_text_view, rendered_document,
    selection_text, source_document, source_line_numbers, source_line_numbers_view, source_units,
    styled_markup, use_virtual_source, vertical_distance,
};
use crate::{
    services::{
        DocumentLayout, DocumentSpan, DocumentSpanStyle, DocumentTableCellLayout, DocumentUnit,
        DocumentUnitKind,
    },
    test_support::ASYNC_MAIN_CONTEXT_DEFAULT,
};

#[test]
fn source_units_bound_normal_rows_and_isolate_pathological_lines() {
    let content = format!(
        "{}{}\ntail\n",
        "short\n".repeat(300),
        "x".repeat(super::PATHOLOGICAL_TEXT_UNIT_BYTES + 1)
    );
    let (units, split_lines) = source_units(&content);

    assert!(split_lines);
    assert!(units.iter().all(|unit| {
        unit.source.len() <= super::SOURCE_UNIT_BYTES && unit.line_count <= super::SOURCE_UNIT_LINES
    }));
    let long = units
        .iter()
        .filter(|unit| unit.first_line == 301)
        .collect::<Vec<_>>();
    assert_eq!(long.len(), 2);
    assert!(
        long.iter()
            .all(|unit| unit.display.len() <= super::PATHOLOGICAL_TEXT_UNIT_BYTES)
    );
    assert!(long[1].continuation);
    assert_eq!(source_line_numbers(long[0]), "301");
    assert_eq!(source_line_numbers(long[1]), "↳");
    assert_eq!(units.iter().map(|unit| unit.line_count).sum::<usize>(), 302);
    assert_eq!(units.last().map(|unit| unit.first_line), Some(302));
    assert_eq!(
        units
            .iter()
            .map(|unit| unit.source.as_str())
            .collect::<String>(),
        content
    );
}

#[test]
fn small_pathological_lines_use_the_virtual_source_path() {
    assert!(use_virtual_source(
        &"x".repeat(super::PATHOLOGICAL_TEXT_UNIT_BYTES + 1)
    ));
    assert!(!use_virtual_source("ordinary source\n"));
}

#[test]
fn maximum_source_lines_stay_bounded_and_complete() {
    let content = format!("{}\n", "x".repeat(64 * 1024 - 1)).repeat(16);
    let (units, split_lines) = source_units(&content);

    assert!(split_lines);
    assert_eq!(units.len(), 512);
    assert!(
        units
            .iter()
            .all(|unit| unit.display.len() <= super::PATHOLOGICAL_TEXT_UNIT_BYTES)
    );
    assert_eq!(
        units
            .iter()
            .map(|unit| unit.source.as_str())
            .collect::<String>(),
        content
    );
}

#[test]
fn cross_row_selection_copies_full_middle_units_from_the_model() {
    let units = Rc::new(vec![
        source("first line\n", "first line\n"),
        source("visible", "visible plus an unrendered tail\n"),
        source("last line", "last line"),
    ]);
    let state = VirtualPreviewState {
        units,
        selection: std::cell::Cell::new(Some(DocumentSelection {
            anchor: SelectionPoint { unit: 0, offset: 6 },
            focus: SelectionPoint { unit: 2, offset: 4 },
        })),
        bound: std::cell::RefCell::default(),
        dragging: std::cell::Cell::new(false),
        press: std::cell::Cell::new((0.0, 0.0)),
        pointer: std::cell::Cell::new((0.0, 0.0)),
        drag_generation: std::cell::Cell::new(0),
        hovered: std::cell::Cell::new(None),
        pressed_link: std::cell::RefCell::new(None),
    };

    assert_eq!(
        selection_text(&state).as_deref(),
        Some("line\nvisible plus an unrendered tail\nlast")
    );
    assert_eq!(local_selection(state.selection.get(), 1, 7), Some((0, 7)));
}

#[test]
fn selection_does_not_invent_newlines_between_line_chunks() {
    let units = Rc::new(vec![source("abcd", "abcd"), source("ef", "ef\n")]);
    let state = VirtualPreviewState {
        units,
        selection: std::cell::Cell::new(Some(DocumentSelection {
            anchor: SelectionPoint { unit: 0, offset: 2 },
            focus: SelectionPoint { unit: 1, offset: 1 },
        })),
        bound: std::cell::RefCell::default(),
        dragging: std::cell::Cell::new(false),
        press: std::cell::Cell::new((0.0, 0.0)),
        pointer: std::cell::Cell::new((0.0, 0.0)),
        drag_generation: std::cell::Cell::new(0),
        hovered: std::cell::Cell::new(None),
        pressed_link: std::cell::RefCell::new(None),
    };

    assert_eq!(selection_text(&state).as_deref(), Some("cde"));
}

#[test]
fn bounded_table_text_keeps_utf8_boundaries() {
    assert_eq!(bounded_text_prefix("abλcd", 3), "ab");
}

#[test]
fn table_selection_is_atomic_and_copies_tsv() {
    let units = Rc::new(vec![PreviewUnit::Document(DocumentUnit {
        kind: DocumentUnitKind::Table {
            list_depth: None,
            rows: Vec::new(),
        },
        text: String::new(),
        copy_text: "A\tB\n1\t2\n".to_owned(),
        spans: Vec::new(),
        wrap: false,
        first: true,
        last: true,
    })]);
    let state = VirtualPreviewState {
        units,
        selection: std::cell::Cell::new(Some(DocumentSelection {
            anchor: SelectionPoint { unit: 0, offset: 0 },
            focus: SelectionPoint { unit: 0, offset: 1 },
        })),
        bound: std::cell::RefCell::default(),
        dragging: std::cell::Cell::new(false),
        press: std::cell::Cell::new((0.0, 0.0)),
        pointer: std::cell::Cell::new((0.0, 0.0)),
        drag_generation: std::cell::Cell::new(0),
        hovered: std::cell::Cell::new(None),
        pressed_link: std::cell::RefCell::new(None),
    };

    assert_eq!(selection_text(&state).as_deref(), Some("A\tB\n1\t2\n"));
}

#[test]
fn visible_table_markup_remains_balanced_with_overlapping_styles() {
    let spans = vec![
        DocumentSpan {
            range: 0..4,
            style: DocumentSpanStyle::Bold,
        },
        DocumentSpan {
            range: 2..6,
            style: DocumentSpanStyle::Link(Arc::from("https://example.test")),
        },
    ];
    let markup = styled_markup("abcdef", &spans);
    let pango_markup = markup
        .replace("<a href=\"https://example.test\">", "<u>")
        .replace("</a>", "</u>");
    let (_, plain, _) =
        gtk::pango::parse_markup(&pango_markup, '\0').expect("balanced Pango markup");
    assert_eq!(plain, "abcdef");
    assert!(markup.contains("href=\"https://example.test\""));
}

#[test]
fn links_activate_only_when_press_and_release_match() {
    assert_eq!(
        matching_link(Some("https://example.test"), Some("https://example.test")),
        Some("https://example.test")
    );
    assert_eq!(
        matching_link(Some("https://first.test"), Some("https://second.test")),
        None
    );
    assert_eq!(matching_link(None, Some("https://example.test")), None);
    assert_eq!(matching_link(Some("https://example.test"), None), None);
}

#[test]
fn gaps_resolve_to_the_nearest_adjacent_row() {
    let rows = [
        gtk::graphene::Rect::new(0.0, 0.0, 100.0, 10.0),
        gtk::graphene::Rect::new(0.0, 20.0, 100.0, 10.0),
        gtk::graphene::Rect::new(0.0, 200.0, 100.0, 10.0),
    ];
    let nearest = |y| {
        rows.iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                vertical_distance(left, y).total_cmp(&vertical_distance(right, y))
            })
            .map(|(index, _)| index)
    };

    assert_eq!(nearest(14.0), Some(0));
    assert_eq!(nearest(16.0), Some(1));
}

#[test]
fn code_copy_reassembles_one_virtualized_block() {
    let code = |text: &str, first, last| {
        PreviewUnit::Document(DocumentUnit {
            kind: DocumentUnitKind::Code {
                list_depth: None,
                language: Some("rust"),
            },
            text: text.trim_end_matches('\n').to_owned(),
            copy_text: text.to_owned(),
            spans: Vec::new(),
            wrap: false,
            first,
            last,
        })
    };
    let units = vec![
        code("first\n", true, false),
        code("second\n", false, true),
        code("separate\n", true, true),
    ];

    assert_eq!(
        code_block_copy_text(&units, 1).as_deref(),
        Some("first\nsecond\n")
    );
    assert_eq!(
        code_block_copy_text(&units, 2).as_deref(),
        Some("separate\n")
    );
    assert_eq!(highlighted_code_language(document(&units[2])), Some("rust"));
    assert_eq!(highlighted_code_language(document(&units[0])), None);
}

#[test]
fn virtual_preview_reuses_source_rows_and_releases_widget_trees() {
    let _serial = ASYNC_MAIN_CONTEXT_DEFAULT
        .lock()
        .expect("the async test lock should not be poisoned");
    if gtk::init().is_err() {
        return;
    }
    let nul_view = plain_text_view("before\0after", true);
    let nul_buffer = nul_view.buffer();
    assert_eq!(
        nul_buffer.text(&nul_buffer.start_iter(), &nul_buffer.end_iter(), false),
        "before�after"
    );
    drop(nul_view);

    let long_view = plain_text_view(&"x".repeat(2_048), true);
    assert!(long_view.hexpands());
    assert!(long_view.measure(gtk::Orientation::Horizontal, -1).0 > 16);
    drop(long_view);

    let source = (1..=200)
        .map(|line| format!("source line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let source_view = plain_text_view(&source, true);
    let numbers = source_line_numbers_view(&SourceUnit {
        display: source.clone(),
        source,
        first_line: 1,
        line_count: 200,
        continuation: false,
    });
    assert_eq!(
        numbers.measure(gtk::Orientation::Vertical, -1).0,
        source_view.measure(gtk::Orientation::Vertical, -1).0
    );
    drop((numbers, source_view));

    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let first = SourceUnit {
        display: "first".to_owned(),
        source: "first".to_owned(),
        first_line: 1,
        line_count: 1,
        continuation: false,
    };
    let first_view = bind_source_row(&row, &first)
        .view
        .upgrade()
        .expect("source row should contain a text view");
    let second = SourceUnit {
        display: "second".to_owned(),
        source: "second".to_owned(),
        first_line: 2,
        line_count: 1,
        continuation: false,
    };
    let second_view = bind_source_row(&row, &second)
        .view
        .upgrade()
        .expect("source row should retain its text view");
    assert_eq!(first_view, second_view);
    assert_eq!(
        second_view.buffer().text(
            &second_view.buffer().start_iter(),
            &second_view.buffer().end_iter(),
            false
        ),
        "second"
    );
    drop((first_view, second_view, row));

    let threshold = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    assert!(!drag_threshold_crossed(&threshold, 10.0, 10.0, 10.0, 10.0));
    assert!(drag_threshold_crossed(&threshold, 10.0, 10.0, 100.0, 100.0));

    let unit = |text: &str| DocumentUnit {
        kind: DocumentUnitKind::Paragraph,
        text: text.to_owned(),
        copy_text: format!("{text}\n"),
        spans: Vec::new(),
        wrap: true,
        first: true,
        last: true,
    };
    let mut styled = unit("second");
    styled.kind = DocumentUnitKind::Heading(2);
    styled.spans.push(DocumentSpan {
        range: 0..6,
        style: DocumentSpanStyle::Bold,
    });
    let units = Rc::new(vec![
        PreviewUnit::Document(unit("first")),
        PreviewUnit::Document(styled),
        PreviewUnit::Document(unit("third")),
    ]);
    let document_tags = document_tag_table();
    let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let first_view = bind_document_row(&row, document(&units[0]), units.clone(), 0, &document_tags);
    let first_buffer = first_view.buffer();
    let second_view =
        bind_document_row(&row, document(&units[1]), units.clone(), 1, &document_tags);
    assert_eq!(first_view, second_view);
    assert_eq!(first_buffer, second_view.buffer());
    assert_eq!(
        second_view.buffer().text(
            &second_view.buffer().start_iter(),
            &second_view.buffer().end_iter(),
            false
        ),
        "second"
    );
    let tag_names = second_view
        .buffer()
        .iter_at_offset(1)
        .tags()
        .into_iter()
        .filter_map(|tag| tag.name())
        .collect::<Vec<_>>();
    assert!(tag_names.iter().any(|name| name == "document-heading-2"));
    assert!(tag_names.iter().any(|name| name == "document-bold"));
    assert_eq!(second_view.accessible_role(), gtk::AccessibleRole::Heading);
    let third_view = bind_document_row(&row, document(&units[2]), units.clone(), 2, &document_tags);
    assert_eq!(second_view, third_view);
    assert_eq!(third_view.accessible_role(), gtk::AccessibleRole::Generic);
    let third_tag_names = third_view
        .buffer()
        .iter_at_offset(1)
        .tags()
        .into_iter()
        .filter_map(|tag| tag.name())
        .collect::<Vec<_>>();
    assert!(
        !third_tag_names
            .iter()
            .any(|name| name == "document-heading-2" || name == "document-bold")
    );
    let other_row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let other_view = bind_document_row(
        &other_row,
        document(&units[0]),
        units.clone(),
        0,
        &document_tags,
    );
    assert_eq!(
        third_view.buffer().tag_table(),
        other_view.buffer().tag_table()
    );

    let code = |language: &'static str, text: &str| DocumentUnit {
        kind: DocumentUnitKind::Code {
            list_depth: None,
            language: Some(language),
        },
        text: text.to_owned(),
        copy_text: format!("{text}\n"),
        spans: Vec::new(),
        wrap: false,
        first: true,
        last: true,
    };
    let rust_view = document_text_view(&code("rust", "let value = 1;"), &document_tags);
    let python_view = document_text_view(&code("python3", "value = 1"), &document_tags);
    let rust_buffer = rust_view
        .buffer()
        .downcast::<sourceview5::Buffer>()
        .expect("Rust code should use a source buffer");
    let python_buffer = python_view
        .buffer()
        .downcast::<sourceview5::Buffer>()
        .expect("Python code should use a source buffer");
    assert_ne!(rust_buffer.tag_table(), python_buffer.tag_table());
    assert!(python_buffer.iter_has_context_class(&python_buffer.start_iter(), "no-spell-check"));
    drop((rust_buffer, rust_view));
    while gtk::glib::MainContext::default().pending() {
        gtk::glib::MainContext::default().iteration(false);
    }
    assert!(python_buffer.iter_has_context_class(&python_buffer.start_iter(), "no-spell-check"));

    let table_row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let table = bind_document_table_row(
        &table_row,
        &[vec![DocumentTableCellLayout {
            header: true,
            text: "Header".to_owned(),
            spans: Vec::new(),
        }]],
    );
    let label = table
        .first_child()
        .and_downcast::<gtk::Label>()
        .expect("table should contain a label");
    let rebound_table = bind_document_table_row(
        &table_row,
        &[vec![DocumentTableCellLayout {
            header: false,
            text: "Cell".to_owned(),
            spans: Vec::new(),
        }]],
    );
    assert_eq!(table, rebound_table);
    assert_eq!(
        label,
        rebound_table
            .first_child()
            .expect("rebound table should retain its label")
    );
    assert_eq!(label.text(), "Cell");
    assert!(!label.has_css_class("header"));
    drop((
        first_view,
        second_view,
        third_view,
        row,
        other_view,
        other_row,
        python_buffer,
        python_view,
        table,
        label,
        table_row,
    ));

    let root = rendered_document(
        DocumentLayout {
            units: vec![unit("first"), unit("second")],
        },
        Vec::new(),
    );
    let weak = root.downgrade();

    drop(root);
    while gtk::glib::MainContext::default().pending() {
        gtk::glib::MainContext::default().iteration(false);
    }

    assert!(weak.upgrade().is_none());

    let source_root = source_document(&"x".repeat(1024 * 1024), false);
    let weak_source = source_root.downgrade();
    drop(source_root);
    while gtk::glib::MainContext::default().pending() {
        gtk::glib::MainContext::default().iteration(false);
    }
    assert!(weak_source.upgrade().is_none());

    let stack = gtk::Stack::new();
    stack.add_named(&gtk::Label::new(Some("rendered")), Some("rendered"));
    stack.set_visible_child_name("rendered");
    let window = gtk::Window::builder()
        .default_width(500)
        .default_height(500)
        .child(&stack)
        .build();
    window.present();
    while gtk::glib::MainContext::default().pending() {
        gtk::glib::MainContext::default().iteration(false);
    }

    let content = "<table><tr><td>cell</td></tr></table>".repeat(300);
    let (source, _) = source_units(&content);
    let units = source.into_iter().map(PreviewUnit::Source).collect();
    let (source_root, source_state) = super::virtual_preview(units, Vec::new(), true);
    stack.add_named(&source_root, Some("source"));
    stack.set_visible_child_name("source");
    while gtk::glib::MainContext::default().pending() {
        gtk::glib::MainContext::default().iteration(false);
    }
    assert!(source_root.is_mapped());
    assert!(!source_state.bound.borrow().is_empty());
    for bound in source_state.bound.borrow().values() {
        let view = bound.view.upgrade().expect("bound source view");
        assert!(view.height_request() >= view.measure(gtk::Orientation::Vertical, view.width()).0);
        assert!(view.buffer().char_count() > 0);
    }

    let mut heading = unit("Mixed supported and unsupported HTML");
    heading.kind = DocumentUnitKind::Heading(1);
    let code = DocumentUnit {
        kind: DocumentUnitKind::Code {
            list_depth: Some(0),
            language: None,
        },
        text: "code inside a list\nwith a second line".to_owned(),
        copy_text: "code inside a list\nwith a second line\n".to_owned(),
        spans: Vec::new(),
        wrap: false,
        first: true,
        last: true,
    };
    let units = vec![
        PreviewUnit::Document(heading),
        PreviewUnit::Document(unit("Safe text remains visible.")),
        PreviewUnit::Document(code),
    ];
    let (rendered_root, rendered_state) =
        super::virtual_preview(units, vec!["Unsupported content omitted".to_owned()], false);
    stack.add_named(&rendered_root, Some("rendered-again"));
    stack.set_visible_child_name("rendered-again");
    while gtk::glib::MainContext::default().pending() {
        gtk::glib::MainContext::default().iteration(false);
    }
    assert!(rendered_root.is_mapped());
    assert!(!rendered_state.bound.borrow().is_empty());
    for bound in rendered_state.bound.borrow().values() {
        let view = bound.view.upgrade().expect("bound rendered view");
        assert!(view.height_request() >= view.measure(gtk::Orientation::Vertical, view.width()).0);
        assert!(view.buffer().char_count() > 0);
    }
    window.close();
}

fn document(unit: &PreviewUnit) -> &DocumentUnit {
    let PreviewUnit::Document(unit) = unit else {
        panic!("expected document unit");
    };
    unit
}

fn source(display: &str, source: &str) -> PreviewUnit {
    PreviewUnit::Source(SourceUnit {
        display: display.strip_suffix('\n').unwrap_or(display).to_owned(),
        source: source.to_owned(),
        first_line: 1,
        line_count: 1,
        continuation: false,
    })
}
