//! The picker's colour value: 8-bit sRGB with straight alpha ([`Srgba`]), its hex text, its
//! conversions to and from `rune::Color` (linear sRGB, straight alpha), and HSV ([`Hsv`]).

use std::fmt;
use std::str::FromStr;

use rune::Color;
use rune::color::{srgb_decode, srgb_encode};

/// A colour as Strata saves it: sRGB-encoded channels and straight alpha, 8 bits each.
///
/// Equality is exact, so a colour read back from its hex text equals the one written. To
/// `rune::Color` each channel is decoded to linear light; from it each is encoded and rounded
/// to the nearest 8-bit step, as `Color::hex` rounds. So `Srgba → Color → Srgba` gives the
/// same colour back, while `Color → Srgba` rounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Srgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// Text that isn't `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseHexError;

impl fmt::Display for ParseHexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a hex colour (#RGB, #RGBA, #RRGGBB or #RRGGBBAA)")
    }
}

impl std::error::Error for ParseHexError {}

impl Srgba {
    pub const BLACK: Srgba = Srgba::rgb(0, 0, 0);
    pub const WHITE: Srgba = Srgba::rgb(255, 255, 255);

    /// An opaque colour.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Srgba {
        Srgba { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Srgba {
        Srgba { r, g, b, a }
    }

    /// The same colour with full alpha.
    pub const fn opaque(self) -> Srgba {
        Srgba { a: 255, ..self }
    }

    pub const fn is_opaque(self) -> bool {
        self.a == 255
    }

    /// Hex text: `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA`, in either case, with or without the
    /// `#`, and with spaces around it. A short form doubles each digit (`#abc` is `#aabbcc`);
    /// a form without alpha is opaque.
    pub fn parse_hex(text: &str) -> Option<Srgba> {
        let t = text.trim();
        let digits = t.strip_prefix('#').unwrap_or(t);
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let nibble = |i: usize| u8::from_str_radix(&digits[i..=i], 16).ok();
        let byte = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
        let short = |i: usize| nibble(i).map(|n| n * 17);
        match digits.len() {
            3 => Some(Srgba::rgb(short(0)?, short(1)?, short(2)?)),
            4 => Some(Srgba::rgba(short(0)?, short(1)?, short(2)?, short(3)?)),
            6 => Some(Srgba::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Some(Srgba::rgba(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
            _ => None,
        }
    }

    /// Lowercase `#rrggbb` when opaque, else `#rrggbbaa`: how Strata saves a colour, and how a
    /// translucent theme colour reads back.
    pub fn hex(self) -> String {
        if self.is_opaque() {
            self.hex_rgb()
        } else {
            format!("{}{:02x}", self.hex_rgb(), self.a)
        }
    }

    /// Lowercase `#rrggbb`, leaving alpha out.
    pub fn hex_rgb(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Linear `rune::Color` with straight alpha.
    pub fn to_color(self) -> Color {
        let lin = |v: u8| srgb_decode(f32::from(v) / 255.0);
        Color {
            r: lin(self.r),
            g: lin(self.g),
            b: lin(self.b),
            a: f32::from(self.a) / 255.0,
        }
    }

    /// The nearest colour to `c` (linear, straight alpha), clamped to `0..=1`; not a number is
    /// 0.
    pub fn from_color(c: Color) -> Srgba {
        let enc = |v: f32| unit_byte(srgb_encode(unit(v)));
        Srgba {
            r: enc(c.r),
            g: enc(c.g),
            b: enc(c.b),
            a: unit_byte(unit(c.a)),
        }
    }

    /// The channels as sRGB-encoded fractions, `0..=1`.
    fn unit_rgb(self) -> [f64; 3] {
        [self.r, self.g, self.b].map(|v| f64::from(v) / 255.0)
    }
}

/// `v` in `0..=1`; not a number is 0.
fn unit(v: f32) -> f32 {
    if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }
}

/// A fraction `0..=1` as the nearest byte.
fn unit_byte(v: f32) -> u8 {
    (unit(v) * 255.0).round() as u8
}

impl From<Srgba> for Color {
    fn from(c: Srgba) -> Color {
        c.to_color()
    }
}

impl From<Color> for Srgba {
    fn from(c: Color) -> Srgba {
        Srgba::from_color(c)
    }
}

impl fmt::Display for Srgba {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

impl FromStr for Srgba {
    type Err = ParseHexError;

    fn from_str(s: &str) -> Result<Srgba, ParseHexError> {
        Srgba::parse_hex(s).ok_or(ParseHexError)
    }
}

/// Hue, saturation, value and alpha, over the sRGB-encoded channels as GTK and CSS compute
/// them: `h` in degrees `0..=360` (360 is red again), `s`, `v` and `a` in `0..=1`.
///
/// A grey has no hue and black no saturation: [`Hsv::of_keeping`] keeps the ones it had, so a
/// drag through the plane's bottom edge or its left edge doesn't lose them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsv {
    pub h: f32,
    pub s: f32,
    pub v: f32,
    pub a: f32,
}

impl Hsv {
    /// `c` as HSV, with hue 0 for a grey.
    pub fn of(c: Srgba) -> Hsv {
        Hsv::of_keeping(
            c,
            Hsv {
                h: 0.0,
                s: 0.0,
                v: 0.0,
                a: 1.0,
            },
        )
    }

    /// `c` as HSV, keeping `previous`'s hue where `c` has none (a grey or black) and its
    /// saturation where `c` has none to speak of (black).
    pub fn of_keeping(c: Srgba, previous: Hsv) -> Hsv {
        let [r, g, b] = c.unit_rgb();
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        let a = f32::from(c.a) / 255.0;
        if max == 0.0 {
            return Hsv {
                h: previous.h,
                s: previous.s,
                v: 0.0,
                a,
            };
        }
        let s = (d / max) as f32;
        let v = max as f32;
        if d == 0.0 {
            return Hsv {
                h: previous.h,
                s,
                v,
                a,
            };
        }
        let sector = if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        Hsv {
            h: (sector * 60.0) as f32,
            s,
            v,
            a,
        }
    }

    /// The colour, each channel rounded to the nearest 8-bit step.
    pub fn to_srgba(self) -> Srgba {
        let [r, g, b] = self.unit_rgb();
        let byte = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        Srgba::rgba(byte(r), byte(g), byte(b), unit_byte(self.a))
    }

    /// The fully saturated, full-value colour of this hue: the plane's corner.
    pub fn pure_hue(self) -> Srgba {
        Hsv {
            s: 1.0,
            v: 1.0,
            a: 1.0,
            ..self
        }
        .to_srgba()
    }

    /// The sRGB-encoded channels, `0..=1`.
    fn unit_rgb(self) -> [f64; 3] {
        let h = f64::from(self.h).rem_euclid(360.0) / 60.0;
        let s = f64::from(unit(self.s));
        let v = f64::from(unit(self.v));
        let sector = h.floor();
        let f = h - sector;
        let p = v * (1.0 - s);
        let q = v * (1.0 - s * f);
        let t = v * (1.0 - s * (1.0 - f));
        match sector as u8 {
            0 => [v, t, p],
            1 => [q, v, p],
            2 => [p, v, t],
            3 => [p, q, v],
            4 => [t, p, v],
            _ => [v, p, q],
        }
    }
}
