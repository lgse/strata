// SPDX-License-Identifier: MIT

//! The bytes of a dropped type, as what they are: an image as a PNG, a browser's file name,
//! and text in its charset.

use std::io::Cursor;

use strata_components::contract::dnd::names;

use super::FILE_CONTENTS;

const TEXT: &str = "text/plain";
/// X11's name for UTF-8 text.
const UTF8_STRING: &str = "UTF8_STRING";

/// An image's data as a PNG, as GTK saves a dropped texture.
pub(super) fn png_of(bytes: &[u8]) -> Option<Vec<u8>> {
    if image::guess_format(bytes).ok()? == image::ImageFormat::Png {
        return Some(bytes.to_vec());
    }
    let decoded = image::load_from_memory(bytes).ok()?;
    let mut png = Cursor::new(Vec::new());
    decoded.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

/// The name in `application/octet-stream;name="…"`, its escapes undone.
pub(super) fn supplied_name(mime: &str) -> Option<String> {
    let quoted = mime
        .strip_prefix(FILE_CONTENTS)?
        .strip_prefix(";name=\"")?
        .strip_suffix('"')?;
    let mut name = String::with_capacity(quoted.len());
    let mut escaped = false;
    for c in quoted.chars() {
        if escaped || c != '\\' {
            name.push(c);
            escaped = false;
        } else {
            escaped = true;
        }
    }
    Some(name)
}

/// Whether `mime` carries text: `text/plain`, in any charset, or X11's `UTF8_STRING`.
pub(super) fn is_text(mime: &str) -> bool {
    names(TEXT, mime) || mime == UTF8_STRING
}

/// The text of `bytes`, offered as `mime`, converted from its charset as GTK's string reader
/// converts it, or `None` when they are not text in it.
///
/// - `charset=utf-8` (in any case) and `UTF8_STRING` are UTF-8, taken only when valid.
/// - Any other charset GLib knows is converted to UTF-8 (`g_convert`). GTK takes UTF-8 and
///   the locale's charset; a source naming another (UTF-16, say) gets its text read too,
///   rather than skipped.
/// - Plain `text/plain` names no charset. GTK reads it as ASCII; Strata reads UTF-8, of which
///   ASCII is a part, since other toolkits write UTF-8 there.
pub(super) fn text(mime: &str, bytes: &[u8]) -> Option<String> {
    match charset(mime) {
        Some(charset) if !is_utf8(charset) => {
            let (converted, _) = glib::convert(bytes, "UTF-8", charset).ok()?;
            String::from_utf8(converted.to_vec()).ok()
        }
        _ => String::from_utf8(bytes.to_vec()).ok(),
    }
}

/// The `charset` parameter of `mime`, unquoted.
fn charset(mime: &str) -> Option<&str> {
    mime.split(';').skip(1).find_map(|parameter| {
        let (key, value) = parameter.split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches('"'))
    })
}

fn is_utf8(charset: &str) -> bool {
    charset.eq_ignore_ascii_case("utf-8") || charset.eq_ignore_ascii_case("utf8")
}
