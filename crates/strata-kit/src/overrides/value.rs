//! A row's value, read for its token's kind.
//!
//! A colour is a literal, or (ruling Q2) one taken from another colour token, so it follows
//! the theme: `"@color.text_base"`, or `"alpha(@color.text_base, 0.06)"` for it at a share of
//! its opacity. `alpha()` takes a literal too, `"alpha(#ff9e64, 0.25)"`, and paints as the
//! kit's own translucent tokens do ([`Kit::alpha`](rune_kit::Kit::alpha), with the colour's
//! alpha times the factor).

use rune::Color;
use toml::de::DeValue;

use super::Problem;
use crate::look::Look;
use crate::theme::Rgba8;
use crate::tokens::{self, Kind, Token, Value};

/// What a row sets its token to.
#[derive(Clone, Copy, Debug)]
pub(super) enum Setting {
    /// A value as written.
    Value(Value),
    /// A colour taken from another token, or a literal at a share of its opacity: resolved
    /// when the look is built.
    Derived { from: Origin, alpha: Option<f32> },
}

/// Where a derived colour starts.
#[derive(Clone, Copy, Debug)]
pub(super) enum Origin {
    Color(Color),
    Token(&'static Token),
}

impl PartialEq for Setting {
    fn eq(&self, other: &Setting) -> bool {
        match (self, other) {
            (Setting::Value(a), Setting::Value(b)) => a == b,
            (Setting::Derived { from: a, alpha: x }, Setting::Derived { from: b, alpha: y }) => {
                x == y
                    && match (a, b) {
                        (Origin::Color(a), Origin::Color(b)) => a == b,
                        (Origin::Token(a), Origin::Token(b)) => a.name() == b.name(),
                        _ => false,
                    }
            }
            _ => false,
        }
    }
}

impl Setting {
    /// The colour a derived setting stands for in `look`.
    pub(super) fn resolve(from: Origin, alpha: Option<f32>, look: &Look) -> Color {
        let c = match from {
            Origin::Color(c) => c,
            Origin::Token(t) => match t.get(look) {
                Value::Color(c) => c,
                other => unreachable!("parse takes colour tokens only, not {other:?}"),
            },
        };
        match alpha {
            Some(x) => look.kit.alpha(c, c.a * x),
            None => c,
        }
    }
}

/// `value` as `token` takes it, or why not. Whatever this returns, `token.set` accepts.
pub(super) fn read(token: &'static Token, value: &DeValue<'_>) -> Result<Setting, Problem> {
    let name = token.name();
    let wrong = || Problem::Kind {
        name,
        expected: token.kind(),
        found: value.type_str(),
    };
    match token.kind() {
        Kind::Color => match value {
            DeValue::String(s) => color(name, s),
            _ => Err(wrong()),
        },
        Kind::Length | Kind::Number => {
            let x = number(value).ok_or_else(wrong)?;
            let (min, max) = token.range().unwrap_or((f32::NEG_INFINITY, f32::INFINITY));
            if !(x.is_finite() && min <= x && x <= max) {
                return Err(Problem::Range { name, min, max });
            }
            Ok(Setting::Value(if token.kind() == Kind::Length {
                Value::Length(x)
            } else {
                Value::Number(x)
            }))
        }
        Kind::Flag => match value {
            DeValue::Boolean(b) => Ok(Setting::Value(Value::Flag(*b))),
            _ => Err(wrong()),
        },
    }
}

/// A colour string: a literal, `@<colour token>`, or `alpha(<either>, <factor>)`.
fn color(name: &'static str, s: &str) -> Result<Setting, Problem> {
    let bad = || Problem::BadColor {
        name,
        value: s.to_owned(),
    };
    let t = s.trim();
    let alpha_args = t
        .get(..6)
        .filter(|f| f.eq_ignore_ascii_case("alpha("))
        .and_then(|_| t[6..].strip_suffix(')'));
    if let Some(args) = alpha_args {
        let (inner, factor) = args.rsplit_once(',').ok_or_else(bad)?;
        // `+ 0.0` turns `-0` into `0`, so equal files compare equal.
        let factor = factor.trim().parse::<f32>().map_err(|_| bad())? + 0.0;
        if !(0.0..=1.0).contains(&factor) {
            return Err(bad());
        }
        let from = source(name, inner.trim()).ok_or_else(bad)??;
        return Ok(Setting::Derived {
            from,
            alpha: Some(factor),
        });
    }
    match source(name, t) {
        Some(Ok(Origin::Color(c))) => Ok(Setting::Value(Value::Color(c))),
        Some(Ok(from)) => Ok(Setting::Derived { from, alpha: None }),
        Some(Err(problem)) => Err(problem),
        None => Err(bad()),
    }
}

/// A literal colour or a reference: `None` when it is neither, an error when it names no
/// colour token.
fn source(name: &'static str, s: &str) -> Option<Result<Origin, Problem>> {
    if let Some(target) = s.strip_prefix('@') {
        return Some(match tokens::find(target) {
            Some(t) if t.kind() == Kind::Color => Ok(Origin::Token(t)),
            _ => Err(Problem::Reference {
                name,
                target: target.to_owned(),
            }),
        });
    }
    Rgba8::parse(s).map(|c| Ok(Origin::Color(c.color())))
}

/// A TOML integer or float as an `f32`: infinite when an integer doesn't fit, so the range
/// check rejects it. `None` for anything else.
fn number(value: &DeValue<'_>) -> Option<f32> {
    match value {
        DeValue::Integer(i) => {
            Some(i64::from_str_radix(i.as_str(), i.radix()).map_or(f32::INFINITY, |v| v as f32))
        }
        DeValue::Float(x) => Some(x.as_str().parse::<f64>().map_or(f32::NAN, |v| v as f32)),
        _ => None,
    }
}
