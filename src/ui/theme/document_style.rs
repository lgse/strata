// SPDX-License-Identifier: MIT

//! Theme colors for rendered documents: their text tags and the view's
//! selection and code-block chrome.

use std::cell::RefCell;

use gtk::{gdk, prelude::*};

use super::{
    super::document_view::DocumentTextView,
    ThemeManager, ThemeTokens, blend,
    weak_registry::{WeakRegistry, for_each_live, register},
};

thread_local! {
    static DOCUMENT_BUFFERS: WeakRegistry<gtk::TextBuffer> = const { RefCell::new(Vec::new()) };
    static DOCUMENT_VIEWS: WeakRegistry<DocumentTextView> = const { RefCell::new(Vec::new()) };
}

pub(in crate::ui) fn register_document_buffer(buffer: &gtk::TextBuffer) {
    let tokens = ThemeManager::shared().appearance_tokens();
    style_document_buffer(buffer, &tokens);
    DOCUMENT_BUFFERS.with(|buffers| register(buffers, buffer));
}

pub(in crate::ui) fn register_document_view(view: &DocumentTextView) {
    let tokens = ThemeManager::shared().appearance_tokens();
    style_document_view(view, &tokens);
    DOCUMENT_VIEWS.with(|views| register(views, view));
}

pub(super) fn style_document_buffers(tokens: &ThemeTokens) {
    DOCUMENT_BUFFERS.with(|buffers| {
        for_each_live(buffers, |buffer| style_document_buffer(buffer, tokens));
    });
}

pub(super) fn style_document_views(tokens: &ThemeTokens) {
    DOCUMENT_VIEWS.with(|views| for_each_live(views, |view| style_document_view(view, tokens)));
}

fn style_document_view(view: &DocumentTextView, tokens: &ThemeTokens) {
    if let (Ok(fill), Ok(border), Ok(selection)) = (
        gdk::RGBA::parse(blend(&tokens.surface, &tokens.muted, 0.54)),
        gdk::RGBA::parse(blend(&tokens.surface, &tokens.border, 0.5)),
        gdk::RGBA::parse(&tokens.accent),
    ) {
        view.set_colors(fill, border, selection);
    }
}

fn parse(value: &str) -> Option<gdk::RGBA> {
    gdk::RGBA::parse(value).ok()
}

fn style_document_buffer(buffer: &gtk::TextBuffer, tokens: &ThemeTokens) {
    let table = buffer.tag_table();
    let foregrounds: [(&str, &[&str]); 4] = [
        (&tokens.accent, &["document-accent", "document-link"]),
        (&tokens.dim_text, &["document-dim"]),
        (&tokens.text, &["document-link-hover"]),
        (&tokens.background, &["document-selection"]),
    ];
    for (value, names) in foregrounds {
        set_tags_foreground(&table, value, names);
    }
    if let Some(color) = parse(&tokens.highlight) {
        style_highlight_tags(&table, color);
    }
}

fn set_tags_foreground(table: &gtk::TextTagTable, value: &str, names: &[&str]) {
    let Some(color) = parse(value) else {
        return;
    };
    for name in names {
        if let Some(tag) = table.lookup(name) {
            tag.set_foreground_rgba(Some(&color));
        }
    }
}

fn style_highlight_tags(table: &gtk::TextTagTable, mut color: gdk::RGBA) {
    color.set_alpha(0.36);
    if let Some(tag) = table.lookup("document-quote") {
        tag.set_paragraph_background_rgba(Some(&color));
    }
    color.set_alpha(0.5);
    if let Some(tag) = table.lookup("document-link-hover") {
        tag.set_background_rgba(Some(&color));
    }
}
