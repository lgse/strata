// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{
    AccentPixels, BandCheck, BlankCheck, ChildNames, ChildrenCheck, Comparison, ContainsPoint,
    ContrastBelow, DescriptionCheck, EditorWindowCheck, Elapsed, FieldCheck, FieldRef, FileCheck,
    FooterFill, HitPath, InsideCheck, LargestImage, MentionCheck, Movement, NameCheck, Names,
    PanesWith, PixelCheck, PrefCheck, Presence, Repainted, Replaced, Resized, RowStates,
    ScrollCheck, SectionsCheck, SomeWindow, StateCheck, SubtreeText, Target, TextMatch,
    WindowCount,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Predicate {
    #[serde(flatten)]
    pub(crate) check: Check,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) dir: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Check {
    Directory(Option<String>),
    Panes(Names),
    Pane(Option<String>),
    Entries(Names),
    Entry(String),
    EntryGone(String),
    Selected(Names),
    AllSelected(Names),
    Focused(Presence),
    FocusedInMatches(bool),
    FocusedPane(Presence),
    ViewMode(String),
    WindowWidth(i32),
    Dialog(Presence),
    MenuItems(Option<Names>),
    Preview(Presence),
    Peek(bool),
    Matches(Names),
    Field(FieldCheck),
    Pref(PrefCheck),
    EditorWithinViewport(FieldRef),
    State(StateCheck),
    Any(Vec<Predicate>),
    Present(Target),
    Absent(Target),
    Moved(Movement),
    OnScreen(Target),
    Blank(BlankCheck),
    Not(Box<Predicate>),
    Visible(Names),
    ScrollValue(ScrollCheck),
    BandSelected(BandCheck),
    FocusWithin(Target),
    Resized(Resized),
    ContainsPoint(ContainsPoint),
    FooterFill(FooterFill),
    Repainted(Repainted),
    ContrastBelow(ContrastBelow),
    Elapsed(Elapsed),
    DiagnosticsMention(Vec<String>),
    All(Vec<Predicate>),
    EditableFields(usize),
    Description(DescriptionCheck),
    Mentions(MentionCheck),
    Presented(Target),
    File(FileCheck),
    FocusedWidget(Option<TextMatch>),
    Inside(InsideCheck),
    EditorWithinWindow(EditorWindowCheck),
    Replaced(Replaced),
    Pixel(PixelCheck),
    AccentPixels(AccentPixels),
    AnyResultRow(RowStates),
    HitPath(HitPath),
    Compare(Comparison),
    PanesWith(PanesWith),
    Name(NameCheck),
    EditableText(String),
    VisibleTop(Names),
    LargestImage(LargestImage),
    Children(ChildrenCheck),
    ListSections(SectionsCheck),
    Text(NameCheck),
    SomeWindow(SomeWindow),
    Windows(WindowCount),
    ChildNames(ChildNames),
    FocusOutside(Target),
    FocusedWidgetIs(Target),
    Alive(String),
    SubtreeText(SubtreeText),
}

impl Check {
    fn takes_directory(&self) -> bool {
        matches!(
            self,
            Self::Entries(_)
                | Self::Selected(_)
                | Self::Entry(_)
                | Self::EntryGone(_)
                | Self::Matches(_)
                | Self::Visible(_)
                | Self::ScrollValue(_)
                | Self::BandSelected(_)
        )
    }

    pub(crate) fn is_live(&self) -> bool {
        match self {
            Self::Present(_)
            | Self::Absent(_)
            | Self::Moved(_)
            | Self::Field(_)
            | Self::Pref(_)
            | Self::EditorWithinViewport(_)
            | Self::State(_)
            | Self::OnScreen(_)
            | Self::Blank(_)
            | Self::Visible(_)
            | Self::ScrollValue(_)
            | Self::BandSelected(_)
            | Self::FocusWithin(_)
            | Self::Resized(_)
            | Self::ContainsPoint(_)
            | Self::FooterFill(_)
            | Self::Repainted(_)
            | Self::ContrastBelow(_)
            | Self::Elapsed(_)
            | Self::DiagnosticsMention(_)
            | Self::EditableFields(_)
            | Self::Description(_)
            | Self::Mentions(_)
            | Self::Presented(_)
            | Self::File(_)
            | Self::FocusedWidget(_)
            | Self::Inside(_)
            | Self::EditorWithinWindow(_)
            | Self::Replaced(_)
            | Self::Pixel(_)
            | Self::AccentPixels(_)
            | Self::HitPath(_)
            | Self::AnyResultRow(_)
            | Self::Compare(_)
            | Self::PanesWith(_)
            | Self::Name(_)
            | Self::EditableText(_)
            | Self::VisibleTop(_)
            | Self::LargestImage(_)
            | Self::Children(_)
            | Self::ListSections(_)
            | Self::Text(_)
            | Self::SomeWindow(_)
            | Self::Windows(_)
            | Self::ChildNames(_)
            | Self::FocusOutside(_)
            | Self::FocusedWidgetIs(_)
            | Self::Alive(_)
            | Self::SubtreeText(_) => true,
            Self::Any(predicates) | Self::All(predicates) => {
                predicates.iter().any(|predicate| predicate.check.is_live())
            }
            Self::Not(predicate) => predicate.check.is_live(),
            _ => false,
        }
    }
}

impl<'de> Deserialize<'de> for Predicate {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut object = Map::<String, Value>::deserialize(deserializer)?;
        let dir = match object.remove("dir") {
            None | Some(Value::Null) => None,
            Some(Value::String(dir)) => Some(dir),
            Some(other) => {
                return Err(serde::de::Error::custom(format!(
                    "dir must be a string, not {other}"
                )));
            }
        };
        if object.len() != 1 {
            let keys: Vec<&String> = object.keys().collect();
            return Err(serde::de::Error::custom(format!(
                "a predicate needs exactly one check, found {keys:?}"
            )));
        }
        let check: Check =
            serde_json::from_value(Value::Object(object)).map_err(serde::de::Error::custom)?;
        let field = match &check {
            Check::Field(check) => Some(check.field()),
            Check::EditorWithinViewport(field) => Some(field.clone()),
            _ => None,
        };
        match &check {
            Check::All(predicates) if predicates.is_empty() => {
                return Err(serde::de::Error::custom("all takes at least one predicate"));
            }
            Check::Any(predicates) if predicates.is_empty() => {
                return Err(serde::de::Error::custom("any takes at least one predicate"));
            }
            _ => {}
        }
        if let Check::Pref(pref) = &check
            && [
                pref.value.is_some(),
                pref.contains.is_some(),
                pref.one_of.is_some(),
            ]
            .into_iter()
            .filter(|given| *given)
            .count()
                != 1
        {
            return Err(serde::de::Error::custom(
                "pref takes exactly one of value, contains, and one_of",
            ));
        }
        if let Check::PanesWith(panes) = &check
            && !matches!(panes.target, Target::Role { within: None, .. })
        {
            return Err(serde::de::Error::custom(
                "panes_with takes a role target without within",
            ));
        }
        if let Check::Mentions(mention) = &check
            && [
                mention.text.is_some(),
                mention.words.is_some(),
                mention.equals.is_some(),
            ]
            .into_iter()
            .filter(|given| *given)
            .count()
                != 1
        {
            return Err(serde::de::Error::custom(
                "mentions takes exactly one of text, words, and equals",
            ));
        }
        if let Check::LargestImage(largest) = &check
            && largest.size.is_some()
            && largest.exceeds.is_some()
        {
            return Err(serde::de::Error::custom(
                "largest_image takes size or exceeds, not both",
            ));
        }
        if let Check::File(file) = &check {
            if !file.path.starts_with('/') {
                return Err(serde::de::Error::custom(
                    "file takes an absolute path, such as $fixture/todo.txt",
                ));
            }
            if file.exists.is_some() == file.mode_bits.is_some() {
                return Err(serde::de::Error::custom(
                    "file takes exactly one of exists and mode_bits",
                ));
            }
            if let Some(bits) = &file.mode_bits
                && u32::from_str_radix(&bits.mask, 8).is_err()
            {
                return Err(serde::de::Error::custom(
                    "mode_bits takes an octal mask such as \"100\"",
                ));
            }
        }
        if field.is_some_and(|field| field.of.is_some() && field.name.is_some()) {
            return Err(serde::de::Error::custom(
                "a field is either remembered (of) or found by name, not both",
            ));
        }
        if dir.is_some() && !check.takes_directory() {
            return Err(serde::de::Error::custom(
                "dir applies only to entries, entry, entry_gone, selected, matches, visible, \
                 scroll_value, and band_selected",
            ));
        }
        Ok(Self { check, dir })
    }
}
