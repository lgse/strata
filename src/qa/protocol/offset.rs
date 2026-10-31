// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct At {
    pub(crate) x: Offset,
    pub(crate) y: Offset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Offset {
    Pixels(i32),
    Percent(u8),
    Fraction(u16, u16),
    FromStart(i32),
    FractionPlus(u16, u16, i32),
    FromEnd(i32),
    EndLessFraction(u16, u16),
}

impl Offset {
    pub(crate) fn parse(value: &Value) -> Result<Self, String> {
        if let Some(pixels) = value.as_i64() {
            return i32::try_from(pixels)
                .map(Self::Pixels)
                .map_err(|_| format!("{pixels} is out of range"));
        }
        let text = value.as_str().unwrap_or_default();
        let percent = text
            .strip_suffix('%')
            .and_then(|number| number.parse::<u8>().ok())
            .filter(|percent| *percent <= 100)
            .map(Self::Percent);
        let fraction = |text: &str| {
            text.split_once('/')
                .and_then(|(numerator, denominator)| {
                    Some((
                        numerator.parse::<u16>().ok()?,
                        denominator.parse::<u16>().ok()?,
                    ))
                })
                .filter(|(numerator, denominator)| *denominator > 0 && numerator <= denominator)
        };
        let share =
            fraction(text).map(|(numerator, denominator)| Self::Fraction(numerator, denominator));
        let share_plus = text
            .rfind(['+', '-'])
            .filter(|position| *position > 0)
            .and_then(|position| {
                let (numerator, denominator) = fraction(&text[..position])?;
                let pixels = text[position..].parse::<i32>().ok()?;
                Some(Self::FractionPlus(numerator, denominator, pixels))
            });
        let from_start = text
            .strip_prefix("start")
            .filter(|offset| offset.starts_with(['+', '-']))
            .and_then(|offset| offset.parse::<i32>().ok())
            .map(Self::FromStart);
        let from_end = text.strip_prefix("end").and_then(|offset| {
            if let Some((numerator, denominator)) = offset.strip_prefix('-').and_then(fraction) {
                return Some(Self::EndLessFraction(numerator, denominator));
            }
            offset
                .starts_with(['+', '-'])
                .then(|| offset.parse::<i32>().ok())
                .flatten()
                .map(Self::FromEnd)
        });
        percent
            .or(share)
            .or(share_plus)
            .or(from_start)
            .or(from_end)
            .ok_or_else(|| {
                format!(
                    "an offset is whole pixels, a share like \"80%\", \"2/3\", or \"2/3+3\", \
                     \"start-6\", \"end+60\", or \"end-1/6\", not {value}"
                )
            })
    }

    pub(crate) fn resolve(self, start: f64, extent: f64) -> f64 {
        match self {
            Self::Pixels(pixels) if pixels < 0 => start + extent + f64::from(pixels),
            Self::Pixels(pixels) | Self::FromStart(pixels) => start + f64::from(pixels),
            Self::Percent(percent) => start + (extent * f64::from(percent) / 100.0).floor(),
            Self::Fraction(numerator, denominator) => {
                start + (extent * f64::from(numerator) / f64::from(denominator)).floor()
            }
            Self::FractionPlus(numerator, denominator, pixels) => {
                Self::Fraction(numerator, denominator).resolve(start, extent) + f64::from(pixels)
            }
            Self::FromEnd(pixels) => start + extent + f64::from(pixels),
            Self::EndLessFraction(numerator, denominator) => {
                start + extent - (extent * f64::from(numerator) / f64::from(denominator)).floor()
            }
        }
    }
}

impl Offset {
    pub(super) fn to_json(self) -> Value {
        match self {
            Self::Pixels(pixels) => json!(pixels),
            Self::Percent(percent) => json!(format!("{percent}%")),
            Self::Fraction(numerator, denominator) => json!(format!("{numerator}/{denominator}")),
            Self::FromStart(pixels) => json!(format!("start{pixels:+}")),
            Self::FractionPlus(numerator, denominator, pixels) => {
                json!(format!("{numerator}/{denominator}{pixels:+}"))
            }
            Self::FromEnd(pixels) => json!(format!("end{pixels:+}")),
            Self::EndLessFraction(numerator, denominator) => {
                json!(format!("end-{numerator}/{denominator}"))
            }
        }
    }
}

impl Serialize for At {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        json!([self.x.to_json(), self.y.to_json()]).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for At {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let parsed = match value.as_array().map(Vec::as_slice) {
            Some([x, y]) => Offset::parse(x).and_then(|x| {
                Ok(Self {
                    x,
                    y: Offset::parse(y)?,
                })
            }),
            _ => Err(format!("at is [x, y], not {value}")),
        };
        parsed.map_err(serde::de::Error::custom)
    }
}
