//! User overrides: `overrides.toml`, applied after the theme, with live reload and errors by
//! line.
//!
//! Work package: K4, `docs/work/phase-1-leaves/K4-user-overrides.md`. This replaces Strata's
//! user stylesheet, `~/.config/strata/style.css` (lgse/strata `upstream/main` 35ee677ab), and
//! [`migrate_css`] converts what of one can be converted. `MIGRATION.md`, beside this file, is
//! the note for Strata's changelog (K4 and K5).
//!
//! # The file
//!
//! Every key is a token's name in the registry ([`crate::tokens`]), as a dotted key or under a
//! table, so `[list.row]` then `height = 30` sets `list.row.height`:
//!
//! ```toml
//! radius.md = 4
//! color.border_focus = "#ff9e64"
//!
//! [sidebar.row]
//! height = 32
//!
//! [dark]
//! color.bg_base = "#101018"
//!
//! [dark.list.row]
//! height = 34
//! ```
//!
//! - **Scopes.** Rows under `[light]` or `[dark]` apply only to a look of that appearance
//!   (`kit.dark`, which the theme picks). Unscoped rows apply first, then the matching scope,
//!   so a scoped row wins wherever it sits in the file. `light` and `dark` are reserved: no
//!   token's name starts with either. A slot scope such as `[list.row]` is just the start of
//!   its tokens' names.
//! - **Values**, by the token's kind:
//!   - a colour is a string in any form a theme file takes ([`Rgba8::parse`](crate::theme::Rgba8::parse):
//!     hex, `rgb()`, `rgba()`, `hsl()`, `hsla()`, CSS names, `transparent`), or one taken
//!     from another colour token so it follows the theme, as a stylesheet's `@strata_*`
//!     colours did: `"@color.text_base"`, or `"alpha(@color.text_base, 0.06)"` for it at a
//!     share of its opacity (a factor from 0 to 1; `alpha()` takes a literal colour too);
//!   - a length or a number is a TOML integer or float, in points for a length, inside the
//!     token's range ([`Token::range`](crate::tokens::Token::range));
//!   - a flag is `true` or `false`.
//! - **One row wins per token**: the one scoped to the look's appearance, else the unscoped
//!   one, whether either is a literal or a reference, and wherever each sits in the file.
//! - **References** (ruling Q2) read the look after the theme, the settings and the winning
//!   literal overrides, so `"@color.border_focus"` follows an override of `color.border_focus`.
//!   They never read another reference's result (references don't chain) nor a row that lost.
//! - **Errors.** Each unknown token, wrong kind, value out of range, bad colour and TOML
//!   syntax error is reported with its line and column ([`Overrides::errors`]). A row with an
//!   error doesn't apply, so its token keeps the theme's value; every other row applies, as
//!   Strata's stylesheet skipped only the rules GTK couldn't parse. A syntax error drops only
//!   its own line, or, for a broken table header, the header and its rows. A key set twice
//!   keeps its first value, and the second is reported, as TOML has it.
//!
//! Icons aren't tokens: which catalog a look draws from is the host's (K3), and the file name
//! → icon mapping is Strata's. A future file-type → icon table here must not be called
//! `icons`, which is IconsGrid's slot id (`icons.*`).
//!
//! # Building a look
//!
//! Overrides are step 3 of the look's build (`look.rs`): after the theme (K1, with Omarchy)
//! and the user's shape, density and type settings (K2), before the syntect theme (K1b).
//!
//! ```ignore
//! let next = overrides.applied(theme.look().with_params(&params));  // then the icon set (K3)
//! strata_kit::theme::switch(&mut state.look, next, tx);
//! ```
//!
//! Any change to the theme, the parameters or the file rebuilds the whole chain.
//! [`Look::apply_params`](crate::look::Look::apply_params) sets absolute values on the
//! fields it owns, so calling it on a look that already carries overrides drops the overrides
//! of those fields (`row.*`, `type.*`, `radius.*`, `glow.on`).
//!
//! What follows from the order:
//!
//! - An override is the final value. Corner roundness doesn't scale an overridden radius, as
//!   it didn't scale a radius in the user stylesheet.
//! - An override of a type style doesn't regrow the rows, which K2 sizes from the ramp before
//!   this step (`density.rs`).
//! - An override of `glow.on` beats Strata's Element glow setting, which then has no effect
//!   while the row stays.
//! - A `syntax.*` override reaches [`syntax::Role::color`](crate::theme::syntax::Role::color),
//!   and so the syntect theme built from it.
//!
//! # The host
//!
//! Nothing here reads a file. Strata reads `$XDG_CONFIG_HOME/strata/overrides.toml`, watches
//! it (Strata's stylesheet watch waited 75 ms for writes to settle), and hands the bytes to
//! [`Overrides::from_bytes`]. A missing or unreadable file is no overrides: pass empty bytes,
//! which restores the theme's look. When the result equals the current [`Overrides`], nothing
//! changed; otherwise the host rebuilds the look as above and moves to it with
//! [`theme::switch`](crate::theme::switch), so a reload animates as a theme switch does.
//!
//! Settings → Appearance keeps a row for the file (open it, the error count from
//! [`Overrides::errors`], Reset); it is C20c's row in Strata. On first launch Strata converts
//! an existing `style.css` with [`migrate_css`], writes the result, and lists the rules it
//! couldn't convert in a one-time notice. It never deletes the stylesheet.
//!
//! # Rulings
//!
//! - **Q1, icons.** No icon overrides here (above): the owner deferred plan §10 question 2.
//! - **Scripts.** K4 has no input behaviour, so it has no headless scripts, as K1 (its Q8):
//!   the unit tests name the feature-map bullets they check.
//! - **Q5, syntax errors.** The rows that survive one still apply.
//! - **Q2, references.** Colours may name another colour token and `alpha()` it, resolved
//!   against the look after the literal overrides, without chains.
//! - **Q6, glow.** An override of `glow.on` beats the Element glow setting.

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

/// A parsed override file: the rows that apply, and every problem found in it.
///
/// [`Overrides::default`] is no overrides, as a missing file is. Two parses of files that set
/// the same values at the same places compare equal, so a host can skip a rebuild when a
/// write changed nothing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overrides {
    rows: Vec<Row>,
    errors: Vec<OverrideError>,
}

impl Overrides {
    /// The overrides in `src`, a file's contents. Never fails: problems are in
    /// [`Overrides::errors`], and every row without one applies.
    pub fn parse(src: &str) -> Overrides {
        parse::parse(src)
    }

    /// The overrides in a file's bytes. A file that isn't UTF-8 is one error, and nothing in
    /// it applies.
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

    /// Sets every row's token on `look`.
    ///
    /// One row wins per token: the one scoped to `look`'s appearance, else the unscoped one.
    /// The winners' literal values are set first; then the winners' colours taken from other
    /// tokens, each read from the look as the literals left it, so references don't chain and
    /// never read a row that lost.
    ///
    /// `look` must be freshly built (the theme, then [`LookParams`](crate::shape::LookParams)):
    /// a reference reads the look it is applied to, so applying twice can change it again
    /// (`alpha(@color.scrim, 0.5)` halves the scrim each time). Rebuild, then apply.
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

    /// True when no row applies: an empty or missing file, or one whose every row has an
    /// error.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}
