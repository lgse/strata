// SPDX-License-Identifier: MIT

use gtk::prelude::*;
use std::{cell::RefCell, rc::Rc};

use super::{matching::Matches, targets::TARGETS};
use crate::ui::settings::keyboard;

pub(in crate::ui::settings) type State = Rc<RefCell<Option<Matches>>>;

pub(in crate::ui::settings) fn apply(widget: &impl IsA<gtk::Widget>, state: &State) {
    filter_tree(widget.upcast_ref(), state.borrow().as_ref());
}

pub(super) fn children(widget: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut children = Vec::new();
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        children.push(widget);
    }
    children
}

fn target_id(widget: &gtk::Widget) -> Option<String> {
    let name = widget.widget_name();
    let id = name.strip_prefix("settings-search-")?;
    let known = TARGETS.iter().any(|target| target.id == id) || keyboard::is_target(id);
    known.then(|| id.to_owned())
}

/// Whether the subtree shows a matching row, or `None` when it holds no tagged row.
fn filter_tree(widget: &gtk::Widget, matches: Option<&Matches>) -> Option<bool> {
    if let Some(id) = target_id(widget) {
        return Some(show_target(widget, &id, matches));
    }
    let children = children(widget);
    let results = children
        .iter()
        .map(|child| filter_tree(child, matches))
        .collect::<Vec<_>>();
    if !results.iter().any(Option::is_some) {
        return None;
    }
    let visible = results.contains(&Some(true));
    if widget.has_css_class("settings-group") {
        widget.set_visible(visible);
    }
    if widget.has_css_class("settings-preferences")
        || widget.has_css_class("settings-search-section")
    {
        show_section_chrome(&children, &results, matches.is_some() && !visible);
    }
    Some(visible)
}

fn show_target(widget: &gtk::Widget, id: &str, matches: Option<&Matches>) -> bool {
    let visible = !widget.has_css_class("settings-search-unavailable")
        && matches.is_none_or(|matches| matches.ids.contains(id));
    widget.set_visible(visible);
    visible
}

/// Shows the placeholder of an empty page, and each heading or description
/// only while a tagged row under it is shown.
fn show_section_chrome(children: &[gtk::Widget], results: &[Option<bool>], empty: bool) {
    for (index, child) in children.iter().enumerate() {
        if child.has_css_class("settings-search-page-empty") {
            child.set_visible(empty);
        }
        if child.has_css_class("menu-heading")
            || child.has_css_class("settings-section-description")
        {
            child.set_visible(heading_shown(&children[index + 1..], &results[index + 1..]));
        }
    }
}

/// A heading stays while a tagged row before the next heading is shown, or when none is tagged.
fn heading_shown(siblings: &[gtk::Widget], results: &[Option<bool>]) -> bool {
    let tagged = siblings
        .iter()
        .zip(results)
        .take_while(|(sibling, _)| !sibling.has_css_class("menu-heading"))
        .filter_map(|(_, result)| *result)
        .collect::<Vec<_>>();
    tagged.is_empty() || tagged.contains(&true)
}

pub(super) fn reset_scroll(widget: &gtk::Widget) {
    if widget.has_css_class("settings-content-scroll")
        && let Some(scroll) = widget.downcast_ref::<gtk::ScrolledWindow>()
    {
        scroll.vadjustment().set_value(0.0);
        return;
    }
    for child in children(widget) {
        reset_scroll(&child);
    }
}
