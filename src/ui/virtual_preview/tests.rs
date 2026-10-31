// SPDX-License-Identifier: MIT

use std::{rc::Rc, sync::Arc};

use gtk::prelude::*;
use sourceview5::prelude::*;

use super::{
    DocumentSelection, PreviewUnit, SelectionPoint, SourceUnit, VirtualPreviewState,
    bind_document_row, bind_source_row, bounded_text_prefix, code_block_copy_text,
    document_tag_table, document_text_view, drag_threshold_crossed, highlighted_code_language,
    local_selection, matching_link, plain_text_view, rendered_document, selection_text,
    set_table_cell, source_document, source_line_numbers, source_units, styled_markup,
    use_virtual_plain_source,
};
use crate::{
    services::{
        DocumentLayout, DocumentSpan, DocumentSpanStyle, DocumentTableCellLayout, DocumentUnit,
        DocumentUnitKind,
    },
    test_support::gtk_test,
};

#[test]
fn smooth_content_scrolling_keeps_thumb_and_hit_testing_in_document_order() {
    gtk_test(
        "ui::virtual_preview::tests::smooth_content_scrolling_keeps_thumb_and_hit_testing_in_document_order",
        || {
            let context = gtk::glib::MainContext::default();
            let advance_frames = |duration| {
                let deadline = std::time::Instant::now() + duration;
                while std::time::Instant::now() < deadline {
                    while context.pending() {
                        context.iteration(false);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            };
            let _themes = crate::ui::theme::ThemeManager::shared();
            let content = (0..400)
                .map(|index| {
                    if index % 2 == 0 {
                        format!("Short paragraph {index}.")
                    } else {
                        (0..10 + (index * 37) % 150)
                            .map(|word| format!("paragraph-{index}-word-{word}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            let cancellation = crate::sandbox::Cancellation::default();
            let kind = crate::services::document_kind(
                "text/markdown",
                std::ffi::OsStr::new("scroll.md"),
                true,
            )
            .expect("Markdown");
            let markdown = content
                .split("\n\n")
                .enumerate()
                .map(|(index, text)| format!("[{text}](https://example.test/{index})"))
                .collect::<Vec<_>>()
                .join("\n\n");
            let parsed = crate::services::parse_document(kind, &markdown, &cancellation)
                .expect("scroll fixture");
            let layout = crate::services::layout_document(parsed.document, &cancellation)
                .expect("scroll layout");
            let previews = [
                rendered_document(layout, Vec::new(), true, None),
                source_document(&content, false, true),
            ];
            for (mode, (root, state)) in previews.into_iter().enumerate() {
                let window = gtk::Window::builder()
                    .default_width(480)
                    .default_height(500)
                    .child(&root)
                    .build();
                window.present();
                advance_frames(std::time::Duration::from_millis(300));
                let list = state.list.upgrade().expect("document list");
                let scroll = list
                    .ancestor(gtk::ScrolledWindow::static_type())
                    .and_downcast::<gtk::ScrolledWindow>()
                    .expect("document scroller");
                let controllers = scroll
                    .observe_controllers()
                    .iter::<gtk::glib::Object>()
                    .filter_map(Result::ok)
                    .filter_map(|controller| {
                        controller.downcast::<gtk::EventControllerScroll>().ok()
                    })
                    .collect::<Vec<_>>();
                let bubble = controllers
                    .iter()
                    .find(|controller| {
                        controller.propagation_phase() == gtk::PropagationPhase::Bubble
                    })
                    .expect("GTK scrolling controller");
                let capture = controllers
                    .iter()
                    .find(|controller| {
                        controller.propagation_phase() == gtk::PropagationPhase::Capture
                            && controller
                                .flags()
                                .contains(gtk::EventControllerScrollFlags::KINETIC)
                    })
                    .expect("GTK smooth scrolling controller");
                let phase = |signal| {
                    for controller in &controllers {
                        controller.emit_by_name::<()>(signal, &[]);
                    }
                };
                let scroll_by = |dy: f64| {
                    if !controllers
                        .iter()
                        .filter(|controller| {
                            controller.propagation_phase() == gtk::PropagationPhase::Capture
                        })
                        .any(|controller| {
                            controller.emit_by_name::<bool>("scroll", &[&0.0_f64, &dy])
                        })
                    {
                        bubble.emit_by_name::<bool>("scroll", &[&0.0_f64, &dy]);
                    }
                };
                let thumb = scroll
                    .parent()
                    .and_then(|parent| parent.last_child())
                    .and_downcast::<gtk::Scrollbar>()
                    .expect("document scrollbar")
                    .adjustment();
                let thumb_progress = || {
                    (thumb.value() - thumb.lower())
                        / (thumb.upper() - thumb.lower() - thumb.page_size()).max(1.0)
                };
                let point = || {
                    let x = f64::from(list.width()) / 4.0;
                    let y = f64::from(list.height()) / 2.0;
                    if mode == 0 {
                        super::link_point_at(&list, &state, x, y)
                    } else {
                        super::point_at(&list, &state, x, y)
                    }
                };
                phase("scroll-begin");
                scroll_by(0.25);
                advance_frames(std::time::Duration::from_millis(300));
                let before_momentum = scroll.vadjustment().value();
                capture.emit_by_name::<()>("decelerate", &[&0.0_f64, &0.35_f64]);
                phase("scroll-end");
                advance_frames(std::time::Duration::from_millis(400));
                assert!(
                    scroll.vadjustment().value() > before_momentum,
                    "native momentum must still advance a settled viewport in mode {mode}"
                );
                phase("scroll-begin");
                let mut previous_progress = thumb_progress();
                let mut previous = point().expect("initial document text");
                let initial = previous;
                let mut samples = 0;
                for step in 0..240 {
                    for _ in 0..if step % 40 == 0 { 16 } else { 1 } {
                        scroll_by(0.75);
                    }
                    advance_frames(std::time::Duration::from_millis(20));
                    let progress = thumb_progress();
                    assert!(
                        progress + 1e-6 >= previous_progress,
                        "forward content scrolling moved the scrollbar backwards in mode {mode}: {previous_progress} -> {progress}; document value={} upper={}",
                        scroll.vadjustment().value(),
                        scroll.vadjustment().upper(),
                    );
                    previous_progress = progress;
                    let current = point();
                    let picked = list
                        .pick(
                            f64::from(list.width()) / 4.0,
                            f64::from(list.height()) / 2.0,
                            gtk::PickFlags::DEFAULT,
                        )
                        .and_then(|picked| {
                            state.bound.borrow().iter().find_map(|(index, bound)| {
                                bound
                                    .root
                                    .upgrade()
                                    .filter(|root| picked == *root || picked.is_ancestor(root))
                                    .map(|_| *index)
                            })
                        });
                    if let Some(current) = current {
                        if picked.is_some() {
                            assert_eq!(
                                Some(current.unit),
                                picked,
                                "hit testing must read the visible row, not a recycled row"
                            );
                        }
                        assert!(
                            if mode == 0 {
                                current >= previous
                            } else {
                                current.unit >= previous.unit
                            },
                            "forward scrolling reversed visible text: {previous:?} -> {current:?}",
                        );
                        previous = current;
                        samples += 1;
                    }
                }
                capture.emit_by_name::<()>("decelerate", &[&0.0_f64, &0.35_f64]);
                phase("scroll-end");
                for _ in 0..60 {
                    advance_frames(std::time::Duration::from_millis(20));
                    let progress = thumb_progress();
                    assert!(
                        progress + 1e-6 >= previous_progress,
                        "forward momentum moved the scrollbar backwards in mode {mode}: {previous_progress} -> {progress}"
                    );
                    previous_progress = progress;
                }
                advance_frames(std::time::Duration::from_millis(300));
                assert!(
                    samples > 100,
                    "scrolling must exercise actual document text"
                );
                assert!(
                    previous > initial,
                    "smooth input must advance document text"
                );
                let stopped = point();
                advance_frames(std::time::Duration::from_millis(300));
                assert_eq!(
                    point(),
                    stopped,
                    "stopping input must leave the document still"
                );
                phase("scroll-begin");
                previous_progress = thumb_progress();
                for _ in 0..120 {
                    scroll_by(-0.75);
                    advance_frames(std::time::Duration::from_millis(20));
                    let progress = thumb_progress();
                    assert!(
                        progress <= previous_progress + 1e-6,
                        "backward input moved the scrollbar forward in mode {mode}: {previous_progress} -> {progress}"
                    );
                    previous_progress = progress;
                }
                phase("scroll-end");
                window.close();
            }
        },
    );
}

#[test]
fn standalone_table_keeps_copy_model_alive_without_document_state() {
    gtk_test(
        "ui::virtual_preview::tests::standalone_table_keeps_copy_model_alive_without_document_state",
        || {
            let cancellation = crate::sandbox::Cancellation::default();
            let kind =
                crate::services::document_kind("text/csv", std::ffi::OsStr::new("table.csv"), true)
                    .expect("CSV kind");
            let parsed = crate::services::parse_document(kind, "value\n10\n2\n", &cancellation)
                .expect("CSV");
            let layout =
                crate::services::layout_document(parsed.document, &cancellation).expect("layout");
            let (widget, state) = rendered_document(layout, Vec::new(), false, None);
            let table = Rc::downgrade(state.tables.borrow().get(&0).expect("standalone table"));
            drop(state);
            assert_eq!(
                table.upgrade().expect("widget owns copy model").copy_text(),
                "value\n2\n10\n"
            );
            drop(widget);
            assert!(table.upgrade().is_none(), "closing table releases model");
        },
    );
}

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
            .all(|unit| unit.display_text().len() <= super::PATHOLOGICAL_TEXT_UNIT_BYTES)
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
    assert!(use_virtual_plain_source(
        &"x".repeat(super::PATHOLOGICAL_TEXT_UNIT_BYTES + 1)
    ));
    assert!(!use_virtual_plain_source("ordinary source\n"));
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
            .all(|unit| unit.display_text().len() <= super::PATHOLOGICAL_TEXT_UNIT_BYTES)
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
    let mut state = VirtualPreviewState {
        list: Default::default(),
        tables: Default::default(),
        media_cache: crate::ui::document_media::MediaCache::new(None),
        units,
        wrapped: std::cell::Cell::new(false),
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

    for content in [
        "\n".to_owned(),
        format!("{}\n", "line\n".repeat(super::SOURCE_UNIT_LINES)),
        format!(
            "{}\n\n",
            "x".repeat(super::PATHOLOGICAL_TEXT_UNIT_BYTES + 1)
        ),
        format!("{}\n\n{}", "x".repeat(2_049), "y".repeat(2_049)),
    ] {
        let (units, _) = source_units(&content);
        state.units = Rc::new(units.into_iter().map(PreviewUnit::Source).collect());
        let last = state.units.len() - 1;
        state.selection.set(Some(DocumentSelection {
            anchor: SelectionPoint { unit: 0, offset: 0 },
            focus: SelectionPoint {
                unit: last,
                offset: state.units[last].selection_len(),
            },
        }));
        assert_eq!(selection_text(&state).as_deref(), Some(content.as_str()));
    }
    let cancellation = crate::sandbox::Cancellation::default();
    let parsed = crate::services::parse_document(
        crate::services::document_kind("text/markdown", std::ffi::OsStr::new("math.md"), true)
            .expect("Markdown"),
        "Before $E=mc^2$ after",
        &cancellation,
    )
    .expect("equation paragraph");
    let layout =
        crate::services::layout_document(parsed.document, &cancellation).expect("equation layout");
    state.units = Rc::new(
        layout
            .units
            .into_iter()
            .map(PreviewUnit::Document)
            .collect(),
    );
    state.selection.set(Some(DocumentSelection {
        anchor: SelectionPoint { unit: 0, offset: 7 },
        focus: SelectionPoint { unit: 0, offset: 8 },
    }));
    assert_eq!(selection_text(&state).as_deref(), Some("$E=mc^2$"));
}

#[test]
fn selection_does_not_invent_newlines_between_line_chunks() {
    let units = Rc::new(vec![source("abcd", "abcd"), source("ef", "ef\n")]);
    let state = VirtualPreviewState {
        list: Default::default(),
        units,
        wrapped: std::cell::Cell::new(false),
        selection: std::cell::Cell::new(Some(DocumentSelection {
            anchor: SelectionPoint { unit: 0, offset: 2 },
            focus: SelectionPoint { unit: 1, offset: 1 },
        })),
        tables: Default::default(),
        media_cache: crate::ui::document_media::MediaCache::new(None),
        bound: std::cell::RefCell::default(),
        dragging: std::cell::Cell::new(false),
        press: std::cell::Cell::new((0.0, 0.0)),
        pointer: std::cell::Cell::new((0.0, 0.0)),
        drag_generation: std::cell::Cell::new(0),
        hovered: std::cell::Cell::new(None),
        pressed_link: std::cell::RefCell::new(None),
    };

    assert_eq!(selection_text(&state).as_deref(), Some("cde"));
    let state = Rc::new(state);
    let matches = state.find_matches("CDE");
    assert_eq!(matches.len(), 1);
    state.select_match(matches[0]);
    assert_eq!(selection_text(&state).as_deref(), Some("cde"));
}

#[test]
fn document_caret_keys_take_ctrl_and_shift_but_not_alt() {
    gtk_test(
        "ui::virtual_preview::tests::document_caret_keys_take_ctrl_and_shift_but_not_alt",
        || {
            use gtk::gdk::{Key, ModifierType as Modifiers};
            let (_root, state) = source_document("one two\nthree\n", false, false);
            let list = state.list.upgrade().expect("document list");
            let keys = list
                .observe_controllers()
                .iter::<gtk::glib::Object>()
                .filter_map(Result::ok)
                .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
                .expect("document key controller");
            let press = |key: Key, modifiers: Modifiers| {
                keys.emit_by_name::<bool>("key-pressed", &[&key, &0u32, &modifiers])
            };
            let last = state.units.len() - 1;
            let start = SelectionPoint { unit: 0, offset: 0 };
            let whole = Some(DocumentSelection {
                anchor: start,
                focus: SelectionPoint {
                    unit: last,
                    offset: state.units[last].selection_len(),
                },
            });

            assert!(press(
                Key::End,
                Modifiers::CONTROL_MASK | Modifiers::SHIFT_MASK
            ));
            assert_eq!(state.selection.get(), whole, "Ctrl+Shift+End extends");
            assert!(
                !press(Key::Left, Modifiers::ALT_MASK),
                "Alt+Left is history"
            );
            assert_eq!(state.selection.get(), whole);
            assert!(press(Key::KP_Home, Modifiers::CONTROL_MASK));
            assert_eq!(
                state.selection.get(),
                Some(DocumentSelection {
                    anchor: start,
                    focus: start,
                }),
                "keypad Ctrl+Home returns to the start"
            );
        },
    );
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
        list: Default::default(),
        units,
        wrapped: std::cell::Cell::new(false),
        selection: std::cell::Cell::new(Some(DocumentSelection {
            anchor: SelectionPoint { unit: 0, offset: 0 },
            focus: SelectionPoint { unit: 0, offset: 1 },
        })),
        tables: Default::default(),
        media_cache: crate::ui::document_media::MediaCache::new(None),
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
    let markup = styled_markup("abcdef", &spans).expect("bounded cell markup");
    let pango_markup = markup
        .replace("<a href=\"https://example.test\">", "<u>")
        .replace("</a>", "</u>");
    let (_, plain, _) =
        gtk::pango::parse_markup(&pango_markup, '\0').expect("balanced Pango markup");
    assert_eq!(plain, "abcdef");
    assert!(markup.contains("href=\"https://example.test\""));
}

#[test]
fn table_formatting_bounds_link_reexpansion_before_gtk() {
    let html = format!(
        "<table><tr><td><a href=\"https://example.test/{}\">{}</a></td></tr></table>",
        "x".repeat(4_096),
        "<b>x</b>".repeat(32),
    );
    let cancellation = crate::sandbox::Cancellation::default();
    let kind = crate::services::document_kind("text/html", std::ffi::OsStr::new("page.html"), true)
        .expect("HTML document");
    let parsed = crate::services::parse_document(kind, &html, &cancellation)
        .expect("the bounded source is valid HTML");
    let layout = crate::services::layout_document(parsed.document, &cancellation)
        .expect("the semantic layout remains bounded");
    let DocumentUnitKind::Table { rows, .. } = &layout.units[0].kind else {
        panic!("expected a table");
    };
    let cell = &rows[0][0];
    assert_eq!(cell.text, "x".repeat(32));
    assert!(styled_markup(&cell.text, &cell.spans).is_none());
    assert_eq!(layout.units[0].copy_text, format!("{}\n", cell.text));
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
    gtk_test(
        "ui::virtual_preview::tests::virtual_preview_reuses_source_rows_and_releases_widget_trees",
        || {
            let nul_view = plain_text_view("before\0after", true);
            let nul_buffer = nul_view.buffer();
            assert_eq!(
                nul_buffer.text(&nul_buffer.start_iter(), &nul_buffer.end_iter(), false),
                "before�after"
            );
            drop(nul_view);

            let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let first = SourceUnit::new("first".to_owned(), 1, 1, false);
            let first_view = bind_source_row(&row, &first, false)
                .view
                .upgrade()
                .expect("source row should contain a text view");
            let second = SourceUnit::new("second".to_owned(), 2, 1, false);
            let second_view = bind_source_row(&row, &second, false)
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
            let first_view = bind_document_row(
                &row,
                document(&units[0]),
                units.clone(),
                0,
                &document_tags,
                false,
            );
            let first_buffer = first_view.buffer();
            let second_view = bind_document_row(
                &row,
                document(&units[1]),
                units.clone(),
                1,
                &document_tags,
                false,
            );
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
            let third_view = bind_document_row(
                &row,
                document(&units[2]),
                units.clone(),
                2,
                &document_tags,
                false,
            );
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
                false,
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
            let rust_view =
                document_text_view(&code("rust", "let value = 1;"), &document_tags, false);
            let python_view =
                document_text_view(&code("python3", "value = 1"), &document_tags, false);
            let rust_buffer = rust_view
                .buffer()
                .downcast::<sourceview5::Buffer>()
                .expect("Rust code should use a source buffer");
            let python_buffer = python_view
                .buffer()
                .downcast::<sourceview5::Buffer>()
                .expect("Python code should use a source buffer");
            assert_ne!(rust_buffer.tag_table(), python_buffer.tag_table());
            assert!(
                python_buffer.iter_has_context_class(&python_buffer.start_iter(), "no-spell-check")
            );
            drop((rust_buffer, rust_view));
            while gtk::glib::MainContext::default().pending() {
                gtk::glib::MainContext::default().iteration(false);
            }
            assert!(
                python_buffer.iter_has_context_class(&python_buffer.start_iter(), "no-spell-check")
            );

            let label = gtk::Label::new(None);
            set_table_cell(
                &label,
                &DocumentTableCellLayout {
                    header: true,
                    text: "Header".into(),
                    spans: Vec::new(),
                },
            );
            set_table_cell(
                &label,
                &DocumentTableCellLayout {
                    header: false,
                    text: "Cell".into(),
                    spans: Vec::new(),
                },
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
                label,
            ));

            let root = rendered_document(
                DocumentLayout {
                    units: vec![unit("first"), unit("second")],
                },
                Vec::new(),
                false,
                None,
            )
            .0;
            let weak = root.downgrade();

            drop(root);
            while gtk::glib::MainContext::default().pending() {
                gtk::glib::MainContext::default().iteration(false);
            }

            assert!(weak.upgrade().is_none());

            let source_root = source_document(&"x".repeat(1024 * 1024), false, false).0;
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
            let (source_root, source_state) =
                super::virtual_preview(units, Vec::new(), true, false, None);
            stack.add_named(&source_root, Some("source"));
            stack.set_visible_child_name("source");
            while gtk::glib::MainContext::default().pending() {
                gtk::glib::MainContext::default().iteration(false);
            }
            assert!(source_root.is_mapped());
            assert!(!source_state.bound.borrow().is_empty());
            for bound in source_state.bound.borrow().values() {
                let view = bound.view.upgrade().expect("bound source view");
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
            let (rendered_root, rendered_state) = super::virtual_preview(
                units,
                vec!["Unsupported content omitted".to_owned()],
                false,
                false,
                None,
            );
            stack.add_named(&rendered_root, Some("rendered-again"));
            stack.set_visible_child_name("rendered-again");
            while gtk::glib::MainContext::default().pending() {
                gtk::glib::MainContext::default().iteration(false);
            }
            assert!(rendered_root.is_mapped());
            assert!(!rendered_state.bound.borrow().is_empty());
            for bound in rendered_state.bound.borrow().values() {
                let view = bound.view.upgrade().expect("bound rendered view");
                assert!(view.buffer().char_count() > 0);
            }
            window.close();
        },
    );
}

fn document(unit: &PreviewUnit) -> &DocumentUnit {
    let PreviewUnit::Document(unit) = unit else {
        panic!("expected document unit");
    };
    unit
}

fn source(display: &str, source: &str) -> PreviewUnit {
    PreviewUnit::Source(SourceUnit {
        display_end: display.strip_suffix('\n').unwrap_or(display).len(),
        source: source.to_owned(),
        first_line: 1,
        line_count: 1,
        continuation: false,
    })
}
