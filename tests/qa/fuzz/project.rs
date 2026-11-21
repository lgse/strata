// SPDX-License-Identifier: MIT

//! The projection: what of a reply an observation keeps, and how its fields are named.
//!
//! A field is a dotted path into the observation: `app.focused`, `app.panes[1].selected`,
//! `fs.documents/notes.txt`, `settings.show_hidden`. The normalisation map and the
//! calibration mask name fields the same way, with `[]` for every element of a list; a name
//! covers everything below it.

use serde_json::{Map, Value};

/// The `app` keys an observation keeps: all ten the snapshot has
/// (`.agents/skills/qa-scenarios/reference.md`). A key calibration cannot make agree is
/// masked in `calibration.mask`, where it is counted, and never dropped from this list.
pub(crate) const APP_KEYS: [&str; 10] = [
    "view_mode",
    "directory",
    "panes",
    "focused",
    "focused_pane",
    "dialog",
    "menu",
    "preview",
    "field",
    "matches",
];
/// What an observation holds beside `app`, in the order fields are compared.
pub(crate) const RESULT: &str = "result";
pub(crate) const CRITICALS: &str = "criticals";
/// The invariants a target holds itself to at rest that failed (`docs/testing.md`,
/// "Invariants").
pub(crate) const INVARIANTS: &str = "invariants";
pub(crate) const APP: &str = "app";
pub(crate) const FS: &str = "fs";
pub(crate) const SETTINGS: &str = "settings";
/// What a portal run's request answered: `answered`, then `status` and `paths`, or `error`.
pub(crate) const PORTAL: &str = "portal";
pub(crate) const ROOTS: [&str; 7] = [RESULT, CRITICALS, INVARIANTS, APP, FS, SETTINGS, PORTAL];
/// What an observation keeps beside its roots and never compares: the windows open beside the
/// one the steps act on (the snapshot's `toplevels`), which a rule's `while` may read.
pub(crate) const CONTEXT: &str = "context";
/// The context's keys: `context.toplevels`, and `context.window_focused`, whether anything
/// in the window has the keyboard.
pub(crate) const TOPLEVELS: &str = "toplevels";
pub(crate) const WINDOW_FOCUSED: &str = "window_focused";
/// How an observation writes the fixture directory's name, which is random in every run.
const ROOT: &str = "$root";
/// How an observation writes a date Strata shows relative to now for a file the run made:
/// "Just now" or "5m ago" follow the clock, not the application, and a long replay passes a
/// minute in one replay and not the next (`archives` seeds 2001 to 2010, F4's round 2). The
/// fixture pins every file it makes to 2023, so only a file the run made shows one, and that is
/// minutes old at most: an hour count is never the clock's, and is left as it is. A wrong count
/// of minutes is hidden, as a projection maps each value alone.
const RECENT: &str = "$recent";

/// The names that differ from one replay to the next and mean the same place in each: the
/// fixture directory is made afresh, so its name and the paths around it are random.
pub(crate) struct Places {
    /// Longest first, so a path is replaced before the name it ends with.
    replacements: Vec<(String, &'static str)>,
    /// The fixture directory's name as its stem, up to its last `-`, and its random suffix: a
    /// name with characters taken off its end, as a BackSpace in the location field leaves it,
    /// is the same edit in every run (`location` seed `0x6e`, F4's round 2).
    cut: Option<(String, String)>,
}

impl Places {
    pub(crate) fn new(fixture: &str, environment: &str) -> Self {
        let root = fixture.rsplit('/').next().unwrap_or(fixture).to_owned();
        let cut = root
            .rsplit_once('-')
            .filter(|(_, suffix)| !suffix.is_empty())
            .map(|(stem, suffix)| (format!("{stem}-"), suffix.to_owned()));
        let mut replacements = vec![
            (fixture.to_owned(), "$fixture"),
            (environment.to_owned(), "$environment"),
            (root, ROOT),
        ];
        replacements.retain(|(name, _)| !name.is_empty());
        replacements.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
        Self { replacements, cut }
    }

    fn text(&self, text: &str) -> String {
        let whole = self
            .replacements
            .iter()
            .fold(text.to_owned(), |text, (name, variable)| {
                text.replace(name, variable)
            });
        self.cut_short(&whole)
    }

    /// `text` with the fixture's name cut short, its stem and some of its suffix, written as
    /// `$root[-<n>]`, `n` the characters it lacks. The whole name was replaced before.
    fn cut_short(&self, text: &str) -> String {
        let Some((stem, suffix)) = &self.cut else {
            return text.to_owned();
        };
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(at) = rest.find(stem.as_str()) {
            out.push_str(&rest[..at]);
            let after = &rest[at + stem.len()..];
            let kept = suffix
                .char_indices()
                .map(|(index, character)| index + character.len_utf8())
                .take_while(|end| after.starts_with(&suffix[..*end]))
                .last()
                .unwrap_or(0);
            out.push_str(&format!("{ROOT}[-{}]", suffix.len() - kept));
            rest = &after[kept..];
        }
        out.push_str(rest);
        out
    }

    /// The value with every run-specific name, in a string or a key, written as the
    /// scenario variable that stands for it.
    pub(crate) fn normalise(&self, value: &Value) -> Value {
        match value {
            Value::String(_) if is_recent(value) => Value::String(RECENT.to_owned()),
            Value::String(text) => Value::String(self.text(text)),
            Value::Array(items) => {
                Value::Array(items.iter().map(|item| self.normalise(item)).collect())
            }
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(key, item)| (self.text(key), self.normalise(item)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

/// Whether `value` is a text that is a date Strata writes relative to now for a file of the
/// last hour: "Just now", or a count of minutes ago (`strata_core::util`'s relative date, in
/// the QA image's English).
fn is_recent(value: &Value) -> bool {
    let Some(text) = value.as_str() else {
        return false;
    };
    if text == "Just now" {
        return true;
    }
    text.strip_suffix("m ago")
        .is_some_and(|count| !count.is_empty() && count.bytes().all(|byte| byte.is_ascii_digit()))
}

/// The projected `app`: its ten keys and nothing a later protocol might add, so adding a
/// key to the snapshot cannot change a stream before it is calibrated.
pub(crate) fn app(reply_app: &Value, places: &Places) -> Value {
    let Some(snapshot) = reply_app.as_object() else {
        return Value::Null;
    };
    let kept: Map<String, Value> = APP_KEYS
        .iter()
        .map(|key| {
            (
                (*key).to_owned(),
                places.normalise(snapshot.get(*key).unwrap_or(&Value::Null)),
            )
        })
        .collect();
    Value::Object(kept)
}

/// The projected context: the titles of the windows open beside the one the steps act on.
pub(crate) fn context(reply_app: &Value, places: &Places) -> Value {
    let toplevels = reply_app
        .get(TOPLEVELS)
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let focused = reply_app
        .get(WINDOW_FOCUSED)
        .cloned()
        .unwrap_or(Value::Null);
    serde_json::json!({ TOPLEVELS: places.normalise(&toplevels), WINDOW_FOCUSED: focused })
}

/// Whether `field` is what a rule's `while` may read: a key of `app`, `context.toplevels` or
/// `context.window_focused`, with no list.
pub(crate) fn names_a_condition(field: &str) -> bool {
    match parts(field).as_slice() {
        [Part::Key(APP), Part::Key(key)] => APP_KEYS.contains(key),
        [Part::Key(CONTEXT), Part::Key(TOPLEVELS | WINDOW_FOCUSED)] => true,
        _ => false,
    }
}

/// Whether the application shows a folder outside the fixture: its first pane is neither the
/// fixture's own nor a directory of the fixture's tree. Columns keeps the root's column
/// first for as long as it is inside; the other views show one pane, the folder they are in.
pub(crate) fn outside(app: &Value, tree: &Value) -> bool {
    let Some(first) = app["panes"][0]["name"].as_str() else {
        return false;
    };
    let is_directory = |what: &Value| what.as_str().is_some_and(|what| what.starts_with("dir "));
    first != ROOT
        && !tree.as_object().is_some_and(|entries| {
            entries
                .iter()
                .any(|(path, what)| is_directory(what) && path.rsplit('/').next() == Some(first))
        })
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Difference {
    pub(crate) field: String,
    pub(crate) left: Value,
    pub(crate) right: Value,
}

/// Every field on which two observations differ, in comparison order: the observation's
/// roots as `ROOTS` lists them, then keys by name and list elements by index.
pub(crate) fn differences(left: &Value, right: &Value) -> Vec<Difference> {
    let mut found = Vec::new();
    for root in ROOTS {
        walk(
            root,
            left.get(root).unwrap_or(&Value::Null),
            right.get(root).unwrap_or(&Value::Null),
            &mut found,
        );
    }
    found
}

fn walk(path: &str, left: &Value, right: &Value, found: &mut Vec<Difference>) {
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => walk_keys(path, left, right, found),
        // A list of records is compared record by record while both have as many; a list of
        // names is one value, since which name is where is the observation.
        (Value::Array(left), Value::Array(right)) if are_records(left, right) => {
            for (index, (left, right)) in left.iter().zip(right).enumerate() {
                walk(&format!("{path}[{index}]"), left, right, found);
            }
        }
        _ if left == right => {}
        _ => found.push(Difference {
            field: path.to_owned(),
            left: left.clone(),
            right: right.clone(),
        }),
    }
}

fn walk_keys(
    path: &str,
    left: &Map<String, Value>,
    right: &Map<String, Value>,
    found: &mut Vec<Difference>,
) {
    let mut keys: Vec<&String> = left.keys().chain(right.keys()).collect();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        walk(
            &format!("{path}.{key}"),
            left.get(key).unwrap_or(&Value::Null),
            right.get(key).unwrap_or(&Value::Null),
            found,
        );
    }
}

fn are_records(left: &[Value], right: &[Value]) -> bool {
    left.len() == right.len() && left.iter().chain(right).all(Value::is_object)
}

/// One part of a field's path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part<'a> {
    Key(&'a str),
    Index(usize),
    /// `[]` in a map or mask entry: every element of a list.
    Every,
}

/// A field as its parts: `app.panes[1].selected` is `app`, `panes`, 1, `selected`.
///
/// What is below `fs` is one key, a file's path, whatever dots and brackets its name has.
/// What is below `app` and `settings` is split at dots, and a key's `[n]` or `[]` is a
/// place in a list.
fn parts(field: &str) -> Vec<Part<'_>> {
    let (root, below) = field.split_once('.').unwrap_or((field, ""));
    let mut parts = vec![Part::Key(root)];
    if below.is_empty() {
        return parts;
    }
    if root == FS {
        parts.push(Part::Key(below));
        return parts;
    }
    for key in below.split('.') {
        let (name, places) = key.split_once('[').unwrap_or((key, ""));
        parts.push(Part::Key(name));
        if places.is_empty() && !key.contains('[') {
            continue;
        }
        for place in places.split('[') {
            let place = place.trim_end_matches(']');
            parts.push(place.parse().map_or(Part::Every, Part::Index));
        }
    }
    parts
}

/// A field with its list indexes taken out, as the map and the mask name it:
/// `app.panes[1].selected` is `app.panes[].selected`. A file's path is left as it is.
pub(crate) fn pattern_of(field: &str) -> String {
    let mut pattern = String::with_capacity(field.len());
    for (index, part) in parts(field).into_iter().enumerate() {
        match part {
            Part::Key(key) if index == 0 => pattern.push_str(key),
            Part::Key(key) => {
                pattern.push('.');
                pattern.push_str(key);
            }
            Part::Index(_) | Part::Every => pattern.push_str("[]"),
        }
    }
    pattern
}

/// Whether a map or mask entry covers a field: the field itself, or what is below it at a
/// boundary of the path. `app.focused` does not cover `app.focused_pane`, and
/// `fs.documents/notes` does not cover `fs.documents/notes.txt`; `fs.documents` covers
/// every file of the folder.
pub(crate) fn covers(entry: &str, field: &str) -> bool {
    let (entry, field) = (parts(entry), parts(field));
    if entry.first() == Some(&Part::Key(FS)) {
        return match (entry.get(1), field.get(1)) {
            (Some(Part::Key(folder)), Some(Part::Key(path))) if field[0] == entry[0] => path
                .strip_prefix(folder)
                .is_some_and(|below| below.is_empty() || below.starts_with('/')),
            _ => false,
        };
    }
    entry.len() <= field.len()
        && entry.iter().zip(&field).all(|(entry, field)| match entry {
            Part::Every => matches!(field, Part::Index(_) | Part::Every),
            entry => entry == field,
        })
}

/// Whether an entry names something a map or a mask may leave out: an `app` key or what is
/// below it, a path of the tree, a key of the settings. A whole root is refused, and so
/// are `result`, `criticals` and `invariants`: one line must not blind a run. A list is
/// written `[]`.
pub(crate) fn names_a_field(entry: &str) -> bool {
    let parts = parts(entry);
    let below_root = match parts.as_slice() {
        [Part::Key(APP), Part::Key(key), ..] => APP_KEYS.contains(key),
        [Part::Key(FS | SETTINGS), Part::Key(key), ..] => !key.is_empty(),
        _ => false,
    };
    let well_formed = parts.iter().all(|part| match part {
        Part::Key(key) => !key.is_empty(),
        Part::Index(_) => false,
        Part::Every => true,
    });
    below_root && well_formed && entry == pattern_of(entry)
}

/// The value at a field of an observation, for a perturbation's condition.
pub(crate) fn value_at<'a>(observation: &'a Value, field: &str) -> Option<&'a Value> {
    parts(field)
        .into_iter()
        .try_fold(observation, |value, part| match part {
            Part::Key(key) => value.get(key),
            Part::Index(index) => value.get(index),
            Part::Every => None,
        })
}

/// Sets the value at a field that exists; false when the observation has no such field.
pub(crate) fn set_at(observation: &mut Value, field: &str, new: Value) -> bool {
    let place = parts(field)
        .into_iter()
        .try_fold(observation, |value, part| match part {
            Part::Key(key) => value.get_mut(key),
            Part::Index(index) => value.get_mut(index),
            Part::Every => None,
        });
    place.is_some_and(|place| {
        *place = new;
        true
    })
}
