// SPDX-License-Identifier: MIT

//! What the shortcut reference lists, as data: sections of rows, each an action and its keys.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShortcutData {
    pub sections: Vec<Section>,
    pub revision: u64,
}

/// A category: its title and rows. The title is also its category filter's name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    pub title: String,
    pub rows: Vec<Shortcut>,
}

/// One row: what it does, the keys that do it, and where.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shortcut {
    /// What the keys do, in the interface language: "Copy".
    pub action: String,
    /// The keys. A row whose command has no key has no row: leave it out.
    pub keys: Keys,
    /// Words around the keys: "Tab / Shift+Tab" + " in a rename".
    pub context: Option<Context>,
    pub search: Vec<String>,
}

impl Shortcut {
    /// A row with `action` and `keys`.
    pub fn new(action: impl Into<String>, keys: Keys) -> Self {
        Self {
            action: action.into(),
            keys,
            context: None,
            search: Vec::new(),
        }
    }

    /// With words around the keys.
    pub fn context(mut self, before: impl Into<String>, after: impl Into<String>) -> Self {
        self.context = Some(Context {
            before: before.into(),
            after: after.into(),
        });
        self
    }

    /// With more text for the search to match.
    pub fn search(mut self, terms: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.search.extend(terms.into_iter().map(Into::into));
        self
    }

    pub fn key_text(&self) -> String {
        let keys = self.keys.text();
        match &self.context {
            Some(c) => format!("{}{keys}{}", c.before, c.after),
            None => keys,
        }
    }
}

/// The words before and after a row's keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub before: String,
    pub after: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keys(pub Vec<Vec<String>>);

impl Keys {
    /// One alternative of one step: `Keys::one("F2")`.
    pub fn one(label: impl Into<String>) -> Self {
        Keys(vec![vec![label.into()]])
    }

    /// Several single-step alternatives: `Keys::any(["↑", "k"])`.
    pub fn any(labels: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Keys(labels.into_iter().map(|l| vec![l.into()]).collect())
    }

    /// One alternative of several steps: `Keys::seq(["g", "h"])`.
    pub fn seq(steps: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Keys(vec![steps.into_iter().map(Into::into).collect()])
    }

    /// Whether no key is given.
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(Vec::is_empty)
    }

    /// The keys as text: steps a space apart, alternatives a " / " apart.
    pub fn text(&self) -> String {
        self.0
            .iter()
            .filter(|a| !a.is_empty())
            .map(|a| a.join(" "))
            .collect::<Vec<_>>()
            .join(" / ")
    }
}
