//! Syntax colours: Strata's source palette, and the rune-kit [`SyntaxTheme`] it becomes.
//!
//! Strata colours code with a GtkSourceView scheme built from five theme colours plus the
//! dim text (`src/ui/theme/source_style.rs` at lgse/strata 35ee677ab). A theme that leaves a
//! syntax colour out gets one blended from its accent and text, as Strata's
//! `resolved_source_palette` does.
//!
//! # Kinds
//!
//! GtkSourceView's `def.lang` maps functions and built-ins onto identifiers, which Strata's
//! scheme draws in the text colour, and styles nothing else. So rune-kit's kinds map as:
//!
//! | rune-kit kind | Colour |
//! | --- | --- |
//! | `Keyword` | keyword (`def:statement`) |
//! | `String` | string |
//! | `Number` | constant (`def:constant`, numbers and special characters) |
//! | `Type` | type |
//! | `Comment` | dim text |
//! | `Function`, `Property`, `Parameter`, `Punctuation` | text |
//! | `Inserted`, `Deleted` | the kit's success and danger colours |
//!
//! Deliberate changes from Strata (owner rulings Q9):
//!
//! - Strata's scheme draws keywords and types bold, comments italic and the shebang bold.
//!   rune-kit highlights change colour only, never weight or style, so those are dropped.
//! - Strata's scheme leaves diff lines unstyled; rune-kit's diff kinds keep green and red,
//!   and so do [`Role::Inserted`] and [`Role::Deleted`].
//! - Strata's scheme draws errors (`def:error`) bold, in the background colour on a danger
//!   fill. C10a's spans change colour only, so [`Role::Error`] draws in the danger colour on
//!   the code's own background: the fill is dropped, and the background-coloured text with it.
//!
//! # Scopes
//!
//! [`SCOPES`] maps TextMate scope selectors, which syntect's grammars emit, onto a [`Role`],
//! following GtkSourceView's `def:` groups and the language files that map onto them, and
//! [`Role::color`] reads each role's colour from a finished [`Look`], so a user's `syntax.*`
//! override reaches it. [`syntect_theme`] builds C10a's syntect theme from both (K1b), and
//! [`editor`] gives the colours around the code.
//!
//! Known gaps, where these selectors don't follow Strata:
//!
//! - Rust's `let`, `const` and `Self` draw as types: the grammar scopes them
//!   `storage.type.rust`, as it does the primitive types, so no selector tells them apart.
//! - C's `struct`, `enum`, `union` and `typedef` draw as types: the grammar scopes them
//!   `storage.type.c`, as it does `int`, though c.lang lists them as keywords.
//! - C++'s `class`, `struct` and `typename` (`storage.type.c++`) and `template`
//!   (`storage.type.template.c++`) draw as types; cpp.lang lists them as keywords.
//! - C's number suffixes (`storage.type.numeric` inside a number) draw as types.
//! - TypeScript's `const`, `let` and `var` (`storage.type.ts`), `interface` and `namespace`
//!   draw as types; Strata's TypeScript files draw them as keywords.
//! - A variable or command substitution inside a shell string (`"$dir"`) draws as the
//!   string; sh.lang draws it as an identifier, in the text colour. Following it would need
//!   text-role rows that override an enclosing string.
//! - A Markdown fence's language name (`constant.other.language-name`) draws as a constant.

use std::str::FromStr;

use rune::Color;
use rune::color::srgb_encode;
use rune_kit::Kit;
use rune_kit::syntax::{Syntax, SyntaxTheme};
use syntect::highlighting::{ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings};

use super::color::{Rgba8, blend};
use super::colors::Colors;
use crate::look::{Look, SyntaxExtra};

#[cfg(test)]
mod tests;

/// The five colours code previews draw from, every one resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Palette {
    /// Keywords and operators (`def:statement`).
    pub statement: Rgba8,
    pub string: Rgba8,
    /// Numbers, booleans, special constants and escapes.
    pub constant: Rgba8,
    pub type_: Rgba8,
    pub preprocessor: Rgba8,
}

/// The theme's syntax colours, each left-out one blended from the accent toward the text.
pub fn resolve(colors: &Colors) -> Palette {
    resolve_over(colors, None)
}

/// [`resolve`], with left-out colours taken from `fallback` when there is one (an Omarchy
/// theme's terminal palette), else blended.
pub(crate) fn resolve_over(colors: &Colors, fallback: Option<Palette>) -> Palette {
    let (accent, text) = (colors.accent, colors.text);
    let f = fallback.unwrap_or(Palette {
        statement: accent,
        string: blend(accent, text, 0.48),
        constant: blend(accent, text, 0.18),
        type_: blend(accent, text, 0.24),
        preprocessor: blend(accent, text, 0.32),
    });
    let s = &colors.syntax;
    Palette {
        statement: s.keyword.unwrap_or(f.statement),
        string: s.string.unwrap_or(f.string),
        constant: s.constant.unwrap_or(f.constant),
        type_: s.type_.unwrap_or(f.type_),
        preprocessor: s.preprocessor.unwrap_or(f.preprocessor),
    }
}

/// The rune-kit syntax theme and Strata's extra kinds for `colors`, on the theme's `kit`.
pub(crate) fn themes(kit: &Kit, colors: &Colors) -> (SyntaxTheme, SyntaxExtra) {
    let p = resolve(colors);
    let text: Color = colors.text.color();
    let theme = SyntaxTheme::from_kit(kit)
        .with(Syntax::Keyword, p.statement.color())
        .with(Syntax::String, p.string.color())
        .with(Syntax::Number, p.constant.color())
        .with(Syntax::Type, p.type_.color())
        .with(Syntax::Comment, colors.dim_text.color())
        .with(Syntax::Function, text)
        .with(Syntax::Property, text)
        .with(Syntax::Parameter, text)
        .with(Syntax::Punctuation, text)
        // The kinds rune-kit added at `2a44156`, placed as [`SCOPES`] places their scopes:
        // `markup.heading` in the accent, links and regexes as strings, `keyword.operator`
        // as a keyword, attribute names as types, and variables as identifiers.
        .with(Syntax::Heading, kit.c.text_accent)
        .with(Syntax::Link, p.string.color())
        .with(Syntax::Regex, p.string.color())
        .with(Syntax::Operator, p.statement.color())
        .with(Syntax::Attribute, p.type_.color())
        .with(Syntax::Variable, text);
    let extra = SyntaxExtra {
        preprocessor: p.preprocessor.color(),
    };
    (theme, extra)
}

/// What colours a scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// Unstyled code: identifiers, functions, built-ins (GtkSourceView's `def:identifier`).
    Text,
    Comment,
    String,
    /// Numbers, booleans, special constants and escapes.
    Constant,
    /// Keywords, storage modifiers and operators (`def:statement`).
    Keyword,
    Type,
    /// Directives, imports and attributes.
    Preprocessor,
    /// Markup headings, in the accent, as Strata's Markdown headings are.
    Heading,
    /// Invalid code, in the danger colour (Strata's danger fill is dropped: see the module).
    Error,
    /// Added diff lines, in the kit's success colour.
    Inserted,
    /// Removed diff lines, in the kit's danger colour.
    Deleted,
}

impl Role {
    pub const ALL: [Role; 11] = [
        Role::Text,
        Role::Comment,
        Role::String,
        Role::Constant,
        Role::Keyword,
        Role::Type,
        Role::Preprocessor,
        Role::Heading,
        Role::Error,
        Role::Inserted,
        Role::Deleted,
    ];

    /// The role's colour in `look`, after the user's overrides.
    pub fn color(self, look: &Look) -> Color {
        let s = &look.syntax;
        match self {
            Role::Text => look.kit.c.text_base,
            Role::Comment => s.color(Syntax::Comment),
            Role::String => s.color(Syntax::String),
            Role::Constant => s.color(Syntax::Number),
            Role::Keyword => s.color(Syntax::Keyword),
            Role::Type => s.color(Syntax::Type),
            Role::Preprocessor => look.syntax_extra.preprocessor,
            Role::Heading => look.kit.c.text_accent,
            Role::Error => look.kit.c.danger.fg,
            Role::Inserted => s.color(Syntax::Inserted),
            Role::Deleted => s.color(Syntax::Deleted),
        }
    }
}

/// TextMate scope selectors and their roles. A scope not listed draws as [`Role::Text`];
/// the most specific match wins, as syntect matches them.
pub const SCOPES: &[(&str, Role)] = &[
    ("comment", Role::Comment),
    ("punctuation.definition.comment", Role::Comment),
    ("string", Role::String),
    ("markup.underline.link", Role::String),
    ("string.other.link", Role::String),
    ("constant", Role::Constant),
    ("constant.numeric", Role::Constant),
    ("constant.language", Role::Constant),
    ("constant.character", Role::Constant),
    ("constant.character.escape", Role::Constant),
    ("constant.other", Role::Constant),
    ("support.constant", Role::Constant),
    ("keyword", Role::Keyword),
    ("keyword.operator", Role::Keyword),
    ("storage", Role::Keyword),
    ("storage.modifier", Role::Keyword),
    ("storage.type", Role::Type),
    // Declaration keywords some grammars file under `storage.type`: Strata's language files
    // list them as keywords (`def:keyword`, then `def:statement`), as rust.lang and js.lang do
    // `fn`, `struct`, `impl`, `class`, `function` and `const`.
    ("storage.type.function", Role::Keyword),
    ("storage.type.class", Role::Keyword),
    ("storage.type.struct", Role::Keyword),
    ("storage.type.enum", Role::Keyword),
    ("storage.type.trait", Role::Keyword),
    ("storage.type.impl", Role::Keyword),
    ("storage.type.module", Role::Keyword),
    ("storage.type.type", Role::Keyword),
    ("storage.type.extends", Role::Keyword),
    // JavaScript has no type keywords: Babel's `storage.type.js` is `const`, `let`, `async`.
    ("storage.type.js", Role::Keyword),
    // Go's `func`, `type`, `struct`, `interface`, `var` and `const`, which go.lang lists as
    // keywords; it lists `map` and `chan` as types.
    ("storage.type.keyword", Role::Keyword),
    ("storage.type.keyword.map", Role::Type),
    ("storage.type.keyword.chan", Role::Type),
    // String prefixes (`r""`, `b""`, `f""`) are part of the string in GtkSourceView.
    ("storage.type.string", Role::String),
    ("entity.name.type", Role::Type),
    ("entity.name.class", Role::Type),
    ("entity.other.inherited-class", Role::Type),
    ("support.type", Role::Type),
    ("support.class", Role::Type),
    // xml.lang's `attribute-name`, which html.lang uses, maps to `def:type`, as css.lang's
    // `attribute-selector-name` (`[href]`) does. The CSS grammar files class and id selectors
    // under attribute names too: css.lang's `class-selector` is `def:identifier`, in the text
    // colour, and its `id-selector` is `def:statement`.
    (
        "entity.other.attribute-name - entity.other.attribute-name.class.css",
        Role::Type,
    ),
    ("entity.other.attribute-name.id.css", Role::Keyword),
    ("meta.preprocessor", Role::Preprocessor),
    ("keyword.control.import", Role::Preprocessor),
    ("keyword.control.directive", Role::Preprocessor),
    ("punctuation.definition.directive", Role::Preprocessor),
    ("meta.attribute", Role::Preprocessor),
    ("meta.annotation", Role::Preprocessor),
    ("markup.heading", Role::Heading),
    ("entity.name.section", Role::Heading),
    ("invalid", Role::Error),
    ("markup.inserted", Role::Inserted),
    ("markup.deleted", Role::Deleted),
];

/// The syntect theme for `look`, after the user's overrides: each [`SCOPES`] selector in its
/// role's colour, the most specific match winning. The default foreground is the look's
/// text, so a scope in [`Role::Text`] gets no span and draws in the host's own text colour.
/// The settings take [`editor`]'s colours; C10a reads only the foregrounds.
///
/// Build a new one when the look changes and hand it to C10a's `Highlighter::set_theme`, which
/// re-colours open texts without re-parsing them.
pub fn syntect_theme(look: &Look) -> Theme {
    let scopes = SCOPES
        .iter()
        .filter_map(|&(selector, role)| {
            Some(ThemeItem {
                scope: ScopeSelectors::from_str(selector).ok()?,
                style: StyleModifier {
                    foreground: Some(to_syntect(role.color(look))),
                    background: None,
                    font_style: None,
                },
            })
        })
        .collect();
    let e = editor(look);
    Theme {
        name: Some("Strata".into()),
        author: None,
        settings: ThemeSettings {
            foreground: Some(to_syntect(e.foreground)),
            background: Some(to_syntect(e.background)),
            caret: Some(to_syntect(e.caret)),
            selection: Some(to_syntect(e.selection)),
            selection_foreground: Some(to_syntect(e.selection_foreground)),
            gutter: Some(to_syntect(e.gutter_background)),
            gutter_foreground: Some(to_syntect(e.gutter)),
            line_highlight: Some(to_syntect(e.line_highlight)),
            ..ThemeSettings::default()
        },
        scopes,
    }
}

/// A Rune colour (linear light) as a syntect colour (sRGB bytes, straight alpha).
fn to_syntect(c: Color) -> syntect::highlighting::Color {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    syntect::highlighting::Color {
        r: byte(srgb_encode(c.r)),
        g: byte(srgb_encode(c.g)),
        b: byte(srgb_encode(c.b)),
        a: byte(c.a),
    }
}

/// The colours around code, as Strata's source scheme sets them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Editor {
    pub foreground: Color,
    /// Code sits on the surface.
    pub background: Color,
    pub caret: Color,
    pub selection: Color,
    /// Selected text, on the selection.
    pub selection_foreground: Color,
    pub gutter: Color,
    pub gutter_background: Color,
    pub line_highlight: Color,
}

/// The colours around code in `look`: text on the surface, an accent caret and selection
/// with background-coloured text, dim line numbers and a background-coloured current line.
pub fn editor(look: &Look) -> Editor {
    let c = &look.kit.c;
    Editor {
        foreground: c.text_base,
        background: c.layer_01,
        caret: c.text_accent,
        selection: c.bg_accent,
        selection_foreground: c.text_inverse,
        gutter: c.text_muted,
        gutter_background: c.bg_base,
        line_highlight: c.bg_base,
    }
}
