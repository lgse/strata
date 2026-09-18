// SPDX-License-Identifier: MIT

use super::super::SourcePreviewView;
use crate::{
    model::{EntryKind, FileEntry, Location, MetadataValue},
    test_support::gtk_test,
    ui::{media::tests::wait, theme::ThemeManager},
};
use gtk::{gdk, glib, prelude::*};
use sourceview5::prelude::*;

fn source_entry(name: &str) -> FileEntry {
    FileEntry {
        location: Location::local(format!("/fixture/{name}")),
        thumbnail_path: None,
        native_name: name.into(),
        display_name: name.into(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        recent_unix_seconds: MetadataValue::Unknown,
        mode: MetadataValue::Unknown,
        image_dimensions: MetadataValue::Unknown,
        child_count: MetadataValue::Unknown,
        duration_seconds: MetadataValue::Unknown,
        is_hidden: false,
    }
}

#[test]
fn javascript_highlighting_survives_large_many_line_and_long_line_sources() {
    gtk_test(
        "ui::preview::tests::source::javascript_highlighting_survives_large_many_line_and_long_line_sources",
        || {
            let manager = ThemeManager::shared();
            manager.set_follow_omarchy(false);
            manager.select_theme("catppuccin");
            let string_color = gdk::RGBA::parse(
                manager
                    .starter_tokens()
                    .syntax_string
                    .expect("theme string color"),
            )
            .expect("valid string color");
            let preview = SourcePreviewView::new();
            let entry = source_entry("large.js");
            for (case, padding) in [
                ("many lines", "// padding\n".repeat(600)),
                (
                    "many bytes",
                    format!("// {}\n", "x".repeat(1000)).repeat(150),
                ),
                ("long line", " ".repeat(4096)),
            ] {
                let content = format!(
                    "const head = \"highlighted-head\";\n{padding}const tail = \"highlighted-tail\";\n"
                );
                let (_widget, virtualized) =
                    preview.show(&entry, "text/javascript", &content, false);
                assert!(
                    !virtualized,
                    "{case} must retain the syntax-aware source view"
                );
                let buffer = preview
                    .view
                    .buffer()
                    .downcast::<sourceview5::Buffer>()
                    .expect("source buffer");
                wait(|| buffer.char_count() as usize == content.chars().count());
                assert_eq!(buffer.language().expect("JavaScript language").id(), "js");
                assert!(buffer.is_highlight_syntax(), "{case}");
                buffer.ensure_highlight(&buffer.start_iter(), &buffer.end_iter());
                assert_eq!(
                    buffer.text(&buffer.start_iter(), &buffer.end_iter(), true),
                    content
                );
                for marker in ["highlighted-head", "highlighted-tail"] {
                    let offset = content.find(marker).expect("fixture string") as i32;
                    let iter = buffer.iter_at_offset(offset);
                    assert!(
                        buffer.iter_has_context_class(&iter, "string"),
                        "{case}: {marker}"
                    );
                    assert!(
                        iter.tags()
                            .iter()
                            .any(|tag| tag.foreground_rgba() == Some(string_color)),
                        "{case}: {marker} must use the theme string color"
                    );
                }
            }
        },
    );
}

#[test]
fn replacing_large_source_cancels_pending_inserts_and_plain_text_stays_virtualized() {
    gtk_test(
        "ui::preview::tests::source::replacing_large_source_cancels_pending_inserts_and_plain_text_stays_virtualized",
        || {
            let preview = SourcePreviewView::new();
            let content = "const message = \"queued\";\n".repeat(600);
            let (_first, virtualized) = preview.show(
                &source_entry("first.js"),
                "text/javascript",
                &content,
                false,
            );
            assert!(!virtualized);
            let previous = preview
                .view
                .buffer()
                .downcast::<sourceview5::Buffer>()
                .expect("first source buffer");
            let loaded = previous.char_count();
            assert!(
                loaded < content.chars().count() as i32,
                "fixture must exercise pending insertion"
            );
            let replacement = "const message = \"replacement\";";
            let (_second, _) = preview.show(
                &source_entry("second.js"),
                "text/javascript",
                replacement,
                false,
            );
            while glib::MainContext::default().pending() {
                glib::MainContext::default().iteration(false);
            }
            assert_eq!(
                previous.char_count(),
                loaded,
                "stale source must stop loading"
            );
            let current = preview
                .view
                .buffer()
                .downcast::<sourceview5::Buffer>()
                .expect("replacement source buffer");
            assert_eq!(
                current.text(&current.start_iter(), &current.end_iter(), true),
                replacement
            );
            assert!(current.is_highlight_syntax());
            let (_plain, virtualized) = preview.show(
                &source_entry("plain.txt"),
                "text/plain",
                &"plain\n".repeat(600),
                false,
            );
            assert!(
                virtualized,
                "unknown-language text should retain virtualized rendering"
            );
        },
    );
}
