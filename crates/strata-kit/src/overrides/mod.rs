// SPDX-License-Identifier: MIT

mod css;
mod error;
mod parse;
mod value;

#[cfg(test)]
mod tests;

use crate::look::Look;
use crate::theme::Location;
use crate::tokens::{Token, Value};
use value::Setting;

pub use css::{Migration, Reason, Unconverted, migrate_css};
pub use error::{OverrideError, Problem};

/// Which looks a row applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Every look: a row outside `[light]` and `[dark]`.
    All,
    /// A light look: a row under `[light]`.
    Light,
    /// A dark look: a row under `[dark]`.
    Dark,
}

impl Scope {
    /// The scope's table, or `None` for [`Scope::All`].
    pub fn table(self) -> Option<&'static str> {
        match self {
            Scope::All => None,
            Scope::Light => Some("light"),
            Scope::Dark => Some("dark"),
        }
    }

    /// The scope a look of this appearance applies after the unscoped rows.
    fn of(dark: bool) -> Scope {
        if dark { Scope::Dark } else { Scope::Light }
    }
}

/// One row that applies: a token, its value, and where the file sets it.
#[derive(Clone, Copy, Debug)]
struct Row {
    scope: Scope,
    token: &'static Token,
    value: Setting,
    at: Location,
}

impl PartialEq for Row {
    fn eq(&self, other: &Row) -> bool {
        self.scope == other.scope
            && self.token.name() == other.token.name()
            && self.value == other.value
            && self.at == other.at
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overrides {
    rows: Vec<Row>,
    errors: Vec<OverrideError>,
}

impl Overrides {
    pub fn parse(src: &str) -> Overrides {
        parse::parse(src)
    }

    pub fn from_bytes(bytes: &[u8]) -> Overrides {
        match std::str::from_utf8(bytes) {
            Ok(src) => Overrides::parse(src),
            Err(e) => {
                let valid = std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or_default();
                Overrides {
                    rows: Vec::new(),
                    errors: vec![OverrideError {
                        at: Location::of(valid, valid.len()),
                        scope: Scope::All,
                        problem: Problem::NotUtf8,
                    }],
                }
            }
        }
    }

    pub fn apply(&self, look: &mut Look) {
        let scoped = Scope::of(look.kit.dark);
        // The rows in the order they win: unscoped, then scoped, so the last for a token wins.
        let mut winners: Vec<&Row> = Vec::new();
        for row in [Scope::All, scoped]
            .into_iter()
            .flat_map(|scope| self.rows.iter().filter(move |r| r.scope == scope))
        {
            match winners
                .iter_mut()
                .find(|w| w.token.name() == row.token.name())
            {
                Some(w) => *w = row,
                None => winners.push(row),
            }
        }
        let set = |look: &mut Look, token: &Token, value| {
            let set = token.set(look, value);
            debug_assert!(set.is_ok(), "parse checks every value: {set:?}");
        };
        for row in &winners {
            if let Setting::Value(v) = row.value {
                set(look, row.token, v);
            }
        }
        let derived: Vec<(&Token, Value)> = winners
            .iter()
            .filter_map(|row| match row.value {
                Setting::Derived { from, alpha } => {
                    Some((row.token, Value::Color(Setting::resolve(from, alpha, look))))
                }
                Setting::Value(_) => None,
            })
            .collect();
        for (token, v) in derived {
            set(look, token, v);
        }
    }

    /// `look` with [`Overrides::apply`] applied. `look` must be freshly built, as there.
    pub fn applied(&self, mut look: Look) -> Look {
        self.apply(&mut look);
        look
    }

    /// Every problem in the file, in the order they appear in it.
    pub fn errors(&self) -> &[OverrideError] {
        &self.errors
    }

    /// The tokens the file sets, with the scope of each row, in no particular order.
    pub fn tokens(&self) -> impl Iterator<Item = (Scope, &'static Token)> + '_ {
        self.rows.iter().map(|r| (r.scope, r.token))
    }

    /// How many rows apply to some look.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}
