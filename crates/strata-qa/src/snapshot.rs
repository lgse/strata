// SPDX-License-Identifier: MIT

//! What the harness reports about the application, and the predicates that read it alone.

use serde::Serialize;
use serde_json::{Value, json};

use crate::{Check, Failure, FieldCheck, Predicate, Presence};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Observation {
    pub panes: Vec<PaneObservation>,
    pub focused_entry: Option<String>,
    pub focused_pane: Option<usize>,
    pub window_width: Option<i32>,
    pub dialog: Option<String>,
    pub menu: Option<Vec<String>>,
    pub preview: Option<Vec<String>>,
    pub peek: bool,
    pub results: Option<Vec<String>>,
    pub field: Option<FieldState>,
}

/// Offsets count characters, as AT-SPI's text interface does.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct FieldState {
    pub name: String,
    pub text: String,
    pub caret: i32,
    pub selection: Option<[i32; 2]>,
    pub focused: bool,
    #[serde(skip)]
    pub showing: bool,
}

pub fn field_matches(check: &FieldCheck, field: Option<&FieldState>) -> bool {
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

pub fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaneObservation {
    pub name: String,
    pub view: &'static str,
    pub entries: Vec<EntryObservation>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntryObservation {
    pub name: String,
    pub selected: bool,
    pub focused: bool,
}

/// The `app` snapshot every reply carries; `.agents/skills/qa-scenarios/reference.md` defines its shape.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AppState {
    pub view_mode: Option<String>,
    pub directory: Option<String>,
    pub panes: Vec<PaneState>,
    pub focused: Option<String>,
    pub focused_pane: Option<String>,
    pub dialog: Option<String>,
    pub menu: Option<Vec<String>>,
    pub preview: Option<Vec<String>>,
    pub field: Option<FieldState>,
    pub matches: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PaneState {
    pub name: String,
    pub selected: Vec<String>,
    pub focused: Option<String>,
}

pub fn empty_app_state() -> serde_json::Value {
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
    pub fn directory(&self) -> Option<&str> {
        self.focused_pane
            .and_then(|index| self.panes.get(index))
            .or_else(|| self.panes.last())
            .map(|pane| pane.name.as_str())
    }

    pub fn view_mode(&self) -> Option<&'static str> {
        self.panes.first().map(|pane| pane.view)
    }

    /// The last match wins.
    pub fn pane(&self, directory: Option<&str>) -> Option<&PaneObservation> {
        match directory {
            None => self.panes.last(),
            Some(directory) => self.panes.iter().rev().find(|pane| pane.name == directory),
        }
    }

    pub fn entry_names(&self, directory: Option<&str>) -> Vec<String> {
        self.pane(directory).map_or_else(Vec::new, |pane| {
            pane.entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect()
        })
    }

    pub fn entry(&self, name: &str, directory: Option<&str>) -> Option<&EntryObservation> {
        let panes: Vec<&PaneObservation> = match directory {
            Some(_) => self.pane(directory).into_iter().collect(),
            None => self.panes.iter().collect(),
        };
        panes
            .into_iter()
            .flat_map(|pane| &pane.entries)
            .find(|entry| entry.name == name)
    }

    pub fn entry_selected(&self, name: &str, directory: Option<&str>) -> bool {
        self.entry(name, directory)
            .is_some_and(|entry| entry.selected)
    }

    pub fn matches(&self, directory: Option<&str>) -> Vec<String> {
        self.results
            .clone()
            .unwrap_or_else(|| self.entry_names(directory))
    }

    pub fn focused_pane_name(&self) -> Option<&str> {
        self.focused_pane
            .and_then(|index| self.panes.get(index))
            .map(|pane| pane.name.as_str())
    }

    pub fn all_selected(&self) -> Vec<String> {
        self.panes
            .iter()
            .flat_map(|pane| &pane.entries)
            .filter(|entry| entry.selected)
            .map(|entry| entry.name.clone())
            .collect()
    }

    pub fn selected_names(&self, directory: Option<&str>) -> Vec<String> {
        self.pane(directory).map_or_else(Vec::new, |pane| {
            pane.entries
                .iter()
                .filter(|entry| entry.selected)
                .map(|entry| entry.name.clone())
                .collect()
        })
    }

    pub fn app_state(&self) -> AppState {
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

    pub fn evaluate(&self, predicate: &Predicate) -> Result<(), Failure> {
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
