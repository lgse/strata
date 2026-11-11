//! Converting a user stylesheet (`~/.config/strata/style.css`) into override rows.
//!
//! [`migrate_css`] is a pure function: Strata reads the stylesheet on first launch, writes
//! [`Migration::toml`] to `overrides.toml`, lists [`Migration::unconverted`] in a one-time
//! notice, and leaves the stylesheet in place.
//!
//! # What converts
//!
//! A declaration converts only where a token stands for exactly what it styled: a selector
//! whose token styles the same elements and nothing more, checked against Strata's
//! `src/style.css` and the code that sets its classes (lgse/strata `upstream/main` 35ee677ab,
//! line numbers there), and a plain colour.
//!
//! | Selector | Property | Token | Strata's rule |
//! | --- | --- | --- | --- |
//! | `.sidebar-row.active` | `background`, `background-color` | `sidebar.selected.fill` | 2378: `alpha(@strata_accent, 0.18)` |
//! | `.sidebar-row.active image` | `color` | `sidebar.selected.icon` | 2386: `@strata_accent` |
//!
//! Strata sets `.active` on the current row's button of every kind, places, pins and device
//! rows, in the full sidebar and the rail alike (`set_sidebar_row_active` in
//! `src/ui/window/sidebar_rows.rs`), and the user stylesheet's priority beats the
//! `.sidebar-device.active > .sidebar-row` rule that clears a device row's own fill. C8 fills
//! and tints the current row of every kind from these two slots, and nothing else.
//!
//! Colours convert from the forms a theme file takes ([`Rgba8::parse`]), from Strata's
//! `@strata_*` colours, as references to the tokens K1 maps each onto ([`STRATA_COLORS`]), and
//! from `alpha()` of either, so a converted colour keeps following the theme. Within a
//! selector list, each selector converts on its own. As in CSS, the later declaration for a
//! token wins: when it doesn't convert, the earlier converted one is dropped too, and both
//! are listed.
//!
//! Near matches are listed with their own reason ([`Reason::Wider`]), because their token
//! styles more than the selector did (ruling B4):
//!
//! - `.file-list row` margins and radius (`collection.row.inset`, `.radius`): Strata sets
//!   `.file-list` on Miller columns, folder peek and inline search only, where List uses
//!   `.file-list-mode`, and the collection slots style every view's rows;
//! - `.sidebar-row:hover` (`sidebar.hover.fill`): the slot fills the sidebar's headings too,
//!   which Strata styles separately.
//!
//! Others are left out because their token isn't the same thing:
//!
//! - `min-height` on `.file-row`, `.list-row` or `.sidebar-row`: a minimum, where `row.*` and
//!   the row slots are fixed heights that K2 grows with the text size;
//! - the `.file-row > *` and `.list-row > *` margins: Content density's pad, which sizes the
//!   rows when K2 builds the look, so an override after it wouldn't regrow them;
//! - `window { background }`: `color.bg_base` is the file columns' fill too;
//! - `.search-result` and `.browser-tab`: no slot holds their fills or padding yet.
//!
//! Everything else is listed as unconverted with its location and a [`Reason`]: other
//! selectors (including `.sidebar-shell` from Strata's own documentation), other properties,
//! `shade()`, `mix()`, `lighter()`, `!important`, `@define-color` (a theme colour belongs in a
//! custom theme) and every other at-rule.

use std::fmt;
use std::ops::Range;

use super::parse::Lines;
use crate::theme::{Location, Rgba8};
use crate::tokens::{self, Token};

#[cfg(test)]
mod tests;

/// What [`migrate_css`] made of a stylesheet.
#[derive(Clone, Debug, PartialEq)]
pub struct Migration {
    /// The override file: one row per token, in the registry's order, each with the line it
    /// came from. Empty when nothing converted.
    pub toml: String,
    /// How many rows the file has, one per token. A later declaration for the same token
    /// replaces the earlier row, as the later one wins in CSS; a later one that doesn't
    /// convert removes it, and both are listed.
    pub converted: usize,
    /// Everything that didn't convert, in the order it appears.
    pub unconverted: Vec<Unconverted>,
}

/// A rule or declaration [`migrate_css`] left alone.
#[derive(Clone, Debug, PartialEq)]
pub struct Unconverted {
    /// Where it starts in the stylesheet.
    pub at: Location,
    /// The selector, or the at-rule's prelude, as written (white space collapsed).
    pub selector: String,
    /// The declaration, when only it didn't convert: `color: red`.
    pub declaration: Option<String>,
    pub reason: Reason,
}

/// Why a rule or declaration didn't convert.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Reason {
    /// No token stands for anything this selector styles.
    Selector,
    /// The selector has tokens, but none for this property.
    Property,
    /// The value isn't a plain colour.
    Value,
    /// The token this selector would set also styles elements the selector doesn't.
    Wider,
    /// It converted, but a later declaration for the same token didn't, and in CSS the later
    /// one wins.
    Superseded,
    /// `@define-color` redefines a theme colour.
    DefineColor,
    /// Another at-rule (`@media`, `@import`, …).
    AtRule,
    /// Not valid CSS: an unclosed block, a declaration without a colon, a stray brace.
    Syntax,
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Reason::Selector => "no token stands for this selector",
            Reason::Property => "no token stands for this property here",
            Reason::Value => "the value isn't a plain colour",
            Reason::Wider => "its token also styles elements this selector doesn't",
            Reason::Superseded => {
                "a later declaration for the same token didn't convert, and the later one wins"
            }
            Reason::DefineColor => "it redefines a theme colour: make a custom theme instead",
            Reason::AtRule => "at-rules don't convert",
            Reason::Syntax => "it isn't valid CSS",
        })
    }
}

impl fmt::Display for Unconverted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}, column {}: `{}`",
            self.at.line, self.at.column, self.selector
        )?;
        if let Some(d) = &self.declaration {
            write!(f, " {{ {d} }}")?;
        }
        write!(f, ": {}", self.reason)
    }
}

/// The properties the table's selectors take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prop {
    /// `background` or `background-color`.
    Background,
    /// `color`.
    Color,
    /// `border-radius`.
    Radius,
    /// `margin`, `margin-left` or `margin-right`.
    Margin,
}

impl Prop {
    fn of(property: &str) -> Option<Prop> {
        Some(match property {
            "background" | "background-color" => Prop::Background,
            "color" => Prop::Color,
            "border-radius" => Prop::Radius,
            "margin" | "margin-left" | "margin-right" => Prop::Margin,
            _ => return None,
        })
    }
}

/// The selectors that convert: each with a colour property and the token it sets. See the
/// table in the module docs.
const RULES: &[(&str, Prop, &str)] = &[
    (
        ".sidebar-row.active",
        Prop::Background,
        "sidebar.selected.fill",
    ),
    (
        ".sidebar-row.active image",
        Prop::Color,
        "sidebar.selected.icon",
    ),
];

/// Near matches whose token styles more than the selector did, so they don't convert
/// (ruling B4): listed so they get their own reason.
const WIDER: &[(&str, Prop, &str)] = &[
    (".file-list row", Prop::Margin, "collection.row.inset"),
    (".file-list row", Prop::Radius, "collection.row.radius"),
    (".sidebar-row:hover", Prop::Background, "sidebar.hover.fill"),
];

/// Converts what of `css` can be converted into override rows, and lists the rest.
pub fn migrate_css(css: &str) -> Migration {
    let text = strip_comments(css);
    let mut out = Out {
        css,
        lines: Lines::new(css),
        rows: Vec::new(),
        unconverted: Vec::new(),
    };
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let Some(stop) = find(b, i, b"{;}") else {
            out.skip(i, collapse(&text[i..]), None, Reason::Syntax);
            break;
        };
        let prelude = collapse(&text[i..stop]);
        match b[stop] {
            b'{' => {
                let Some((close, nested)) = block_end(b, stop + 1) else {
                    out.skip(i, prelude, None, Reason::Syntax);
                    break;
                };
                if prelude.starts_with('@') {
                    out.skip(i, prelude, None, Reason::AtRule);
                } else if nested {
                    out.skip(i, prelude, None, Reason::Syntax);
                } else {
                    out.rule(&text, i..stop, stop + 1..close);
                }
                i = close + 1;
            }
            b';' => {
                let reason = if prelude.starts_with("@define-color") {
                    Reason::DefineColor
                } else if prelude.starts_with('@') {
                    Reason::AtRule
                } else {
                    Reason::Syntax
                };
                out.skip(i, prelude, None, reason);
                i = stop + 1;
            }
            _ => {
                out.skip(i, "}".into(), None, Reason::Syntax);
                i = stop + 1;
            }
        }
    }
    out.finish()
}

/// A converted row: the token, its value as TOML writes it, and the declaration it came
/// from, which is listed if a later one supersedes it.
struct Converted {
    token: &'static Token,
    value: String,
    at: usize,
    selector: String,
    declaration: String,
}

struct Out<'a> {
    css: &'a str,
    lines: Lines,
    rows: Vec<Converted>,
    unconverted: Vec<Unconverted>,
}

impl Out<'_> {
    fn skip(&mut self, at: usize, selector: String, declaration: Option<String>, reason: Reason) {
        self.unconverted.push(Unconverted {
            at: self.lines.locate(self.css, at),
            selector,
            declaration,
            reason,
        });
    }

    /// Converts a declaration: it replaces an earlier one for the same token, as the later
    /// declaration wins in CSS.
    fn set(&mut self, name: &'static str, value: String, at: usize, selector: &str, decl: &str) {
        let token = tokens::find(name).expect("RULES names registry tokens");
        self.rows.retain(|r| r.token.name() != name);
        self.rows.push(Converted {
            token,
            value,
            at,
            selector: selector.to_owned(),
            declaration: decl.to_owned(),
        });
    }

    /// A declaration for `name` that doesn't convert: in CSS it wins over an earlier one that
    /// did, so that row goes too, and is listed.
    fn supersede(&mut self, name: &str) {
        if let Some(i) = self.rows.iter().position(|r| r.token.name() == name) {
            let r = self.rows.remove(i);
            self.skip(r.at, r.selector, Some(r.declaration), Reason::Superseded);
        }
    }

    /// One rule: `text[prelude]` is its selector list, `text[body]` its declarations.
    fn rule(&mut self, text: &str, prelude: Range<usize>, body: Range<usize>) {
        let decls = split(text, body, b';');
        for (at, selector) in split(text, prelude, b',') {
            let rules: Vec<_> = RULES.iter().filter(|r| r.0 == selector).collect();
            let wider: Vec<_> = WIDER.iter().filter(|r| r.0 == selector).collect();
            if rules.is_empty() && wider.is_empty() {
                self.skip(at, selector, None, Reason::Selector);
                continue;
            }
            for (at, decl) in &decls {
                let (at, unconverted) =
                    (*at, |reason| (selector.clone(), Some(decl.clone()), reason));
                let Some((property, value)) = decl.split_once(':') else {
                    let (s, d, r) = unconverted(Reason::Syntax);
                    self.skip(at, s, d, r);
                    continue;
                };
                let prop = Prop::of(&property.trim().to_ascii_lowercase());
                let reason = match prop.and_then(|p| rules.iter().find(|r| r.1 == p)) {
                    Some(&&(_, _, token)) => match color(value) {
                        Some(c) => {
                            self.set(token, format!("{c:?}"), at, &selector, decl);
                            continue;
                        }
                        None => {
                            self.supersede(token);
                            Reason::Value
                        }
                    },
                    None if prop.is_some_and(|p| wider.iter().any(|r| r.1 == p)) => Reason::Wider,
                    None => Reason::Property,
                };
                let (s, d, r) = unconverted(reason);
                self.skip(at, s, d, r);
            }
        }
    }

    fn finish(mut self) -> Migration {
        self.unconverted.sort_by_key(|u| (u.at.line, u.at.column));
        let order = |t: &Token| tokens::all().position(|r| r.name() == t.name());
        self.rows.sort_by_key(|r| order(r.token));
        let mut toml = String::new();
        let mut table = "";
        for r in &self.rows {
            let name = r.token.name();
            let (prefix, key) = name.rsplit_once('.').unwrap_or(("", name));
            if toml.is_empty() {
                toml.push_str(
                    "# Converted from a user stylesheet (style.css): each row names the line it \
                     came from.\n",
                );
            }
            if prefix != table {
                toml.push_str(&format!("\n[{prefix}]\n"));
                table = prefix;
            }
            let line = self.lines.locate(self.css, r.at).line;
            toml.push_str(&format!("{key} = {} # style.css line {line}\n", r.value));
        }
        Migration {
            toml,
            converted: self.rows.len(),
            unconverted: self.unconverted,
        }
    }
}

/// Strata's stylesheet colours (`@define-color strata_*` in `src/ui/theme/generated_css.rs`,
/// and `@strata_glow`) and the token K1 maps each theme colour onto (`theme/map.rs`).
pub(super) const STRATA_COLORS: &[(&str, &str)] = &[
    ("strata_bg", "color.bg_base"),
    ("strata_surface", "color.bg_deep"),
    ("strata_text", "color.text_base"),
    ("strata_accent", "color.bg_accent"),
    ("strata_danger", "color.danger.fg"),
    ("strata_muted", "fill.muted"),
    ("strata_highlight", "fill.highlight"),
    ("strata_border", "color.std_border"),
    ("strata_dim_text", "color.text_muted"),
    ("strata_glow", "glow.color"),
];

/// A colour value as the override file writes it: a literal's hex, a reference to the token
/// a `@strata_*` colour maps onto, or `alpha()` of either.
fn color(value: &str) -> Option<String> {
    let v = value.trim();
    let alpha_args = v
        .get(..6)
        .filter(|f| f.eq_ignore_ascii_case("alpha("))
        .and_then(|_| v[6..].strip_suffix(')'));
    if let Some(args) = alpha_args {
        let (inner, factor) = args.rsplit_once(',')?;
        let factor = factor.trim().parse::<f32>().ok()? + 0.0;
        return (0.0..=1.0)
            .contains(&factor)
            .then(|| Some(format!("alpha({}, {factor})", operand(inner.trim())?)))?;
    }
    operand(v)
}

/// A literal colour as hex, or a `@strata_*` colour as a reference.
fn operand(v: &str) -> Option<String> {
    match v.strip_prefix('@') {
        Some(name) => STRATA_COLORS
            .iter()
            .find(|(strata, _)| *strata == name)
            .map(|(_, token)| format!("@{token}")),
        None => Rgba8::parse(v).map(Rgba8::hex),
    }
}

/// `css` with every comment written over in spaces, keeping line breaks, so offsets still
/// point into the original.
fn strip_comments(css: &str) -> String {
    let mut b = css.as_bytes().to_vec();
    let mut i = 0;
    let mut quote = None;
    while i < b.len() {
        match (quote, b[i]) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), b'\\') => i += 1,
            (Some(_), _) => {}
            (None, c @ (b'"' | b'\'')) => quote = Some(c),
            (None, b'/') if b.get(i + 1) == Some(&b'*') => {
                let end = css[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
                for c in &mut b[i..end] {
                    if *c != b'\n' {
                        *c = b' ';
                    }
                }
                i = end;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    String::from_utf8(b).expect("spaces over whole comments keep UTF-8")
}

/// The first of `stops` at or after `from`, outside quotes and parentheses.
fn find(b: &[u8], from: usize, stops: &[u8]) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut i = from;
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) if c == b'\\' => i += 1,
            Some(_) => {}
            None => match c {
                b'"' | b'\'' => quote = Some(c),
                b'(' => depth += 1,
                b')' => depth = depth.saturating_sub(1),
                _ if depth == 0 && stops.contains(&c) => return Some(i),
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// The `}` closing a block whose body starts at `from`, and whether a block nests inside it.
fn block_end(b: &[u8], from: usize) -> Option<(usize, bool)> {
    let mut depth = 1;
    let mut nested = false;
    let mut i = from;
    while let Some(at) = find(b, i, b"{}") {
        if b[at] == b'{' {
            depth += 1;
            nested = true;
        } else {
            depth -= 1;
            if depth == 0 {
                return Some((at, nested));
            }
        }
        i = at + 1;
    }
    None
}

/// `text[range]` split on `sep` outside quotes and parentheses: each part's start and its
/// text with white space collapsed, empty parts left out.
fn split(text: &str, range: Range<usize>, sep: u8) -> Vec<(usize, String)> {
    let b = text.as_bytes();
    let mut parts = Vec::new();
    let mut start = range.start;
    loop {
        let end = find(&b[..range.end], start, &[sep]).unwrap_or(range.end);
        let part = &text[start..end];
        let lead = part.len() - part.trim_start().len();
        if !part.trim().is_empty() {
            parts.push((start + lead, collapse(part)));
        }
        if end >= range.end {
            return parts;
        }
        start = end + 1;
    }
}

/// `s` trimmed, with each run of white space as one space.
fn collapse(s: &str) -> String {
    s.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}
