// SPDX-License-Identifier: MIT

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
    ("storage.type.keyword", Role::Keyword),
    ("storage.type.keyword.map", Role::Type),
    ("storage.type.keyword.chan", Role::Type),
    ("storage.type.string", Role::String),
    ("entity.name.type", Role::Type),
    ("entity.name.class", Role::Type),
    ("entity.other.inherited-class", Role::Type),
    ("support.type", Role::Type),
    ("support.class", Role::Type),
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
