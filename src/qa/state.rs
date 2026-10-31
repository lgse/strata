// SPDX-License-Identifier: MIT

//! Reads the application's state from its own widget tree, as AT-SPI would expose it.

use std::{collections::HashMap, fmt::Write as _};

use gtk::{graphene, prelude::*};
use serde::Serialize;
use serde_json::{Value, json};

use super::{
    protocol::{Check, Failure, FieldCheck, PointAxis, Predicate, Presence},
    targets,
};
use crate::ui::accessibility_qa::{self as names, Mark};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Observation {
    pub(crate) panes: Vec<PaneObservation>,
    pub(crate) focused_entry: Option<String>,
    pub(crate) focused_pane: Option<usize>,
    pub(crate) window_width: Option<i32>,
    pub(crate) dialog: Option<String>,
    pub(crate) menu: Option<Vec<String>>,
    pub(crate) preview: Option<Vec<String>>,
    pub(crate) peek: bool,
    pub(crate) results: Option<Vec<String>>,
    pub(crate) field: Option<FieldState>,
}

/// Offsets count characters, as AT-SPI's text interface does.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct FieldState {
    pub(crate) name: String,
    pub(crate) text: String,
    pub(crate) caret: i32,
    pub(crate) selection: Option<[i32; 2]>,
    pub(crate) focused: bool,
    #[serde(skip)]
    pub(crate) showing: bool,
}

impl FieldState {
    pub(crate) fn read(widget: &gtk::Widget) -> Option<Self> {
        let editable = widget.dynamic_cast_ref::<gtk::Editable>()?;
        Some(Self {
            name: targets::accessible_name(widget),
            text: editable.text().to_string(),
            caret: editable.position(),
            selection: editable
                .selection_bounds()
                .filter(|(start, end)| start != end)
                .map(|(start, end)| [start.min(end), start.max(end)]),
            focused: targets::editable_focused(widget),
            showing: widget.is_mapped(),
        })
    }
}

pub(crate) fn field_matches(check: &FieldCheck, field: Option<&FieldState>) -> bool {
    let Some(field) = field else {
        return false;
    };
    check
        .text
        .as_ref()
        .is_none_or(|text| text.matches(&field.text))
        && check
            .caret
            .as_ref()
            .is_none_or(|caret| caret.matches(field.caret))
        && check
            .selection
            .is_none_or(|selection| selection == field.selection)
        && check.focused.is_none_or(|focused| focused == field.focused)
        && check.showing.is_none_or(|showing| showing == field.showing)
}

pub(crate) fn within_viewport(editor: [i64; 4], pane_x: i64, window_width: i64) -> bool {
    let [x, _, width, height] = editor;
    height > 0 && pane_x <= x && x < window_width && 0 < width && width <= window_width - pane_x
}

pub(crate) fn within_window(editor: [i64; 4], pane_x: i64, window_width: i64) -> bool {
    let [x, _, width, _] = editor;
    width > 1 && x >= pane_x.max(0) && x + width <= window_width
}

pub(crate) fn inside(inner: [i64; 4], outer: [i64; 4], axis: Option<PointAxis>) -> bool {
    let [x, y, width, height] = inner;
    let [outer_x, outer_y, outer_width, outer_height] = outer;
    let across = x >= outer_x && x + width <= outer_x + outer_width;
    let down = y >= outer_y && y + height <= outer_y + outer_height;
    width > 0
        && height > 0
        && match axis {
            None => across && down,
            Some(PointAxis::X) => across,
            Some(PointAxis::Y) => down,
        }
}

pub(crate) fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn on_screen(bounds: [i64; 4], window: [i64; 4]) -> bool {
    let [_, y, _, height] = bounds;
    height > 0 && y >= window[1] && y + height <= window[1] + window[3]
}

pub(crate) fn contains(bounds: [i64; 4], point: (i64, i64)) -> bool {
    let [x, y, width, height] = bounds;
    x <= point.0 && point.0 < x + width && y <= point.1 && point.1 < y + height
}

pub(crate) fn middle_visible(
    entries: &[(String, [i64; 4])],
    viewport: [i64; 4],
    unless_first: &str,
) -> Option<String> {
    let [_, top, _, height] = viewport;
    let visible: Vec<&String> = entries
        .iter()
        .filter(|(_, bounds)| top < bounds[1] && bounds[1] < top + height - bounds[3])
        .map(|(name, _)| name)
        .collect();
    visible
        .first()
        .filter(|first| first.as_str() != unless_first)
        .map(|_| visible[visible.len() / 2].clone())
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PaneObservation {
    pub(crate) name: String,
    pub(crate) view: &'static str,
    pub(crate) entries: Vec<EntryObservation>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EntryObservation {
    pub(crate) name: String,
    pub(crate) selected: bool,
    pub(crate) focused: bool,
}

/// The `app` snapshot every reply carries; `.agents/skills/qa-scenarios/reference.md` defines its shape.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct AppState {
    pub(crate) view_mode: Option<String>,
    pub(crate) directory: Option<String>,
    pub(crate) panes: Vec<PaneState>,
    pub(crate) focused: Option<String>,
    pub(crate) focused_pane: Option<String>,
    pub(crate) dialog: Option<String>,
    pub(crate) menu: Option<Vec<String>>,
    pub(crate) preview: Option<Vec<String>>,
    pub(crate) field: Option<FieldState>,
    pub(crate) matches: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct PaneState {
    pub(crate) name: String,
    pub(crate) selected: Vec<String>,
    pub(crate) focused: Option<String>,
}

pub(crate) fn empty_app_state() -> serde_json::Value {
    serde_json::json!({
        "view_mode": null,
        "directory": null,
        "panes": [],
        "focused": null,
        "focused_pane": null,
        "dialog": null,
        "menu": null,
        "preview": null,
        "field": null,
        "matches": [],
    })
}

impl Observation {
    /// Columns keeps parent panes on screen, so the focused pane is the current one.
    pub(crate) fn directory(&self) -> Option<&str> {
        self.focused_pane
            .and_then(|index| self.panes.get(index))
            .or_else(|| self.panes.last())
            .map(|pane| pane.name.as_str())
    }

    pub(crate) fn view_mode(&self) -> Option<&'static str> {
        self.panes.first().map(|pane| pane.view)
    }

    /// The last match wins.
    pub(crate) fn pane(&self, directory: Option<&str>) -> Option<&PaneObservation> {
        match directory {
            None => self.panes.last(),
            Some(directory) => self.panes.iter().rev().find(|pane| pane.name == directory),
        }
    }

    pub(crate) fn entry_names(&self, directory: Option<&str>) -> Vec<String> {
        self.pane(directory).map_or_else(Vec::new, |pane| {
            pane.entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect()
        })
    }

    pub(crate) fn entry(&self, name: &str, directory: Option<&str>) -> Option<&EntryObservation> {
        let panes: Vec<&PaneObservation> = match directory {
            Some(_) => self.pane(directory).into_iter().collect(),
            None => self.panes.iter().collect(),
        };
        panes
            .into_iter()
            .flat_map(|pane| &pane.entries)
            .find(|entry| entry.name == name)
    }

    pub(crate) fn entry_selected(&self, name: &str, directory: Option<&str>) -> bool {
        self.entry(name, directory)
            .is_some_and(|entry| entry.selected)
    }

    pub(crate) fn matches(&self, directory: Option<&str>) -> Vec<String> {
        self.results
            .clone()
            .unwrap_or_else(|| self.entry_names(directory))
    }

    pub(crate) fn focused_pane_name(&self) -> Option<&str> {
        self.focused_pane
            .and_then(|index| self.panes.get(index))
            .map(|pane| pane.name.as_str())
    }

    pub(crate) fn all_selected(&self) -> Vec<String> {
        self.panes
            .iter()
            .flat_map(|pane| &pane.entries)
            .filter(|entry| entry.selected)
            .map(|entry| entry.name.clone())
            .collect()
    }

    pub(crate) fn selected_names(&self, directory: Option<&str>) -> Vec<String> {
        self.pane(directory).map_or_else(Vec::new, |pane| {
            pane.entries
                .iter()
                .filter(|entry| entry.selected)
                .map(|entry| entry.name.clone())
                .collect()
        })
    }

    pub(crate) fn app_state(&self) -> AppState {
        AppState {
            view_mode: self.view_mode().map(str::to_owned),
            directory: self.directory().map(str::to_owned),
            panes: self
                .panes
                .iter()
                .map(|pane| PaneState {
                    name: pane.name.clone(),
                    selected: pane
                        .entries
                        .iter()
                        .filter(|entry| entry.selected)
                        .map(|entry| entry.name.clone())
                        .collect(),
                    focused: pane
                        .entries
                        .iter()
                        .find(|entry| entry.focused)
                        .map(|entry| entry.name.clone()),
                })
                .collect(),
            focused: self.focused_entry.clone(),
            focused_pane: self.focused_pane_name().map(str::to_owned),
            dialog: self.dialog.clone(),
            menu: self.menu.clone(),
            preview: self.preview.clone(),
            field: self.field.clone(),
            matches: self.matches(None),
        }
    }

    pub(crate) fn evaluate(&self, predicate: &Predicate) -> Result<(), Failure> {
        let directory = predicate.dir.as_deref();
        let (holds, actual) = match &predicate.check {
            Check::Directory(expected) => {
                let actual = self.directory();
                (expected.as_deref() == actual, json!(actual))
            }
            Check::Panes(expected) => {
                let actual: Vec<String> = self.panes.iter().map(|pane| pane.name.clone()).collect();
                (expected.matches(&actual), json!(actual))
            }
            Check::Pane(expected) => {
                let actual = self.panes.last().map(|pane| pane.name.as_str());
                (expected.as_deref() == actual, json!(actual))
            }
            Check::Entry(name) => {
                let listed = self.entry(name, directory).is_some();
                (listed, json!(self.entry_names(directory)))
            }
            Check::EntryGone(name) => {
                let listed = self.entry(name, directory).is_some();
                (!listed, json!(self.entry_names(directory)))
            }
            Check::WindowWidth(expected) => (
                Some(*expected) == self.window_width,
                json!(self.window_width),
            ),
            // The controller resolves these against live widgets and files.
            Check::Present(_)
            | Check::Absent(_)
            | Check::Moved(_)
            | Check::Field(_)
            | Check::Pref(_)
            | Check::EditorWithinViewport(_)
            | Check::State(_)
            | Check::OnScreen(_)
            | Check::Blank(_)
            | Check::Visible(_)
            | Check::ScrollValue(_)
            | Check::BandSelected(_)
            | Check::FocusWithin(_)
            | Check::Resized(_)
            | Check::ContainsPoint(_)
            | Check::FooterFill(_)
            | Check::Repainted(_)
            | Check::ContrastBelow(_)
            | Check::Elapsed(_)
            | Check::DiagnosticsMention(_)
            | Check::EditableFields(_)
            | Check::Description(_)
            | Check::Mentions(_)
            | Check::Presented(_)
            | Check::File(_)
            | Check::FocusedWidget(_)
            | Check::Inside(_)
            | Check::EditorWithinWindow(_)
            | Check::Replaced(_)
            | Check::Pixel(_)
            | Check::AccentPixels(_)
            | Check::HitPath(_)
            | Check::AnyResultRow(_)
            | Check::Compare(_)
            | Check::PanesWith(_)
            | Check::Name(_)
            | Check::EditableText(_)
            | Check::VisibleTop(_)
            | Check::LargestImage(_)
            | Check::Children(_)
            | Check::ListSections(_)
            | Check::Text(_)
            | Check::SomeWindow(_)
            | Check::Windows(_)
            | Check::ChildNames(_)
            | Check::FocusOutside(_)
            | Check::FocusedWidgetIs(_)
            | Check::Alive(_)
            | Check::SubtreeText(_) => (false, json!("needs the live window")),
            Check::Not(inner) => match self.evaluate(inner) {
                Ok(()) => (false, json!("the negated predicate holds")),
                Err(failure) => (true, failure.actual),
            },
            Check::Any(predicates) => {
                let actual: Vec<Value> = predicates
                    .iter()
                    .map(|predicate| match self.evaluate(predicate) {
                        Ok(()) => json!(true),
                        Err(failure) => failure.actual,
                    })
                    .collect();
                (
                    predicates
                        .iter()
                        .any(|predicate| self.evaluate(predicate).is_ok()),
                    json!(actual),
                )
            }
            Check::All(predicates) => {
                let actual: Vec<Value> = predicates
                    .iter()
                    .map(|predicate| match self.evaluate(predicate) {
                        Ok(()) => json!(true),
                        Err(failure) => failure.actual,
                    })
                    .collect();
                (
                    predicates
                        .iter()
                        .all(|predicate| self.evaluate(predicate).is_ok()),
                    json!(actual),
                )
            }
            Check::FocusedPane(expected) => {
                let actual = self.focused_pane_name();
                (expected.matches(actual), json!(actual))
            }
            Check::Dialog(expected) => {
                (expected.matches(self.dialog.as_deref()), json!(self.dialog))
            }
            Check::MenuItems(expected) => (
                match (expected, &self.menu) {
                    (None, None) => true,
                    (Some(expected), Some(items)) => expected.matches(items),
                    _ => false,
                },
                json!(self.menu),
            ),
            Check::Preview(expected) => (
                match (expected, &self.preview) {
                    (Presence::Absent, None) => true,
                    (Presence::Any, Some(_)) => true,
                    (Presence::Named(text), Some(shown)) => {
                        shown.iter().any(|shown| shown.contains(text.as_str()))
                    }
                    (Presence::OneOf(texts), Some(shown)) => texts
                        .iter()
                        .any(|text| shown.iter().any(|shown| shown.contains(text.as_str()))),
                    _ => false,
                },
                json!(self.preview),
            ),
            Check::Peek(expected) => (*expected == self.peek, json!(self.peek)),
            Check::Matches(expected) => {
                let actual = self.matches(directory);
                (expected.matches(&actual), json!(actual))
            }
            Check::Entries(expected) => {
                let actual = self.entry_names(directory);
                (expected.matches(&actual), json!(actual))
            }
            Check::Selected(expected) => {
                let actual = self.selected_names(directory);
                (expected.matches(&actual), json!(actual))
            }
            Check::AllSelected(expected) => {
                let actual = self.all_selected();
                (expected.matches(&actual), json!(actual))
            }
            Check::Focused(expected) => (
                expected.matches(self.focused_entry.as_deref()),
                json!(self.focused_entry),
            ),
            Check::FocusedInMatches(expected) => {
                let matches = self.matches(None);
                let among = self
                    .focused_entry
                    .as_ref()
                    .is_some_and(|focused| matches.contains(focused));
                (
                    among == *expected,
                    json!({ "focused": self.focused_entry, "matches": matches }),
                )
            }
            Check::ViewMode(expected) => {
                let actual = self.view_mode();
                (Some(expected.as_str()) == actual, json!(actual))
            }
        };
        if holds {
            return Ok(());
        }
        Err(Failure {
            predicate: serde_json::to_value(predicate).unwrap_or(Value::Null),
            actual,
            message: None,
        })
    }
}

pub(crate) struct Scene {
    pub(crate) observation: Observation,
    widgets: Vec<PaneWidgets>,
}

pub(crate) struct PaneWidgets {
    pub(crate) pane: gtk::Widget,
    pub(crate) container: Option<gtk::Widget>,
    pub(crate) entries: Vec<gtk::Widget>,
}

impl Scene {
    pub(crate) fn entry_widget(&self, name: &str, directory: Option<&str>) -> Option<&gtk::Widget> {
        let panes = &self.observation.panes;
        let candidates: Vec<usize> = match directory {
            Some(_) => self.pane_index(directory).into_iter().collect(),
            None => (0..panes.len()).collect(),
        };
        candidates.into_iter().find_map(|pane| {
            let index = panes[pane]
                .entries
                .iter()
                .position(|entry| entry.name == name)?;
            self.widgets[pane].entries.get(index)
        })
    }

    pub(crate) fn pane_index(&self, directory: Option<&str>) -> Option<usize> {
        let panes = &self.observation.panes;
        match directory {
            None => panes.len().checked_sub(1),
            Some(directory) => panes.iter().rposition(|pane| pane.name == directory),
        }
    }

    pub(crate) fn pane_widgets(&self, directory: Option<&str>) -> Option<&PaneWidgets> {
        self.pane_index(directory)
            .and_then(|index| self.widgets.get(index))
    }
    pub(crate) fn pane_widgets_at(&self, index: usize) -> Option<&PaneWidgets> {
        self.widgets.get(index)
    }
}

pub(crate) fn rendered_bounds(widget: &gtk::Widget) -> Option<graphene::Rect> {
    if !widget.is_mapped() || !widget.is_visible() {
        return None;
    }
    let native = widget.native()?;
    let bounds = widget.compute_bounds(&native)?;
    let sane = |extent: f32| extent > 0.0 && extent < 100_000.0;
    (sane(bounds.width()) && sane(bounds.height())).then_some(bounds)
}

fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(widget) = stack.pop() {
        let mut children = Vec::new();
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            children.push(current);
        }
        stack.extend(children.into_iter().rev());
        found.push(widget);
    }
    found
}

fn address(widget: &gtk::Widget) -> usize {
    widget.as_ptr() as usize
}

struct Row {
    item: gtk::ListItem,
    name: String,
}

/// The row widget carrying a list item's focus and selected states is the parent of its child.
fn rows() -> HashMap<usize, Row> {
    names::entries()
        .into_iter()
        .filter_map(|(item, naming)| {
            let row = item.child()?.parent()?;
            let name = naming.label.filter(|name| !name.is_empty())?;
            Some((address(&row), Row { item, name }))
        })
        .collect()
}

pub(crate) fn row_name(widget: &gtk::Widget) -> Option<String> {
    names::entries().into_iter().find_map(|(item, naming)| {
        let row = item.child()?.parent()?;
        (row == *widget)
            .then_some(naming.label)
            .flatten()
            .filter(|name| !name.is_empty())
    })
}

pub(crate) fn row_description(widget: &gtk::Widget) -> Option<String> {
    names::entries().into_iter().find_map(|(item, naming)| {
        let row = item.child()?.parent()?;
        (row == *widget).then_some(naming.description).flatten()
    })
}

pub(crate) fn row_selected(widget: &gtk::Widget) -> bool {
    names::entries().into_iter().any(|(item, _)| {
        item.child()
            .and_then(|child| child.parent())
            .is_some_and(|row| row == *widget)
            && item.is_selected()
    })
}

pub(crate) fn scan(window: &gtk::Window) -> Scene {
    let rows = rows();
    // AT-SPI reports no focus for a focused row a list keeps unmapped while
    // it is scrolled away, so neither does `focused_pane()`.
    let focus = gtk::prelude::GtkWindowExt::focus(window)
        .filter(|focus| focus.has_focus() && focus.is_mapped());
    let root = window.upcast_ref::<gtk::Widget>();
    let all = descendants(root);

    let mut panes: Vec<(f32, PaneObservation, PaneWidgets, bool)> = Vec::new();
    for widget in &all {
        let Some(naming) = names::naming(widget) else {
            continue;
        };
        let Some(Mark::Pane(view)) = naming.mark else {
            continue;
        };
        let Some(bounds) = rendered_bounds(widget) else {
            continue;
        };
        let (entries, widgets) = pane_entries(widget, &rows);
        let holds_focus = focus
            .as_ref()
            .is_some_and(|focus| focus == widget || focus.is_ancestor(widget));
        panes.push((
            bounds.x(),
            PaneObservation {
                name: naming.label.unwrap_or_default(),
                view,
                entries,
            },
            widgets,
            holds_focus,
        ));
    }
    panes.sort_by(|left, right| left.0.total_cmp(&right.0));

    let focused_entry = all.iter().find_map(|widget| {
        let row = rows.get(&address(widget))?;
        (widget.has_focus() && rendered_bounds(widget).is_some()).then(|| row.name.clone())
    });
    let focused_pane = panes.iter().rposition(|pane| pane.3);
    let (observed, widgets) = panes
        .into_iter()
        .map(|(_, pane, widgets, _)| (pane, widgets))
        .unzip();
    let presented = targets::presented(root);
    Scene {
        observation: Observation {
            panes: observed,
            focused_entry,
            focused_pane,
            window_width: Some(window.width()),
            dialog: targets::open_dialog(&presented)
                .map(|dialog| targets::accessible_name(&dialog)),
            menu: open_menu(&presented),
            preview: open_preview(&presented),
            peek: presented.iter().any(|widget| {
                rendered_bounds(widget).is_some()
                    && targets::accessible_name(widget) == crate::i18n::tr(super::PEEK_LABEL)
            }),
            results: search_results(&presented),
            field: targets::focused_field(&presented).and_then(|field| FieldState::read(&field)),
        },
        widgets,
    }
}

fn rendered_with_role<'a>(
    presented: &'a [gtk::Widget],
    role: &'a str,
) -> impl Iterator<Item = &'a gtk::Widget> {
    presented.iter().filter(move |widget| {
        targets::role_name(widget) == role && rendered_bounds(widget).is_some()
    })
}

fn open_menu(presented: &[gtk::Widget]) -> Option<Vec<String>> {
    let menu = rendered_with_role(presented, "menu").next()?;
    let inside = targets::presented(menu);
    Some(
        rendered_with_role(&inside, "menu item")
            .map(targets::accessible_name)
            .collect(),
    )
}

fn open_preview(presented: &[gtk::Widget]) -> Option<Vec<String>> {
    let label = crate::i18n::tr(crate::ui::PREVIEW_LABEL);
    let drawer = presented.iter().find(|widget| {
        rendered_bounds(widget).is_some() && targets::accessible_name(widget) == label
    })?;
    let mut shown = Vec::new();
    for widget in targets::presented(drawer) {
        let name = targets::accessible_name(&widget);
        let text = targets::text_content(&widget);
        if !name.is_empty() {
            shown.push(name);
        }
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            shown.push(text);
        }
    }
    Some(shown)
}

fn search_results(presented: &[gtk::Widget]) -> Option<Vec<String>> {
    let (results, row_role) = targets::search_results(presented)?;
    let inside = targets::presented(&results);
    Some(
        rendered_with_role(&inside, row_role)
            .map(targets::accessible_name)
            .filter(|name| !name.is_empty())
            .collect(),
    )
}

fn pane_entries(
    pane: &gtk::Widget,
    rows: &HashMap<usize, Row>,
) -> (Vec<EntryObservation>, PaneWidgets) {
    let container = descendants(pane).into_iter().find(|widget| {
        names::naming(widget).is_some_and(|naming| naming.mark == Some(Mark::EntryContainer))
            && rendered_bounds(widget).is_some()
    });
    let mut widgets = PaneWidgets {
        pane: pane.clone(),
        container: container.clone(),
        entries: Vec::new(),
    };
    let Some(container) = container else {
        return (Vec::new(), widgets);
    };
    let mut found: Vec<(graphene::Rect, EntryObservation, gtk::Widget)> = Vec::new();
    let mut occupied: HashMap<[i64; 4], String> = HashMap::new();
    for widget in descendants(&container) {
        let Some(row) = rows.get(&address(&widget)) else {
            continue;
        };
        let Some(bounds) = rendered_bounds(&widget) else {
            continue;
        };
        if !row_label_matches(&widget, &row.name) {
            continue;
        }
        // GtkListView can give a recycled row its new name before its bounds
        // catch up, so two names share one box mid-refresh; `_entries_in`
        // then reports nothing rather than a wrong order.
        let key = [bounds.x(), bounds.y(), bounds.width(), bounds.height()]
            .map(|value| f64::from(value).floor() as i64);
        match occupied.get(&key) {
            Some(prior) if *prior != row.name => return (Vec::new(), widgets),
            _ => {
                occupied.insert(key, row.name.clone());
            }
        }
        let entry = EntryObservation {
            name: row.name.clone(),
            selected: row.item.is_selected(),
            focused: widget.has_focus(),
        };
        found.push((bounds, entry, widget));
    }
    found.sort_by(|left, right| {
        left.0
            .y()
            .total_cmp(&right.0.y())
            .then(left.0.x().total_cmp(&right.0.x()))
    });
    let (entries, rows): (Vec<EntryObservation>, Vec<gtk::Widget>) = found
        .into_iter()
        .map(|(_, entry, widget)| (entry, widget))
        .unzip();
    widgets.entries = rows;
    (entries, widgets)
}

/// False while a recycled row's label still shows the previous name.
fn row_label_matches(row: &gtk::Widget, name: &str) -> bool {
    let presented = targets::presented(row);
    // List metadata may precede the name; the thumbnail identifies its caption.
    let name_cell = presented
        .iter()
        .find(|widget| targets::role_name(widget) == "image")
        .and_then(WidgetExt::parent);
    let label = name_cell
        .and_then(|cell| {
            targets::presented(&cell)
                .into_iter()
                .find(|widget| targets::role_name(widget) == "label")
        })
        .or_else(|| {
            presented
                .into_iter()
                .find(|widget| targets::role_name(widget) == "label")
        });
    label.is_none_or(|label| targets::accessible_name(&label) == name)
}

pub(crate) fn is_ready(window: &gtk::Window) -> bool {
    window.is_mapped()
        && descendants(window.upcast_ref()).iter().any(|widget| {
            names::naming(widget).is_some_and(|naming| naming.mark == Some(Mark::EntryContainer))
                && rendered_bounds(widget).is_some()
        })
}

const MAX_TREE_DEPTH: usize = 64;

pub(crate) fn dump(window: &gtk::Window) -> String {
    let rows = rows();
    let mut output = String::new();
    if let Some(focus) = gtk::prelude::GtkWindowExt::focus(window) {
        let _ = writeln!(
            output,
            "focus: {} role={:?} name={:?}{}",
            focus.type_().name(),
            targets::role_name(&focus),
            targets::accessible_name(&focus),
            if focus.is_mapped() {
                ""
            } else {
                " (not mapped)"
            }
        );
    }
    dump_widget(window.upcast_ref(), 0, &rows, &mut output);
    output
}

fn dump_widget(
    widget: &gtk::Widget,
    depth: usize,
    rows: &HashMap<usize, Row>,
    output: &mut String,
) {
    if !widget.is_mapped() || depth > MAX_TREE_DEPTH {
        return;
    }
    let _ = write!(
        output,
        "{:indent$}{} role={:?}",
        "",
        widget.type_().name(),
        targets::role_name(widget),
        indent = depth * 2
    );
    let name = targets::accessible_name(widget);
    if !name.is_empty() {
        let _ = write!(output, " name={name:?}");
    }
    if rows
        .get(&address(widget))
        .is_some_and(|row| row.item.is_selected())
    {
        output.push_str(" [selected]");
    }
    if let Some(description) = names::naming(widget)
        .and_then(|naming| naming.description)
        .filter(|text| !text.is_empty())
    {
        let _ = write!(output, " description={description:?}");
    }
    if let Some(text) = widget_text(widget).filter(|text| *text != name) {
        let _ = write!(output, " text={text:?}");
    }
    if widget.has_focus() {
        output.push_str(" [focused]");
    }
    if let Some(bounds) = rendered_bounds(widget) {
        let _ = write!(
            output,
            " ({:.0},{:.0} {:.0}x{:.0})",
            bounds.x(),
            bounds.y(),
            bounds.width(),
            bounds.height()
        );
    }
    output.push('\n');
    let mut child = widget.first_child();
    while let Some(current) = child {
        dump_widget(&current, depth + 1, rows, output);
        child = current.next_sibling();
    }
}

fn widget_text(widget: &gtk::Widget) -> Option<String> {
    let text = if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        label.label().to_string()
    } else if let Some(inscription) = widget.downcast_ref::<gtk::Inscription>() {
        inscription
            .text()
            .map(|text| text.to_string())
            .unwrap_or_default()
    } else {
        widget.downcast_ref::<gtk::Text>()?.text().to_string()
    };
    (!text.is_empty()).then_some(text)
}
