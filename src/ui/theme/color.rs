// SPDX-License-Identifier: MIT

//! Parsing theme colors and writing them in the forms GTK CSS, GtkSourceView
//! schemes and SVG strokes accept.

use gtk::gdk;

/// Parses colours GTK accepts (`#rgb`, `#rrggbb`, `rgb(...)`, names) into 8-bit
/// channels. Strata emits these channels as `#rrggbb` in GtkSourceView schemes.
pub(crate) fn parse_rgb_channels(value: &str) -> Option<[u8; 3]> {
    parse_rgba_channels(value).map(|[red, green, blue, _]| [red, green, blue])
}

fn parse_rgba_channels(value: &str) -> Option<[u8; 4]> {
    let color = gdk::RGBA::parse(value).ok()?;
    let channel = |component: f32| (f64::from(component).clamp(0.0, 1.0) * 255.0).round() as u8;
    Some([
        channel(color.red()),
        channel(color.green()),
        channel(color.blue()),
        channel(color.alpha()),
    ])
}

/// Rewrites a color GTK can parse as `#rrggbb`, or `#rrggbbaa` when translucent.
/// `gdk::RGBA::parse` also accepts 9-, 12- and 16-digit hex, which GTK CSS rejects.
pub(super) fn canonical_color(value: &str) -> Option<String> {
    let [red, green, blue, alpha] = parse_rgba_channels(value)?;
    let rgb = hex_from_channels([red, green, blue]);
    Some(if alpha == u8::MAX {
        rgb
    } else {
        format!("{rgb}{alpha:02x}")
    })
}

fn hex_from_channels(channels: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", channels[0], channels[1], channels[2])
}

/// Canonicalizes a colour token to Strata's `#rrggbb` scheme representation.
pub(crate) fn color_to_hex(value: &str) -> String {
    parse_rgb_channels(value)
        .map(hex_from_channels)
        .unwrap_or_else(|| value.to_owned())
}

pub(super) fn blend(left: &str, right: &str, amount: f64) -> String {
    let (Some(left), Some(right)) = (parse_rgb_channels(left), parse_rgb_channels(right)) else {
        return right.to_owned();
    };
    let channel = |index: usize| {
        let a = f64::from(left[index]);
        let b = f64::from(right[index]);
        (a + (b - a) * amount).round() as u32
    };
    format!("#{:02x}{:02x}{:02x}", channel(0), channel(1), channel(2))
}
