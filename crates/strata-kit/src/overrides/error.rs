//! What can be wrong with a row of the override file, and how it reads.

use std::fmt;

use super::Scope;
use crate::theme::Location;
use crate::tokens::{self, Kind};

/// One problem in the override file, and where. Its `Display` is the line a user reads:
/// `line 4, column 1: unknown token `list.row.length``.
#[derive(Clone, Debug, PartialEq)]
pub struct OverrideError {
    /// Where the problem is: the key for an unknown token, the value for a bad one.
    pub at: Location,
    /// The table the row sits under.
    pub scope: Scope,
    pub problem: Problem,
}

/// What is wrong. Names of known tokens are the registry's own (`Token::name`); an unknown
/// name is as the file wrote it, without its scope.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Problem {
    /// The line isn't valid TOML. It doesn't apply; the rest of the file does.
    Syntax(String),
    /// The file isn't UTF-8 text, so nothing in it applies.
    NotUtf8,
    /// No token has this name, and none starts with it.
    Unknown(String),
    /// A value is set on a name that only starts tokens' names, such as `list.row`.
    Group(String),
    /// The value is of the wrong kind: `found` is its TOML type (`string`, `integer`, …).
    Kind {
        name: &'static str,
        expected: Kind,
        found: &'static str,
    },
    /// The number is outside the token's range, or not finite.
    Range {
        name: &'static str,
        min: f32,
        max: f32,
    },
    /// The string isn't a colour, a reference or an `alpha()` of either.
    BadColor { name: &'static str, value: String },
    /// A colour refers to `@target`, which isn't a colour token.
    Reference { name: &'static str, target: String },
    /// `light` or `dark` somewhere other than a table at the top of the file.
    Scope(String),
}

impl fmt::Display for OverrideError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}, column {}: {}",
            self.at.line, self.at.column, self.problem
        )?;
        if let Some(table) = self.scope.table() {
            write!(f, " (in [{table}])")?;
        }
        Ok(())
    }
}

impl std::error::Error for OverrideError {}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Syntax(message) => f.write_str(message),
            Problem::NotUtf8 => f.write_str("the file isn't UTF-8 text, so none of it applies"),
            Problem::Unknown(name) => write!(f, "unknown token `{name}`"),
            Problem::Group(name) => {
                let prefix = format!("{name}.");
                match tokens::all().find(|t| t.name().starts_with(&prefix)) {
                    Some(t) => write!(
                        f,
                        "`{name}` names a group of tokens: set one of them, such as `{}`",
                        t.name()
                    ),
                    None => write!(f, "`{name}` names a group of tokens"),
                }
            }
            Problem::Kind {
                name,
                expected,
                found,
            } => {
                let article = if found.starts_with(['a', 'e', 'i', 'o', 'u']) {
                    "an"
                } else {
                    "a"
                };
                write!(f, "`{name}` takes {expected}, not {article} {found}")?;
                if matches!(expected, Kind::Length | Kind::Number) && *found == "string" {
                    f.write_str(": write the number without quotes or units")?;
                }
                Ok(())
            }
            Problem::Range { name, min, max } => {
                if max.is_infinite() {
                    write!(f, "`{name}` takes a finite number of at least {min}")
                } else {
                    write!(f, "`{name}` takes a number from {min} to {max}")
                }
            }
            Problem::BadColor { name, value } => {
                write!(f, "`{name}` takes a colour, and {value:?} isn't one")
            }
            Problem::Reference { name, target } => {
                write!(
                    f,
                    "`{name}` refers to `@{target}`, which isn't a colour token"
                )
            }
            Problem::Scope(key) => write!(
                f,
                "`{key}` is a scope: write it as a table at the top of the file, [{key}]"
            ),
        }
    }
}
