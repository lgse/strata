// SPDX-License-Identifier: MIT

use crate::sandbox::Cancellation;

use super::document::DOCUMENT_MARKUP_LIMIT;

const CANCEL_CHECK_INTERVAL: usize = 4096;

// Windows-1252 fills 0x80..=0x9F, where Latin-1 has controls; undefined slots stay replacement.
const CP1252_HIGH: [char; 32] = [
    '€', '\u{fffd}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{fffd}', 'Ž',
    '\u{fffd}', '\u{fffd}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{fffd}',
    'ž', 'Ÿ',
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Style {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}

#[derive(Clone, Copy)]
struct Group {
    style: Style,
    unicode_skip: usize,
    skipped: bool,
}

impl Default for Group {
    fn default() -> Self {
        Self {
            style: Style::default(),
            unicode_skip: 1,
            skipped: false,
        }
    }
}

#[derive(Default)]
struct Writer {
    html: String,
    paragraph: bool,
    applied: Style,
}

impl Writer {
    fn text(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        if !self.paragraph {
            self.html.push_str("<p>");
            self.paragraph = true;
            self.applied = Style::default();
        }
        self.sync(style);
        for character in text.chars() {
            match character {
                '&' => self.html.push_str("&amp;"),
                '<' => self.html.push_str("&lt;"),
                '>' => self.html.push_str("&gt;"),
                _ => self.html.push(character),
            }
        }
    }

    fn sync(&mut self, style: Style) {
        if style == self.applied {
            return;
        }
        for (open, tag) in [
            (self.applied.strike, "</s>"),
            (self.applied.underline, "</u>"),
            (self.applied.italic, "</i>"),
            (self.applied.bold, "</b>"),
        ] {
            if open {
                self.html.push_str(tag);
            }
        }
        for (open, tag) in [
            (style.bold, "<b>"),
            (style.italic, "<i>"),
            (style.underline, "<u>"),
            (style.strike, "<s>"),
        ] {
            if open {
                self.html.push_str(tag);
            }
        }
        self.applied = style;
    }

    fn line_break(&mut self) {
        if self.paragraph {
            self.html.push_str("<br>");
        }
    }

    fn end_paragraph(&mut self) {
        if !self.paragraph {
            return;
        }
        self.sync(Style::default());
        self.html.push_str("</p>");
        self.paragraph = false;
    }

    fn finish(mut self) -> String {
        self.end_paragraph();
        self.html
    }
}

pub(crate) fn to_html(source: &str, cancellation: &Cancellation) -> Result<String, String> {
    if !source.trim_start().starts_with("{\\rtf") {
        return Err("Rendered preview is unavailable because the file is not RTF".to_owned());
    }
    let bytes = source.as_bytes();
    let mut groups = vec![Group::default()];
    let mut skipped = 0usize;
    let mut writer = Writer::default();
    let mut index = 0;
    let mut checkpoint = 0usize;

    while index < bytes.len() {
        if index >= checkpoint {
            checkpoint = index.saturating_add(CANCEL_CHECK_INTERVAL);
            if cancellation.is_cancelled() {
                return Err("Rendered preview was cancelled".to_owned());
            }
        }
        if writer.html.len() > DOCUMENT_MARKUP_LIMIT {
            return Err("Rendered preview exceeded the 4 MB markup limit".to_owned());
        }
        match bytes[index] {
            b'{' => {
                let parent = *groups.last().expect("root group");
                groups.push(Group {
                    skipped: false,
                    ..parent
                });
                index += 1;
            }
            b'}' => {
                if groups.len() > 1 && groups.pop().expect("nested group").skipped {
                    skipped = skipped.saturating_sub(1);
                }
                index += 1;
            }
            b'\\' => index = control(bytes, index, &mut groups, &mut skipped, &mut writer),
            b'\r' | b'\n' => index += 1,
            _ => {
                let end = bytes[index..]
                    .iter()
                    .position(|byte| matches!(byte, b'{' | b'}' | b'\\' | b'\r' | b'\n'))
                    .map_or(bytes.len(), |offset| index + offset);
                if skipped == 0 {
                    // Control-word offsets must not panic on a UTF-8 boundary.
                    writer.text(&String::from_utf8_lossy(&bytes[index..end]), style(&groups));
                }
                index = end;
            }
        }
    }
    Ok(writer.finish())
}

fn style(groups: &[Group]) -> Style {
    groups.last().expect("root group").style
}

fn control(
    bytes: &[u8],
    index: usize,
    groups: &mut [Group],
    skipped: &mut usize,
    writer: &mut Writer,
) -> usize {
    match bytes.get(index + 1) {
        None => index + 1,
        Some(b'\'') => {
            let value = bytes
                .get(index + 2..index + 4)
                .and_then(|digits| std::str::from_utf8(digits).ok())
                .and_then(|digits| u8::from_str_radix(digits, 16).ok());
            if let Some(byte) = value
                && *skipped == 0
            {
                let character = match byte {
                    0x80..=0x9f => CP1252_HIGH[usize::from(byte - 0x80)],
                    _ => char::from(byte),
                };
                writer.text(character.encode_utf8(&mut [0; 4]), style(groups));
            }
            if value.is_some() {
                index + 4
            } else {
                index + 2
            }
        }
        Some(byte) if byte.is_ascii_alphabetic() => {
            let (word, parameter, next) = control_word(bytes, index);
            apply(word, parameter, bytes, next, groups, skipped, writer)
        }
        Some(byte) => {
            match byte {
                b'*' => {
                    let group = groups.last_mut().expect("root group");
                    if !group.skipped {
                        group.skipped = true;
                        *skipped += 1;
                    }
                }
                b'\r' | b'\n' => writer.end_paragraph(),
                b'\\' | b'{' | b'}' | b'~' | b'_' if *skipped == 0 => {
                    let literal = match byte {
                        b'~' => '\u{a0}',
                        b'_' => '-',
                        _ => char::from(*byte),
                    };
                    writer.text(literal.encode_utf8(&mut [0; 4]), style(groups));
                }
                _ => {}
            }
            index + 2
        }
    }
}

fn control_word(bytes: &[u8], index: usize) -> (&str, Option<i32>, usize) {
    let start = index + 1;
    let mut end = start;
    while bytes.get(end).is_some_and(u8::is_ascii_alphabetic) {
        end += 1;
    }
    let word = std::str::from_utf8(&bytes[start..end]).unwrap_or_default();
    let mut cursor = end;
    let negative = bytes.get(cursor) == Some(&b'-');
    if negative {
        cursor += 1;
    }
    let digits = cursor;
    while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
        cursor += 1;
    }
    let parameter = std::str::from_utf8(&bytes[digits..cursor])
        .ok()
        .and_then(|digits| digits.parse::<i32>().ok())
        .map(|value| if negative { -value } else { value });
    if bytes.get(cursor) == Some(&b' ') {
        cursor += 1;
    }
    (word, parameter, cursor)
}

fn apply(
    word: &str,
    parameter: Option<i32>,
    bytes: &[u8],
    next: usize,
    groups: &mut [Group],
    skipped: &mut usize,
    writer: &mut Writer,
) -> usize {
    let enabled = parameter != Some(0);
    let group = groups.last_mut().expect("root group");
    match word {
        "b" => group.style.bold = enabled,
        "i" => group.style.italic = enabled,
        "ul" => group.style.underline = enabled,
        "ulnone" => group.style.underline = false,
        "strike" => group.style.strike = enabled,
        "plain" => group.style = Style::default(),
        "uc" => group.unicode_skip = parameter.unwrap_or(1).clamp(0, 32) as usize,
        "u" => {
            let skip = group.unicode_skip;
            if let Some(value) = parameter {
                // \uN is a signed 16-bit code unit; negative values wrap.
                let scalar = if value < 0 {
                    (i64::from(value) + 0x1_0000) as u32
                } else {
                    value as u32
                };
                if *skipped == 0
                    && let Some(character) = char::from_u32(scalar)
                {
                    let style = style(groups);
                    writer.text(character.encode_utf8(&mut [0; 4]), style);
                }
                return skip_fallback(bytes, next, skip);
            }
        }
        // Lossy decoding erased byte lengths; character counts only approximate
        // the original binary payload, but cannot split a UTF-8 sequence.
        "bin" => {
            let count = parameter.unwrap_or(0).max(0) as usize;
            let mut index = next;
            for _ in 0..count {
                if index >= bytes.len() {
                    break;
                }
                index = next_character(bytes, index);
            }
            return index;
        }
        "par" | "sect" | "page" | "row" | "pard" if *skipped == 0 => writer.end_paragraph(),
        "line" if *skipped == 0 => writer.line_break(),
        "tab" | "cell" if *skipped == 0 => {
            let style = style(groups);
            writer.text("\t", style);
        }
        _ => {
            if is_skipped_destination(word) && !group.skipped {
                group.skipped = true;
                *skipped += 1;
            }
        }
    }
    next
}

/// Skips the plain-text characters a `\uN` escape repeats for legacy readers.
fn skip_fallback(bytes: &[u8], mut index: usize, count: usize) -> usize {
    for _ in 0..count {
        match bytes.get(index) {
            Some(b'\\') if bytes.get(index + 1) == Some(&b'\'') => index += 4,
            Some(b'\\') if bytes.get(index + 1).is_some_and(u8::is_ascii_alphabetic) => {
                index = control_word(bytes, index).2;
            }
            Some(b'{' | b'}') | None => break,
            Some(_) => index = next_character(bytes, index),
        }
    }
    index.min(bytes.len())
}

fn next_character(bytes: &[u8], mut index: usize) -> usize {
    index += 1;
    while bytes.get(index).is_some_and(|byte| byte & 0xc0 == 0x80) {
        index += 1;
    }
    index.min(bytes.len())
}

fn is_skipped_destination(word: &str) -> bool {
    matches!(
        word,
        "fonttbl"
            | "colortbl"
            | "stylesheet"
            | "listtable"
            | "listoverridetable"
            | "revtbl"
            | "rsidtbl"
            | "filetbl"
            | "xmlnstbl"
            | "info"
            | "pict"
            | "object"
            | "objdata"
            | "themedata"
            | "colorschememapping"
            | "datastore"
            | "latentstyles"
            | "generator"
            | "fldinst"
            | "footnote"
            | "annotation"
            | "atnauthor"
            | "atnid"
            | "header"
            | "headerl"
            | "headerr"
            | "headerf"
            | "footer"
            | "footerl"
            | "footerr"
            | "footerf"
            | "bkmkstart"
            | "bkmkend"
            | "panose"
            | "falt"
    )
}

#[cfg(test)]
mod tests;
