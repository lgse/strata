//! The token registry: `get` and `set` by dotted name over the `Kit` tokens, the extensions
//! and the component slots.
//!
//! Work package: W2, `docs/work/phase-0-groundwork/W2-look-contract.md`.
//!
//! K4's override file names tokens as this registry does, so it can set any of them without
//! knowing which component reads it. A name is dot-separated snake_case segments
//! (`[a-z0-9_]+`), at least two of them:
//!
//! | Names | Kind | Field |
//! | --- | --- | --- |
//! | `color.<field>` | colour | every `kit.c` colour, such as `color.bg_base` |
//! | `color.<role>.<part>` | colour | `kit.c.{success,warning,danger,info}.{bg,fg,border,icon}` |
//! | `radius.{xs,sm,md,lg,xl}` | length | `kit.radius` |
//! | `type.<style>.{size,line_height}` | length | `kit.ty`'s styles, then `look.ty`'s (`title`, `heading`) |
//! | `type.<style>.weight` | number, 1–1000 | the same styles |
//! | `type.<style>.tracking` | number, −10–10 pt | the same styles |
//! | `space.{xxs,xs,sm,md,lg,xl,xxl}` | length | `look.space` |
//! | `frame.{inset,bar,control}` | length | `look.frame` |
//! | `content.pad` | length | `look.content` |
//! | `row.{list,columns,sidebar,palette}` | length | `look.row` |
//! | `icon_size.{sm,md,lg}` | length | `look.icon_size` |
//! | `glow.on` | flag | `look.glow.on` |
//! | `glow.color` | colour | `look.glow.color` |
//! | `glow.radius` | length | `look.glow.radius` |
//! | `glow.opacity` | number, 0–1 | `look.glow.opacity` |
//! | `fill.{muted,highlight}` | colour | `look.fill` |
//! | `syntax.<kind>` | colour | `look.syntax`, one per `rune_kit::syntax::Syntax` kind |
//! | `syntax.preprocessor` | colour | `look.syntax_extra.preprocessor` |
//! | `<slot>.<part>.<property>` | any | a component slot ([`crate::slots`]), such as `list.row.height`: exactly three segments, the slot's id first |
//!
//! Lengths are in points: finite and not negative.
//!
//! Some things are deliberately not tokens:
//!
//! - roundness, density and text size are K2's settings, and the registry holds the tokens
//!   they produce;
//! - the animations switch is a setting;
//! - the tile and thumbnail ladders are lists;
//! - Kit's elevation shadows, glyphs and motion springs aren't settable here yet.
//!
//! Adding any of them later is an addition.

use std::fmt;

use rune::Color;
use rune_kit::syntax::Syntax;
use rune_kit::{Kit, Palette, Radius, StateColors, Type};

use crate::look::{Content, Fills, Frame, IconSizes, Look, Ramp, RowHeights, Space, SyntaxExtra};
use crate::slots;

#[cfg(test)]
mod tests;

/// What a token holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kind {
    Color,
    /// Points: finite and not negative.
    Length,
    /// A unitless number in the token's range ([`Token::range`]).
    Number,
    /// On or off.
    Flag,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Color => "a colour",
            Kind::Length => "a length",
            Kind::Number => "a number",
            Kind::Flag => "true or false",
        })
    }
}

/// A token's value.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Value {
    Color(Color),
    Length(f32),
    Number(f32),
    Flag(bool),
}

impl Value {
    pub fn kind(&self) -> Kind {
        match self {
            Value::Color(_) => Kind::Color,
            Value::Length(_) => Kind::Length,
            Value::Number(_) => Kind::Number,
            Value::Flag(_) => Kind::Flag,
        }
    }
}

/// Why a token couldn't be read or set. A failed `set` leaves the look unchanged.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TokenError {
    /// No token has this name.
    Unknown(String),
    /// The value is of another kind than the token's.
    Kind {
        name: &'static str,
        expected: Kind,
        got: Kind,
    },
    /// The number is outside the token's range, or not finite.
    Range {
        name: &'static str,
        min: f32,
        max: f32,
    },
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenError::Unknown(name) => write!(f, "unknown token `{name}`"),
            TokenError::Kind {
                name,
                expected,
                got,
            } => write!(f, "`{name}` takes {expected}, not {got}"),
            TokenError::Range { name, min, max } => {
                if max.is_infinite() {
                    write!(f, "`{name}` takes a finite number of at least {min}")
                } else {
                    write!(f, "`{name}` takes a number from {min} to {max}")
                }
            }
        }
    }
}

impl std::error::Error for TokenError {}

#[derive(Clone, Copy)]
enum Access {
    Color(fn(&Look) -> Color, fn(&mut Look, Color)),
    Number(fn(&Look) -> f32, fn(&mut Look, f32)),
    Flag(fn(&Look) -> bool, fn(&mut Look, bool)),
}

/// One named token: how to read it from a [`Look`] and how to set it.
///
/// A slot registers its tokens in its own file with these constructors. `get` returns what a
/// component would draw: for a slot token that nothing set, the value it inherits.
#[derive(Clone, Copy)]
pub struct Token {
    name: &'static str,
    kind: Kind,
    min: f32,
    max: f32,
    access: Access,
}

impl Token {
    pub const fn color(
        name: &'static str,
        get: fn(&Look) -> Color,
        set: fn(&mut Look, Color),
    ) -> Token {
        Token {
            name,
            kind: Kind::Color,
            min: 0.0,
            max: 0.0,
            access: Access::Color(get, set),
        }
    }

    /// A length in points: finite and not negative.
    pub const fn length(
        name: &'static str,
        get: fn(&Look) -> f32,
        set: fn(&mut Look, f32),
    ) -> Token {
        Token {
            name,
            kind: Kind::Length,
            min: 0.0,
            max: f32::INFINITY,
            access: Access::Number(get, set),
        }
    }

    /// A unitless number from `min` to `max`, both included.
    pub const fn number(
        name: &'static str,
        min: f32,
        max: f32,
        get: fn(&Look) -> f32,
        set: fn(&mut Look, f32),
    ) -> Token {
        Token {
            name,
            kind: Kind::Number,
            min,
            max,
            access: Access::Number(get, set),
        }
    }

    pub const fn flag(
        name: &'static str,
        get: fn(&Look) -> bool,
        set: fn(&mut Look, bool),
    ) -> Token {
        Token {
            name,
            kind: Kind::Flag,
            min: 0.0,
            max: 0.0,
            access: Access::Flag(get, set),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The values a length or number takes, both ends included. `None` for colours and flags.
    pub fn range(&self) -> Option<(f32, f32)> {
        matches!(self.kind, Kind::Length | Kind::Number).then_some((self.min, self.max))
    }

    pub fn get(&self, look: &Look) -> Value {
        match (self.access, self.kind) {
            (Access::Color(get, _), _) => Value::Color(get(look)),
            (Access::Number(get, _), Kind::Length) => Value::Length(get(look)),
            (Access::Number(get, _), _) => Value::Number(get(look)),
            (Access::Flag(get, _), _) => Value::Flag(get(look)),
        }
    }

    /// Sets the token, or leaves `look` unchanged and says why not.
    pub fn set(&self, look: &mut Look, value: Value) -> Result<(), TokenError> {
        let wrong_kind = || TokenError::Kind {
            name: self.name,
            expected: self.kind,
            got: value.kind(),
        };
        if value.kind() != self.kind {
            return Err(wrong_kind());
        }
        match (self.access, value) {
            (Access::Color(_, set), Value::Color(c)) => set(look, c),
            (Access::Number(_, set), Value::Length(x) | Value::Number(x)) => {
                if !(x.is_finite() && self.min <= x && x <= self.max) {
                    return Err(TokenError::Range {
                        name: self.name,
                        min: self.min,
                        max: self.max,
                    });
                }
                set(look, x);
            }
            (Access::Flag(_, set), Value::Flag(b)) => set(look, b),
            _ => return Err(wrong_kind()),
        }
        Ok(())
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Token")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// Every registered token: the core tokens, then each slot's.
pub fn all() -> impl Iterator<Item = &'static Token> {
    CORE.iter()
        .flat_map(|group| group.iter())
        .chain(slots::TOKENS.iter().flat_map(|(_, tokens)| tokens.iter()))
}

/// The token named `name`.
pub fn find(name: &str) -> Option<&'static Token> {
    all().find(|t| t.name == name)
}

/// The value of the token named `name`.
pub fn get(look: &Look, name: &str) -> Result<Value, TokenError> {
    find(name)
        .map(|t| t.get(look))
        .ok_or_else(|| TokenError::Unknown(name.to_owned()))
}

/// Sets the token named `name`, or leaves `look` unchanged and says why not.
pub fn set(look: &mut Look, name: &str, value: Value) -> Result<(), TokenError> {
    find(name)
        .ok_or_else(|| TokenError::Unknown(name.to_owned()))?
        .set(look, value)
}

/// Every core token, by group. A slot's tokens are in `slots::TOKENS`.
pub(crate) const CORE: &[&[Token]] = &[
    COLORS, STATES, RADII, TYPE, SPACE, FRAME, CONTENT, ROW, ICON_SIZE, GLOW, FILL, SYNTAX,
];

/// Names every `Kit` field, so a Rune bump that adds one fails to build until it is either
/// registered or listed here as not a token:
///
/// - `c`, `ty` and `radius` are tokens: `color.*`, `type.*` and `radius.*`;
/// - `dark` is the appearance, which the theme picks, not a value to override;
/// - `elevation` (shadow lists), `glyph` (icons) and `motion` (springs) have no token kind
///   yet; adding one is an addition;
/// - `blend_over`, `ring` and `invalid_text` are how the theme is painted, not tokens.
const _: fn(Kit) = |Kit {
                        dark: _,
                        c: _,
                        elevation: _,
                        glyph: _,
                        ty: _,
                        radius: _,
                        motion: _,
                        blend_over: _,
                        ring: _,
                        invalid_text: _,
                    }| {};

/// `color.<field>` for each `kit.c` colour. The pattern names every `Palette` field, so a
/// Rune bump that adds one fails to build until it is registered here.
macro_rules! colors {
    ($($field:ident),* $(,)?) => {{
        const _: fn(Palette) = |Palette { $($field: _,)* success: _, warning: _, danger: _, info: _ }| {};
        &[$(Token::color(
            concat!("color.", stringify!($field)),
            |l| l.kit.c.$field,
            |l, v| l.kit.c.$field = v,
        ),)*]
    }};
}

const COLORS: &[Token] = colors!(
    page,
    bg_base,
    bg_deep,
    layer_01,
    layer_02,
    layer_03,
    layer_04,
    bg_inverse,
    bg_contrast,
    bg_button_neutral,
    bg_icon_button_contrast,
    bg_accent,
    submit_highlight,
    submit_hover,
    submit_fg,
    submit_icon,
    text_base,
    text_muted,
    text_faint,
    text_inverse,
    text_contrast,
    text_accent,
    icon_base,
    icon_muted,
    icon_faint,
    icon_inverse,
    icon_contrast,
    icon_accent,
    border_muted,
    border_base,
    border_strong,
    border_focus,
    hover,
    pressed,
    contrast_hover,
    contrast_pressed,
    scrim,
    depth_top,
    depth_bottom,
    white,
    black,
    knob,
    knob_on,
    surface_base,
    surface_weak,
    surface_hover,
    surface_raised,
    surface_raised_base,
    surface_raised_hover,
    surface_float,
    surface_disabled,
    border_weak,
    border_hover,
    border_active,
    border_disabled,
    text_strong,
    text_weak,
    text_invert_strong,
    icon_weaker,
    std_border,
    std_icon,
    std_icon_invert,
    std_icon_strong,
    std_text,
);

/// `color.<role>.<part>` for each status role.
macro_rules! states {
    ($($role:ident),* $(,)?) => {{
        const _: fn(StateColors) = |StateColors { bg: _, fg: _, border: _, icon: _ }| {};
        &[$(
            Token::color(concat!("color.", stringify!($role), ".bg"), |l| l.kit.c.$role.bg, |l, v| l.kit.c.$role.bg = v),
            Token::color(concat!("color.", stringify!($role), ".fg"), |l| l.kit.c.$role.fg, |l, v| l.kit.c.$role.fg = v),
            Token::color(concat!("color.", stringify!($role), ".border"), |l| l.kit.c.$role.border, |l, v| l.kit.c.$role.border = v),
            Token::color(concat!("color.", stringify!($role), ".icon"), |l| l.kit.c.$role.icon, |l, v| l.kit.c.$role.icon = v),
        )*]
    }};
}

const STATES: &[Token] = states!(success, warning, danger, info);

/// `<prefix>.<field>` lengths over the struct at `look.<a>` or `look.<a>.<b>`, whose
/// fields the pattern names exhaustively.
macro_rules! lengths {
    ($ty:ident, $prefix:literal, $a:ident . $b:ident => $($field:ident),* $(,)?) => {{
        const _: fn($ty) = |$ty { $($field: _,)* }| {};
        &[$(Token::length(
            concat!($prefix, ".", stringify!($field)),
            |l| l.$a.$b.$field,
            |l, v| l.$a.$b.$field = v,
        ),)*]
    }};
    ($ty:ident, $prefix:literal, $a:ident => $($field:ident),* $(,)?) => {{
        const _: fn($ty) = |$ty { $($field: _,)* }| {};
        &[$(Token::length(
            concat!($prefix, ".", stringify!($field)),
            |l| l.$a.$field,
            |l, v| l.$a.$field = v,
        ),)*]
    }};
}

const RADII: &[Token] = lengths!(Radius, "radius", kit.radius => xs, sm, md, lg, xl);

/// `type.<style>.{size,line_height,weight,tracking}` for each style of `kit.ty`, then of
/// `look.ty`. The patterns name every field of both, so this one list is the whole ramp.
macro_rules! type_styles {
    (kit.ty: [$($kit:ident),* $(,)?], ty: [$($ramp:ident),* $(,)?] $(,)?) => {{
        const _: fn(Type) = |Type { $($kit: _,)* }| {};
        const _: fn(Ramp) = |Ramp { $($ramp: _,)* }| {};
        &[
            $(type_styles!(@style kit.ty, $kit),)*
            $(type_styles!(@style ty, $ramp),)*
        ]
    }};
    (@style $($path:ident).+, $style:ident) => {
        [
            Token::length(concat!("type.", stringify!($style), ".size"), |l| l.$($path).+.$style.size, |l, v| l.$($path).+.$style.size = v),
            Token::length(concat!("type.", stringify!($style), ".line_height"), |l| l.$($path).+.$style.line_height, |l, v| l.$($path).+.$style.line_height = v),
            Token::number(concat!("type.", stringify!($style), ".weight"), 1.0, 1000.0, |l| l.$($path).+.$style.weight, |l, v| l.$($path).+.$style.weight = v),
            Token::number(concat!("type.", stringify!($style), ".tracking"), -10.0, 10.0, |l| l.$($path).+.$style.tracking, |l, v| l.$($path).+.$style.tracking = v),
        ]
    };
}

const TYPE_STYLES: &[[Token; 4]] = type_styles!(
    kit.ty: [
        ui,
        ui_medium,
        ui_compact,
        small,
        small_medium,
        body,
        body_medium,
        text_small,
        text_small_medium,
        text_xsmall,
        text_xsmall_medium,
        mono,
    ],
    ty: [title, heading],
);
const TYPE: &[Token] = TYPE_STYLES.as_flattened();

const SPACE: &[Token] = lengths!(Space, "space", space => xxs, xs, sm, md, lg, xl, xxl);
const FRAME: &[Token] = lengths!(Frame, "frame", frame => inset, bar, control);
const CONTENT: &[Token] = lengths!(Content, "content", content => pad);
const ROW: &[Token] = lengths!(RowHeights, "row", row => list, columns, sidebar, palette);
const ICON_SIZE: &[Token] = lengths!(IconSizes, "icon_size", icon_size => sm, md, lg);

const GLOW: &[Token] = &[
    Token::flag("glow.on", |l| l.glow.on, |l, v| l.glow.on = v),
    Token::color("glow.color", |l| l.glow.color, |l, v| l.glow.color = v),
    Token::length("glow.radius", |l| l.glow.radius, |l, v| l.glow.radius = v),
    Token::number(
        "glow.opacity",
        0.0,
        1.0,
        |l| l.glow.opacity,
        |l, v| l.glow.opacity = v,
    ),
];

/// `fill.<field>` for each `look.fill` colour. The pattern names every `Fills` field, so one
/// added there fails to build until it is registered here.
const FILL: &[Token] = {
    const _: fn(Fills) = |Fills {
                              muted: _,
                              highlight: _,
                          }| {};
    &[
        Token::color("fill.muted", |l| l.fill.muted, |l, v| l.fill.muted = v),
        Token::color(
            "fill.highlight",
            |l| l.fill.highlight,
            |l, v| l.fill.highlight = v,
        ),
    ]
};

/// `syntax.<kind>` for each kind rune-kit colours, then for each of `look.syntax_extra`.
macro_rules! syntax {
    ($($kind:ident => $name:literal),* $(,)?; extra: $($extra:ident),* $(,)?) => {{
        // `Syntax` is non-exhaustive, so a kind a Rune bump adds can't fail this build:
        // check this list against `rune_kit::syntax::Syntax` on every bump.
        const _: fn(SyntaxExtra) = |SyntaxExtra { $($extra: _,)* }| {};
        &[$(Token::color(
            concat!("syntax.", $name),
            |l| l.syntax.color(Syntax::$kind),
            |l, v| l.syntax = l.syntax.with(Syntax::$kind, v),
        ),)* $(Token::color(
            concat!("syntax.", stringify!($extra)),
            |l| l.syntax_extra.$extra,
            |l, v| l.syntax_extra.$extra = v,
        ),)*]
    }};
}

const SYNTAX: &[Token] = syntax!(
    Keyword => "keyword",
    String => "string",
    Number => "number",
    Comment => "comment",
    Type => "type",
    Function => "function",
    Property => "property",
    Punctuation => "punctuation",
    Inserted => "inserted",
    Deleted => "deleted",
    Parameter => "parameter",
    Heading => "heading",
    Link => "link",
    Operator => "operator",
    Regex => "regex",
    Attribute => "attribute",
    Variable => "variable";
    extra: preprocessor,
);
