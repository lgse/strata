// SPDX-License-Identifier: MIT

use super::{apply_to_inscription, apply_to_label, match_bytes};
use crate::test_support::gtk_test;
use crate::ui::theme::ThemeManager;
use gtk::prelude::*;

pub(in crate::ui) fn highlighted_slice(
    text: &str,
    attrs: Option<&gtk::pango::AttrList>,
) -> Option<String> {
    let mut iter = attrs?.iterator();
    loop {
        if let Some(attr) = iter
            .get(gtk::pango::AttrType::Background)
            .or_else(|| iter.get(gtk::pango::AttrType::Underline))
        {
            let start = attr.start_index() as usize;
            let end = (attr.end_index() as usize).min(text.len());
            return text
                .get(start..end)
                .filter(|slice| !slice.is_empty())
                .map(str::to_owned);
        }
        if !iter.next_style_change() {
            return None;
        }
    }
}

#[test]
fn match_bytes_finds_the_folded_substring() {
    assert_eq!(match_bytes("b.txt", "b"), Some((0, 1)));
    assert_eq!(match_bytes("b.txt", "txt"), Some((2, 5)));
    assert_eq!(match_bytes("b.txt", "B.TXT"), Some((0, 5)));
    assert_eq!(match_bytes("FILE.TXT", "file"), Some((0, 4)));
    assert_eq!(match_bytes("Café.txt", "café"), Some((0, 5)));
    assert_eq!(
        match_bytes("e\u{0301}.txt", "é"),
        Some((0, "e\u{0301}".len() as u32))
    );
}

#[test]
fn match_bytes_skips_empty_queries_and_misses() {
    assert_eq!(match_bytes("b.txt", ""), None);
    assert_eq!(match_bytes("b.txt", "   "), None);
    assert_eq!(match_bytes("sub", "txt"), None);
    assert_eq!(match_bytes("", "a"), None);
}

#[test]
fn label_highlight_covers_the_match_and_clears() {
    gtk_test(
        "ui::browser::find_highlight::tests::label_highlight_covers_the_match_and_clears",
        || {
            ThemeManager::shared();
            let label = gtk::Label::new(Some("b.txt"));
            let window = gtk::Window::builder().child(&label).build();
            window.present();
            apply_to_label(&label, "txt");
            assert_eq!(
                highlighted_slice(label.text().as_str(), label.attributes().as_ref()).as_deref(),
                Some("txt")
            );
            apply_to_label(&label, "");
            assert_eq!(
                highlighted_slice(label.text().as_str(), label.attributes().as_ref()),
                None
            );
            apply_to_label(&label, "missing");
            assert_eq!(
                highlighted_slice(label.text().as_str(), label.attributes().as_ref()),
                None
            );
            window.destroy();
        },
    );
}

#[test]
fn inscription_highlight_covers_the_match() {
    gtk_test(
        "ui::browser::find_highlight::tests::inscription_highlight_covers_the_match",
        || {
            ThemeManager::shared();
            let label = gtk::Inscription::new(Some("Notes.TXT"));
            let window = gtk::Window::builder().child(&label).build();
            window.present();
            apply_to_inscription(&label, "txt");
            assert_eq!(
                highlighted_slice(
                    label.text().as_deref().unwrap_or(""),
                    label.attributes().as_ref()
                )
                .as_deref(),
                Some("TXT")
            );
            apply_to_inscription(&label, "nope");
            assert_eq!(
                highlighted_slice(
                    label.text().as_deref().unwrap_or(""),
                    label.attributes().as_ref()
                ),
                None
            );
            window.destroy();
        },
    );
}
