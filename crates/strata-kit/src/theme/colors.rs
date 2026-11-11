//! The typed theme: Strata's nine interface colours and five syntax colours, as its theme
//! files and catalogue write them (`ThemeTokens` in `src/ui/theme.rs` at lgse/strata
//! 35ee677ab), and the reader that turns a TOML table into one.

use std::fmt;
use std::ops::Range;

use toml::Spanned;
use toml::de::{DeTable, DeValue};

use super::color::{Rgba8, luminance};

#[cfg(test)]
mod tests;

/// A key a theme file sets, in the order Strata's theme editor lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Background,
    Surface,
    Text,
    Accent,
    Danger,
    Muted,
    Highlight,
    Border,
    DimText,
    SyntaxKeyword,
    SyntaxString,
    SyntaxConstant,
    SyntaxType,
    SyntaxPreprocessor,
}

impl Key {
    pub const ALL: [Key; 14] = [
        Key::Background,
        Key::Surface,
        Key::Text,
        Key::Accent,
        Key::Danger,
        Key::Muted,
        Key::Highlight,
        Key::Border,
        Key::DimText,
        Key::SyntaxKeyword,
        Key::SyntaxString,
        Key::SyntaxConstant,
        Key::SyntaxType,
        Key::SyntaxPreprocessor,
    ];

    /// The key as a theme file writes it.
    pub fn name(self) -> &'static str {
        match self {
            Key::Background => "background",
            Key::Surface => "surface",
            Key::Text => "text",
            Key::Accent => "accent",
            Key::Danger => "danger",
            Key::Muted => "muted",
            Key::Highlight => "highlight",
            Key::Border => "border",
            Key::DimText => "dim_text",
            Key::SyntaxKeyword => "syntax_keyword",
            Key::SyntaxString => "syntax_string",
            Key::SyntaxConstant => "syntax_constant",
            Key::SyntaxType => "syntax_type",
            Key::SyntaxPreprocessor => "syntax_preprocessor",
        }
    }

    /// True for the five syntax keys, which a custom file may leave out.
    pub fn is_syntax(self) -> bool {
        matches!(
            self,
            Key::SyntaxKeyword
                | Key::SyntaxString
                | Key::SyntaxConstant
                | Key::SyntaxType
                | Key::SyntaxPreprocessor
        )
    }
}

/// Strata's default `danger`, for a custom file that leaves it out.
pub const DEFAULT_DANGER: Rgba8 = Rgba8::rgb(0xe5, 0x48, 0x4d);

/// A theme's colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Colors {
    /// The window and the file columns.
    pub background: Rgba8,
    /// Panes, headers, the sidebar, popovers and dialogs.
    pub surface: Rgba8,
    pub text: Rgba8,
    pub accent: Rgba8,
    pub danger: Rgba8,
    /// The quiet control and hover surface.
    pub muted: Rgba8,
    /// The selection surface.
    pub highlight: Rgba8,
    pub border: Rgba8,
    /// Secondary text, and comments in code.
    pub dim_text: Rgba8,
    pub syntax: SourceColors,
}

/// The syntax colours a theme sets. A colour left out is derived from the accent and text
/// ([`super::syntax::resolve`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SourceColors {
    pub keyword: Option<Rgba8>,
    pub string: Option<Rgba8>,
    pub constant: Option<Rgba8>,
    pub type_: Option<Rgba8>,
    pub preprocessor: Option<Rgba8>,
}

impl Colors {
    /// The colour behind `key`, or `None` for a syntax colour the theme leaves out.
    pub fn get(&self, key: Key) -> Option<Rgba8> {
        let s = &self.syntax;
        match key {
            Key::Background => Some(self.background),
            Key::Surface => Some(self.surface),
            Key::Text => Some(self.text),
            Key::Accent => Some(self.accent),
            Key::Danger => Some(self.danger),
            Key::Muted => Some(self.muted),
            Key::Highlight => Some(self.highlight),
            Key::Border => Some(self.border),
            Key::DimText => Some(self.dim_text),
            Key::SyntaxKeyword => s.keyword,
            Key::SyntaxString => s.string,
            Key::SyntaxConstant => s.constant,
            Key::SyntaxType => s.type_,
            Key::SyntaxPreprocessor => s.preprocessor,
        }
    }

    /// Sets the colour behind `key`; `None` leaves a syntax colour out (it has no effect on
    /// an interface colour).
    pub fn set(&mut self, key: Key, value: Option<Rgba8>) {
        let s = &mut self.syntax;
        let slot = match key {
            Key::Background => &mut self.background,
            Key::Surface => &mut self.surface,
            Key::Text => &mut self.text,
            Key::Accent => &mut self.accent,
            Key::Danger => &mut self.danger,
            Key::Muted => &mut self.muted,
            Key::Highlight => &mut self.highlight,
            Key::Border => &mut self.border,
            Key::DimText => &mut self.dim_text,
            Key::SyntaxKeyword => return s.keyword = value,
            Key::SyntaxString => return s.string = value,
            Key::SyntaxConstant => return s.constant = value,
            Key::SyntaxType => return s.type_ = value,
            Key::SyntaxPreprocessor => return s.preprocessor = value,
        };
        if let Some(v) = value {
            *slot = v;
        }
    }
}

/// Where a theme came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// Strata's catalogue (`themes/catalog.toml`).
    Bundled,
    /// A file in `~/.config/strata/themes/`.
    Custom,
    /// The current Omarchy theme.
    Omarchy,
}

/// One theme: its id (a catalogue id or a file stem), its name as written, and its colours.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub colors: Colors,
    pub source: Source,
}

impl Theme {
    /// Strata's library filter: the theme is listed under Light when its background's
    /// relative luminance exceeds 0.4.
    pub fn is_light(&self) -> bool {
        luminance(self.colors.background) > 0.4
    }

    /// Strata's rule for GTK's dark variant and the Catppuccin icon flavour:
    /// `(299 R + 587 G + 114 B) / 1000` below 128. K3 picks Mocha icons when it holds.
    pub fn dark_background(&self) -> bool {
        let Rgba8 { r, g, b, .. } = self.colors.background;
        u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114 < 128_000
    }
}

/// Where in a file something is: 1-based line and column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

impl Location {
    /// The location of byte `at` in `src`.
    pub fn of(src: &str, at: usize) -> Location {
        let before = &src[..at.min(src.len())];
        let line = before.bytes().filter(|&b| b == b'\n').count() + 1;
        let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        Location { line, column }
    }
}

/// Why a theme file didn't load.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The file isn't valid TOML.
    Syntax(String),
    /// A required key is missing.
    Missing(&'static str),
    /// A key holds something other than a string.
    NotAString(&'static str),
    /// A colour key's value isn't a colour.
    BadColor { key: &'static str, value: String },
    /// `name` is empty or only whitespace.
    EmptyName,
}

/// A theme file that didn't load, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeError {
    /// `None` when the problem has no place in the file (a missing key).
    pub at: Option<Location>,
    pub kind: ErrorKind,
}

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(at) = self.at {
            write!(f, "line {}, column {}: ", at.line, at.column)?;
        }
        match &self.kind {
            ErrorKind::Syntax(m) => write!(f, "{m}"),
            ErrorKind::Missing(key) => write!(f, "`{key}` is missing"),
            ErrorKind::NotAString(key) => write!(f, "`{key}` must be a string"),
            ErrorKind::BadColor { key, value } => {
                write!(f, "`{key}` is not a colour: {value:?}")
            }
            ErrorKind::EmptyName => write!(f, "`name` is empty"),
        }
    }
}

impl std::error::Error for ThemeError {}

/// Parses `src` as a TOML document, with spans.
pub(crate) fn parse_document(src: &str) -> Result<Spanned<DeTable<'_>>, ThemeError> {
    DeTable::parse(src).map_err(|e| {
        let message = e.message().trim().to_owned();
        ThemeError {
            at: e.span().map(|s| Location::of(src, s.start)),
            kind: ErrorKind::Syntax(message),
        }
    })
}

/// Reads a theme's name and colours from `table`, a theme file or a catalogue entry in
/// `src`. `danger` defaults to [`DEFAULT_DANGER`]; the syntax keys are optional. Unknown keys
/// are ignored, as Strata ignores them. The first problem wins.
pub(crate) fn read_table(src: &str, table: &DeTable<'_>) -> Result<(String, Colors), ThemeError> {
    let at = |span: Range<usize>| Some(Location::of(src, span.start));
    let string = |key: &'static str| -> Result<Option<(&str, Range<usize>)>, ThemeError> {
        let Some(v) = table.get(key) else {
            return Ok(None);
        };
        match v.get_ref() {
            DeValue::String(s) => Ok(Some((s.as_ref(), v.span()))),
            _ => Err(ThemeError {
                at: at(v.span()),
                kind: ErrorKind::NotAString(key),
            }),
        }
    };
    let name = match string("name")? {
        Some((name, span)) if name.trim().is_empty() => {
            return Err(ThemeError {
                at: at(span),
                kind: ErrorKind::EmptyName,
            });
        }
        Some((name, _)) => name.to_owned(),
        None => {
            return Err(ThemeError {
                at: None,
                kind: ErrorKind::Missing("name"),
            });
        }
    };
    let mut colors = Colors {
        background: Rgba8::BLACK,
        surface: Rgba8::BLACK,
        text: Rgba8::BLACK,
        accent: Rgba8::BLACK,
        danger: DEFAULT_DANGER,
        muted: Rgba8::BLACK,
        highlight: Rgba8::BLACK,
        border: Rgba8::BLACK,
        dim_text: Rgba8::BLACK,
        syntax: SourceColors::default(),
    };
    for key in Key::ALL {
        let name = key.name();
        match string(name)? {
            Some((value, span)) => {
                let parsed = Rgba8::parse(value).ok_or_else(|| ThemeError {
                    at: at(span),
                    kind: ErrorKind::BadColor {
                        key: name,
                        value: value.to_owned(),
                    },
                })?;
                colors.set(key, Some(parsed));
            }
            None if key.is_syntax() || key == Key::Danger => {}
            None => {
                return Err(ThemeError {
                    at: None,
                    kind: ErrorKind::Missing(name),
                });
            }
        }
    }
    Ok((name, colors))
}
