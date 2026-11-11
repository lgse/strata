//! Theme colours as Strata stores them: 8-bit sRGB with alpha, parsed from the forms GTK
//! accepts, and the blending and contrast arithmetic Strata's theme code does on them.
//!
//! Strata blends and measures contrast on 8-bit sRGB channels (`src/ui/theme/color.rs` and
//! `omarchy.rs` at lgse/strata 35ee677ab), rounding half away from zero. The same arithmetic
//! here gives the same colours, so a bundled theme's lifted dim text and an Omarchy Contrast
//! palette match Strata's to the byte. Rune's own `Color::mix` works in linear light and
//! would not.

use std::fmt;

use rune::Color;

#[cfg(test)]
mod tests;

/// A colour as a theme stores it: sRGB channels and alpha, 0–255 each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const WHITE: Rgba8 = Rgba8::rgb(255, 255, 255);
    pub const BLACK: Rgba8 = Rgba8::rgb(0, 0, 0);

    /// An opaque colour.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Rgba8 {
        Rgba8 { r, g, b, a: 255 }
    }

    /// Parses a colour in a form GTK's `gdk::RGBA::parse` accepts, as Strata's theme files
    /// may use:
    ///
    /// - hex: `#rgb`, `#rrggbb`, `#rrrgggbbb` and `#rrrrggggbbbb`, and with alpha `#rgba`,
    ///   `#rrggbbaa` and `#rrrrggggbbbbaaaa`;
    /// - `rgb()` and `rgba()`, with channels from 0 to 255 or in percent;
    /// - `hsl()` and `hsla()`;
    /// - CSS Color 4's named colours, and `transparent`.
    ///
    /// Commas or spaces separate the arguments, and a `/` may come before the alpha. Case
    /// doesn't matter. X11 colour names that CSS lacks (`lightgoldenrod`) don't parse:
    /// GTK reads those through Pango's table, which this doesn't carry.
    pub fn parse(value: &str) -> Option<Rgba8> {
        let v = value.trim().to_ascii_lowercase();
        if let Some(hex) = v.strip_prefix('#') {
            return parse_hex(hex);
        }
        if v == "transparent" {
            return Some(Rgba8 {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            });
        }
        if let Some((func, rest)) = v.split_once('(') {
            let args = rest.strip_suffix(')')?;
            return match func.trim() {
                "rgb" | "rgba" => parse_rgb(args),
                "hsl" | "hsla" => parse_hsl(args),
                _ => None,
            };
        }
        NAMED
            .binary_search_by(|(name, _)| name.cmp(&v.as_str()))
            .ok()
            .map(|i| {
                let [r, g, b] = NAMED[i].1;
                Rgba8::rgb(r, g, b)
            })
    }

    /// The canonical form Strata keeps in memory: `#rrggbb`, or `#rrggbbaa` when translucent.
    pub fn hex(self) -> String {
        let Rgba8 { r, g, b, a } = self;
        if a == u8::MAX {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        }
    }

    /// The colour as Rune draws it.
    pub fn color(self) -> Color {
        let encoded = |v: u8| f32::from(v) / 255.0;
        let decode = |c: f32| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        Color {
            r: decode(encoded(self.r)),
            g: decode(encoded(self.g)),
            b: decode(encoded(self.b)),
            a: encoded(self.a),
        }
    }

    /// The same colour, opaque: what Strata's blends and contrast checks read.
    pub fn opaque(self) -> Rgba8 {
        Rgba8 { a: 255, ..self }
    }
}

impl fmt::Display for Rgba8 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

/// GTK's hex forms. Four, eight and sixteen digits carry alpha; otherwise three channels of
/// one to four digits each. A channel of `n` digits widens to 16 bits by repeating its bits,
/// as Pango does, then rounds to 8.
fn parse_hex(hex: &str) -> Option<Rgba8> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let n = hex.len();
    let (channels, digits) = match n {
        4 | 8 | 16 => (4, n / 4),
        3 | 6 | 9 | 12 => (3, n / 3),
        _ => return None,
    };
    let mut out = [255u8; 4];
    for (i, slot) in out.iter_mut().enumerate().take(channels) {
        let part = &hex[i * digits..(i + 1) * digits];
        let mut v = u32::from_str_radix(part, 16).ok()?;
        let mut bits = digits as u32 * 4;
        v <<= 16 - bits;
        while bits < 16 {
            v |= v >> bits;
            bits *= 2;
        }
        *slot = (f64::from(v) / 65535.0 * 255.0).round() as u8;
    }
    let [r, g, b, a] = out;
    Some(Rgba8 { r, g, b, a })
}

/// The arguments of a colour function: commas or spaces between them, an optional `/` before
/// the alpha.
fn args(s: &str) -> Vec<&str> {
    s.split([',', ' ', '/'])
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .collect()
}

/// A number, or a percentage of `full`.
fn number(s: &str, full: f64) -> Option<f64> {
    let v = match s.strip_suffix('%') {
        Some(p) => p.parse::<f64>().ok()? / 100.0 * full,
        None => s.parse::<f64>().ok()?,
    };
    v.is_finite().then_some(v)
}

fn alpha(args: &[&str]) -> Option<u8> {
    match args {
        [] => Some(255),
        [a] => Some(unit(number(a, 1.0)?)),
        _ => None,
    }
}

/// 0–1 to 0–255.
fn unit(v: f64) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn parse_rgb(s: &str) -> Option<Rgba8> {
    let a = args(s);
    if a.len() < 3 {
        return None;
    }
    let ch = |s: &str| number(s, 255.0).map(|v| v.clamp(0.0, 255.0).round() as u8);
    Some(Rgba8 {
        r: ch(a[0])?,
        g: ch(a[1])?,
        b: ch(a[2])?,
        a: alpha(&a[3..])?,
    })
}

fn parse_hsl(s: &str) -> Option<Rgba8> {
    let a = args(s);
    if a.len() < 3 {
        return None;
    }
    let h = a[0]
        .strip_suffix("deg")
        .unwrap_or(a[0])
        .parse::<f64>()
        .ok()?;
    let sat = number(a[1].strip_suffix('%')?, 1.0)? / 100.0;
    let light = number(a[2].strip_suffix('%')?, 1.0)? / 100.0;
    if !h.is_finite() {
        return None;
    }
    let (sat, light) = (sat.clamp(0.0, 1.0), light.clamp(0.0, 1.0));
    let h = h.rem_euclid(360.0) / 30.0;
    let k = |n: f64| (n + h).rem_euclid(12.0);
    let reach = sat * light.min(1.0 - light);
    let f = |n: f64| unit(light - reach * (k(n) - 3.0).min(9.0 - k(n)).clamp(-1.0, 1.0));
    Some(Rgba8 {
        r: f(0.0),
        g: f(8.0),
        b: f(4.0),
        a: alpha(&a[3..])?,
    })
}

/// `t` of the way from `a` to `b`, per 8-bit channel, rounded half away from zero: Strata's
/// `blend`. The result is opaque.
pub fn blend(a: Rgba8, b: Rgba8, t: f64) -> Rgba8 {
    let ch = |x: u8, y: u8| {
        let (x, y) = (f64::from(x), f64::from(y));
        (x + (y - x) * t).round() as u8
    };
    Rgba8::rgb(ch(a.r, b.r), ch(a.g, b.g), ch(a.b, b.b))
}

/// WCAG relative luminance of the colour's sRGB channels, alpha ignored.
pub fn luminance(c: Rgba8) -> f64 {
    let lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    lin(c.r) * 0.2126 + lin(c.g) * 0.7152 + lin(c.b) * 0.0722
}

/// WCAG contrast ratio, 1 to 21.
pub fn contrast(a: Rgba8, b: Rgba8) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The first of `color` blended toward `pole` in 1/255 steps that reaches `ratio` against
/// every one of `backgrounds`: Strata's `readable`. Strata's version panics when no step does;
/// this returns the pole.
pub fn readable(color: Rgba8, backgrounds: &[Rgba8], pole: Rgba8, ratio: f64) -> Rgba8 {
    toward(color, pole, |c| {
        backgrounds.iter().all(|&bg| contrast(c, bg) >= ratio)
    })
}

/// The first of `color` blended toward `pole` in 1/255 steps for which `done` holds, or the
/// pole.
pub fn toward(color: Rgba8, pole: Rgba8, done: impl Fn(Rgba8) -> bool) -> Rgba8 {
    (0..=255)
        .map(|step| blend(color, pole, f64::from(step) / 255.0))
        .find(|&c| done(c))
        .unwrap_or(pole)
}

/// CSS Color 4's named colours, sorted by name for the binary search.
const NAMED: &[(&str, [u8; 3])] = &[
    ("aliceblue", [240, 248, 255]),
    ("antiquewhite", [250, 235, 215]),
    ("aqua", [0, 255, 255]),
    ("aquamarine", [127, 255, 212]),
    ("azure", [240, 255, 255]),
    ("beige", [245, 245, 220]),
    ("bisque", [255, 228, 196]),
    ("black", [0, 0, 0]),
    ("blanchedalmond", [255, 235, 205]),
    ("blue", [0, 0, 255]),
    ("blueviolet", [138, 43, 226]),
    ("brown", [165, 42, 42]),
    ("burlywood", [222, 184, 135]),
    ("cadetblue", [95, 158, 160]),
    ("chartreuse", [127, 255, 0]),
    ("chocolate", [210, 105, 30]),
    ("coral", [255, 127, 80]),
    ("cornflowerblue", [100, 149, 237]),
    ("cornsilk", [255, 248, 220]),
    ("crimson", [220, 20, 60]),
    ("cyan", [0, 255, 255]),
    ("darkblue", [0, 0, 139]),
    ("darkcyan", [0, 139, 139]),
    ("darkgoldenrod", [184, 134, 11]),
    ("darkgray", [169, 169, 169]),
    ("darkgreen", [0, 100, 0]),
    ("darkgrey", [169, 169, 169]),
    ("darkkhaki", [189, 183, 107]),
    ("darkmagenta", [139, 0, 139]),
    ("darkolivegreen", [85, 107, 47]),
    ("darkorange", [255, 140, 0]),
    ("darkorchid", [153, 50, 204]),
    ("darkred", [139, 0, 0]),
    ("darksalmon", [233, 150, 122]),
    ("darkseagreen", [143, 188, 143]),
    ("darkslateblue", [72, 61, 139]),
    ("darkslategray", [47, 79, 79]),
    ("darkslategrey", [47, 79, 79]),
    ("darkturquoise", [0, 206, 209]),
    ("darkviolet", [148, 0, 211]),
    ("deeppink", [255, 20, 147]),
    ("deepskyblue", [0, 191, 255]),
    ("dimgray", [105, 105, 105]),
    ("dimgrey", [105, 105, 105]),
    ("dodgerblue", [30, 144, 255]),
    ("firebrick", [178, 34, 34]),
    ("floralwhite", [255, 250, 240]),
    ("forestgreen", [34, 139, 34]),
    ("fuchsia", [255, 0, 255]),
    ("gainsboro", [220, 220, 220]),
    ("ghostwhite", [248, 248, 255]),
    ("gold", [255, 215, 0]),
    ("goldenrod", [218, 165, 32]),
    ("gray", [128, 128, 128]),
    ("green", [0, 128, 0]),
    ("greenyellow", [173, 255, 47]),
    ("grey", [128, 128, 128]),
    ("honeydew", [240, 255, 240]),
    ("hotpink", [255, 105, 180]),
    ("indianred", [205, 92, 92]),
    ("indigo", [75, 0, 130]),
    ("ivory", [255, 255, 240]),
    ("khaki", [240, 230, 140]),
    ("lavender", [230, 230, 250]),
    ("lavenderblush", [255, 240, 245]),
    ("lawngreen", [124, 252, 0]),
    ("lemonchiffon", [255, 250, 205]),
    ("lightblue", [173, 216, 230]),
    ("lightcoral", [240, 128, 128]),
    ("lightcyan", [224, 255, 255]),
    ("lightgoldenrodyellow", [250, 250, 210]),
    ("lightgray", [211, 211, 211]),
    ("lightgreen", [144, 238, 144]),
    ("lightgrey", [211, 211, 211]),
    ("lightpink", [255, 182, 193]),
    ("lightsalmon", [255, 160, 122]),
    ("lightseagreen", [32, 178, 170]),
    ("lightskyblue", [135, 206, 250]),
    ("lightslategray", [119, 136, 153]),
    ("lightslategrey", [119, 136, 153]),
    ("lightsteelblue", [176, 196, 222]),
    ("lightyellow", [255, 255, 224]),
    ("lime", [0, 255, 0]),
    ("limegreen", [50, 205, 50]),
    ("linen", [250, 240, 230]),
    ("magenta", [255, 0, 255]),
    ("maroon", [128, 0, 0]),
    ("mediumaquamarine", [102, 205, 170]),
    ("mediumblue", [0, 0, 205]),
    ("mediumorchid", [186, 85, 211]),
    ("mediumpurple", [147, 112, 219]),
    ("mediumseagreen", [60, 179, 113]),
    ("mediumslateblue", [123, 104, 238]),
    ("mediumspringgreen", [0, 250, 154]),
    ("mediumturquoise", [72, 209, 204]),
    ("mediumvioletred", [199, 21, 133]),
    ("midnightblue", [25, 25, 112]),
    ("mintcream", [245, 255, 250]),
    ("mistyrose", [255, 228, 225]),
    ("moccasin", [255, 228, 181]),
    ("navajowhite", [255, 222, 173]),
    ("navy", [0, 0, 128]),
    ("oldlace", [253, 245, 230]),
    ("olive", [128, 128, 0]),
    ("olivedrab", [107, 142, 35]),
    ("orange", [255, 165, 0]),
    ("orangered", [255, 69, 0]),
    ("orchid", [218, 112, 214]),
    ("palegoldenrod", [238, 232, 170]),
    ("palegreen", [152, 251, 152]),
    ("paleturquoise", [175, 238, 238]),
    ("palevioletred", [219, 112, 147]),
    ("papayawhip", [255, 239, 213]),
    ("peachpuff", [255, 218, 185]),
    ("peru", [205, 133, 63]),
    ("pink", [255, 192, 203]),
    ("plum", [221, 160, 221]),
    ("powderblue", [176, 224, 230]),
    ("purple", [128, 0, 128]),
    ("rebeccapurple", [102, 51, 153]),
    ("red", [255, 0, 0]),
    ("rosybrown", [188, 143, 143]),
    ("royalblue", [65, 105, 225]),
    ("saddlebrown", [139, 69, 19]),
    ("salmon", [250, 128, 114]),
    ("sandybrown", [244, 164, 96]),
    ("seagreen", [46, 139, 87]),
    ("seashell", [255, 245, 238]),
    ("sienna", [160, 82, 45]),
    ("silver", [192, 192, 192]),
    ("skyblue", [135, 206, 235]),
    ("slateblue", [106, 90, 205]),
    ("slategray", [112, 128, 144]),
    ("slategrey", [112, 128, 144]),
    ("snow", [255, 250, 250]),
    ("springgreen", [0, 255, 127]),
    ("steelblue", [70, 130, 180]),
    ("tan", [210, 180, 140]),
    ("teal", [0, 128, 128]),
    ("thistle", [216, 191, 216]),
    ("tomato", [255, 99, 71]),
    ("turquoise", [64, 224, 208]),
    ("violet", [238, 130, 238]),
    ("wheat", [245, 222, 179]),
    ("white", [255, 255, 255]),
    ("whitesmoke", [245, 245, 245]),
    ("yellow", [255, 255, 0]),
    ("yellowgreen", [154, 205, 50]),
];
