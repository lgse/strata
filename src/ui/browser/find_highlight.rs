// SPDX-License-Identifier: MIT

use crate::services::fold_for_search;
use gtk::prelude::*;

/// First matching substring of `name` for a `/` `?` query, in UTF-8 bytes.
pub(in crate::ui) fn match_bytes(name: &str, query: &str) -> Option<(u32, u32)> {
    let folded_query = fold_for_search(query.trim());
    if folded_query.is_empty() || !fold_for_search(name).contains(&folded_query) {
        return None;
    }
    let bounds: Vec<usize> = name
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(name.len()))
        .collect();
    for (start_index, start) in bounds.iter().enumerate() {
        for end in bounds.iter().skip(start_index + 1) {
            if fold_for_search(&name[*start..*end]) == folded_query {
                return Some((*start as u32, *end as u32));
            }
        }
    }
    None
}

pub(in crate::ui) fn apply_to_label(label: &gtk::Label, query: &str) {
    if skip_empty_apply(query, label.attributes().is_some()) {
        return;
    }
    apply_attributes(label.upcast_ref(), label.text().as_str(), query, |attrs| {
        label.set_attributes(attrs);
    });
}

pub(in crate::ui) fn apply_to_inscription(label: &gtk::Inscription, query: &str) {
    if skip_empty_apply(query, label.attributes().is_some()) {
        return;
    }
    let text = label.text().unwrap_or_default();
    // Inscription only attaches a new AttrList after the text is re-set.
    label.set_text(Some(&text));
    apply_attributes(label.upcast_ref(), &text, query, |attrs| {
        label.set_attributes(attrs);
    });
    label.queue_draw();
    if !fold_for_search(query.trim()).is_empty()
        && let Some(parent) = label.parent()
    {
        parent.queue_resize();
    }
}

pub(in crate::ui) fn apply_to_widget(widget: &gtk::Widget, query: &str) {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        apply_to_label(label, query);
    } else if let Some(label) = widget.downcast_ref::<gtk::Inscription>() {
        apply_to_inscription(label, query);
    }
}

fn skip_empty_apply(query: &str, has_attributes: bool) -> bool {
    !has_attributes && fold_for_search(query.trim()).is_empty()
}

fn apply_attributes(
    widget: &gtk::Widget,
    text: &str,
    query: &str,
    set: impl FnOnce(Option<&gtk::pango::AttrList>),
) {
    let Some((start, end)) = match_bytes(text, query) else {
        set(None);
        return;
    };
    let Some(attrs) = highlight_attributes(widget, text, start, end) else {
        set(None);
        return;
    };
    set(Some(&attrs));
}

fn highlight_attributes(
    widget: &gtk::Widget,
    text: &str,
    start: u32,
    end: u32,
) -> Option<gtk::pango::AttrList> {
    if start >= end || end as usize > text.len() {
        return None;
    }
    let attrs = gtk::pango::AttrList::new();
    let mut underline = gtk::pango::AttrInt::new_underline(gtk::pango::Underline::Single);
    underline.set_start_index(start);
    underline.set_end_index(end);
    attrs.insert(underline);
    if let (Some(background), Some(foreground)) = (
        theme_rgb(widget, &["theme_accent"]),
        theme_rgb(widget, &["theme_surface", "theme_bg"]),
    ) {
        for (index, rgb) in [background, foreground].iter().enumerate() {
            let mut attr = if index == 0 {
                gtk::pango::AttrColor::new_background(rgb[0], rgb[1], rgb[2])
            } else {
                gtk::pango::AttrColor::new_foreground(rgb[0], rgb[1], rgb[2])
            };
            attr.set_start_index(start);
            attr.set_end_index(end);
            attrs.insert(attr);
        }
    }
    Some(attrs)
}

fn theme_rgb(widget: &gtk::Widget, names: &[&str]) -> Option<[u16; 3]> {
    #[expect(
        deprecated,
        reason = "GTK exposes named theme colors through StyleContext"
    )]
    let context = widget.style_context();
    for name in names {
        #[expect(
            deprecated,
            reason = "GTK exposes named theme colors through StyleContext"
        )]
        if let Some(color) = context.lookup_color(name) {
            return Some([
                (color.red() * 65535.0) as u16,
                (color.green() * 65535.0) as u16,
                (color.blue() * 65535.0) as u16,
            ]);
        }
    }
    None
}

#[cfg(test)]
pub(in crate::ui) mod tests;
