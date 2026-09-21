// SPDX-License-Identifier: MIT

use std::{fs::File, io::Read, path::Path};

use super::embedded;
use crate::sandbox::CodeLanguage;

const TEXT_READ_LIMIT: u64 = 16 * 1024;
const CODE_READ_LIMIT: u64 = 4 * 1024;
const MARGIN: f64 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SyntaxRole {
    Keyword,
    String,
    Constant,
    Type,
    Comment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SyntaxSpan {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) role: SyntaxRole,
}

/// Renders the file's first lines as a document-style preview, like Finder's text icons.
pub(super) fn render(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    render_file(path, size, CodeLanguage::from_path(path))
}

pub(super) fn render_code(
    path: &Path,
    size: i32,
    language: CodeLanguage,
) -> Result<Vec<u8>, String> {
    render_file(path, size, Some(language))
}

fn render_file(path: &Path, size: i32, language: Option<CodeLanguage>) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut data = Vec::new();
    file.take(if language.is_some() {
        CODE_READ_LIMIT
    } else {
        TEXT_READ_LIMIT
    })
    .read_to_end(&mut data)
    .map_err(|error| error.to_string())?;
    if data.is_empty() || data.contains(&0) {
        return language.map_or_else(
            || Err("No readable text".to_owned()),
            |language| render_page("", size, Some(language)),
        );
    }
    let text = String::from_utf8_lossy(&data);
    let text = if text.starts_with("{\\rtf") {
        rtf_text(&text)
    } else if language.is_none() && looks_like_html(&text) {
        embedded::markup_text(&text)
    } else {
        text.into_owned()
    };
    if text.trim().is_empty() && language.is_none() {
        return Err("No readable text".to_owned());
    }
    render_page(&text, size, language)
}

fn looks_like_html(text: &str) -> bool {
    let head = text.trim_start();
    head.starts_with('<')
        && ["<html", "<!doctype", "<head", "<body"]
            .iter()
            .any(|marker| {
                head[..head.len().min(4096)]
                    .to_ascii_lowercase()
                    .contains(marker)
            })
}

// Raw RTF is control-word noise; extract the document text instead.
fn rtf_text(input: &str) -> String {
    // Destination groups that carry formatting data rather than document text.
    const SKIP: &[&str] = &[
        "annotation",
        "colortbl",
        "datastore",
        "filetbl",
        "fonttbl",
        "footer",
        "footnote",
        "generator",
        "header",
        "info",
        "latentstyles",
        "listtable",
        "pict",
        "revtbl",
        "rsidtbl",
        "stylesheet",
        "themedata",
        "xmlnstbl",
    ];
    let bytes = input.as_bytes();
    let mut text = Vec::with_capacity(input.len() / 4);
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                let mut j = i + 1;
                if j < bytes.len() && bytes[j] == b'\\' {
                    j += 1;
                    // \* marks an ignorable destination group.
                    let ignorable = j < bytes.len() && bytes[j] == b'*';
                    if ignorable {
                        j += 1;
                    }
                    let start = j;
                    while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
                        j += 1;
                    }
                    if ignorable || SKIP.contains(&&input[start..j]) {
                        let mut depth = 1;
                        let mut k = j;
                        while k < bytes.len() && depth > 0 {
                            match bytes[k] {
                                b'{' => depth += 1,
                                b'}' => depth -= 1,
                                _ => {}
                            }
                            k += 1;
                        }
                        i = k;
                        continue;
                    }
                }
                i += 1;
            }
            b'}' => i += 1,
            b'\\' => {
                i += 1;
                if i >= bytes.len() {
                    break;
                }
                match bytes[i] {
                    b'\'' if i + 2 < bytes.len() => {
                        if let Ok(byte) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                            text.push(byte);
                        }
                        i += 3;
                    }
                    b'{' | b'}' | b'\\' => {
                        text.push(bytes[i]);
                        i += 1;
                    }
                    b'~' => {
                        text.push(b' ');
                        i += 1;
                    }
                    byte if byte.is_ascii_alphabetic() => {
                        let start = i;
                        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                            i += 1;
                        }
                        let word = &input[start..i];
                        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'-') {
                            i += 1;
                        }
                        match word {
                            "par" | "line" | "row" => text.push(b'\n'),
                            "tab" | "bullet" => text.push(b' '),
                            _ => {}
                        }
                        // A space after a control word is the delimiter, not text.
                        if i < bytes.len() && bytes[i] == b' ' {
                            i += 1;
                        }
                    }
                    _ => i += 1,
                }
            }
            // Bare newlines in RTF source are markup layout, not content.
            b'\r' | b'\n' => i += 1,
            byte => {
                text.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&text).into_owned()
}

pub(super) fn syntax_spans(text: &str, language: CodeLanguage) -> Vec<SyntaxSpan> {
    const KEYWORDS: &[&str] = &[
        "as",
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "def",
        "defer",
        "do",
        "else",
        "enum",
        "export",
        "extends",
        "extern",
        "finally",
        "fn",
        "for",
        "foreach",
        "from",
        "func",
        "function",
        "if",
        "impl",
        "import",
        "in",
        "interface",
        "lambda",
        "let",
        "loop",
        "match",
        "mod",
        "move",
        "mut",
        "namespace",
        "new",
        "package",
        "private",
        "protected",
        "pub",
        "public",
        "raise",
        "ref",
        "return",
        "static",
        "struct",
        "super",
        "switch",
        "throw",
        "trait",
        "try",
        "type",
        "typeof",
        "unsafe",
        "use",
        "using",
        "var",
        "virtual",
        "when",
        "where",
        "while",
        "with",
        "yield",
    ];
    const TYPES: &[&str] = &[
        "bool", "byte", "char", "double", "f32", "f64", "float", "i8", "i16", "i32", "i64", "i128",
        "int", "isize", "long", "number", "object", "short", "str", "string", "u8", "u16", "u32",
        "u64", "u128", "uint", "usize", "void",
    ];
    const CONSTANTS: &[&str] = &[
        "false",
        "nil",
        "None",
        "null",
        "true",
        "undefined",
        "NaN",
        "Infinity",
    ];

    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    let hash_comments = matches!(
        language,
        CodeLanguage::Python
            | CodeLanguage::Ruby
            | CodeLanguage::Shell
            | CodeLanguage::R
            | CodeLanguage::Julia
            | CodeLanguage::Perl
            | CodeLanguage::PowerShell
    );
    let dash_comments = matches!(
        language,
        CodeLanguage::Lua | CodeLanguage::Haskell | CodeLanguage::Elm | CodeLanguage::Generic
    );

    while index < bytes.len() {
        let start = index;
        if bytes[index..].starts_with(b"<!--") {
            index = find_after(bytes, index + 4, b"-->");
            spans.push(SyntaxSpan {
                start,
                end: index,
                role: SyntaxRole::Comment,
            });
        } else if bytes[index..].starts_with(b"/*") {
            index = find_after(bytes, index + 2, b"*/");
            spans.push(SyntaxSpan {
                start,
                end: index,
                role: SyntaxRole::Comment,
            });
        } else if bytes[index..].starts_with(b"//")
            || (dash_comments && bytes[index..].starts_with(b"--"))
            || (hash_comments && bytes[index] == b'#')
        {
            index = bytes[index..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |offset| index + offset);
            spans.push(SyntaxSpan {
                start,
                end: index,
                role: SyntaxRole::Comment,
            });
        } else if bytes[index] == b'#'
            && bytes[..index]
                .iter()
                .rev()
                .take_while(|byte| **byte != b'\n')
                .all(u8::is_ascii_whitespace)
        {
            index = bytes[index..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |offset| index + offset);
            spans.push(SyntaxSpan {
                start,
                end: index,
                role: SyntaxRole::Keyword,
            });
        } else if matches!(bytes[index], b'\'' | b'"' | b'`') {
            let quote = bytes[index];
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index = (index + 2).min(bytes.len());
                } else {
                    let closed = bytes[index] == quote;
                    index += 1;
                    if closed {
                        break;
                    }
                }
            }
            spans.push(SyntaxSpan {
                start,
                end: index,
                role: SyntaxRole::String,
            });
        } else if bytes[index].is_ascii_digit() {
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric()
                    || matches!(bytes[index], b'.' | b'_' | b'x' | b'X'))
            {
                index += 1;
            }
            spans.push(SyntaxSpan {
                start,
                end: index,
                role: SyntaxRole::Constant,
            });
        } else if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let word = &text[start..index];
            let role = if KEYWORDS.contains(&word) {
                Some(SyntaxRole::Keyword)
            } else if TYPES.contains(&word) {
                Some(SyntaxRole::Type)
            } else if CONSTANTS.contains(&word) {
                Some(SyntaxRole::Constant)
            } else {
                None
            };
            if let Some(role) = role {
                spans.push(SyntaxSpan {
                    start,
                    end: index,
                    role,
                });
            }
        } else {
            index += 1;
        }
    }
    spans
}

fn find_after(bytes: &[u8], from: usize, needle: &[u8]) -> usize {
    bytes[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map_or(bytes.len(), |offset| from + offset + needle.len())
}

fn syntax_attributes(text: &str, language: CodeLanguage) -> gtk::pango::AttrList {
    let attributes = gtk::pango::AttrList::new();
    for span in syntax_spans(text, language) {
        let (red, green, blue) = match span.role {
            SyntaxRole::Keyword => (u16::MAX, 0, 0),
            SyntaxRole::String => (0, u16::MAX, 0),
            SyntaxRole::Constant => (0, 0, u16::MAX),
            SyntaxRole::Type => (u16::MAX, u16::MAX, 0),
            SyntaxRole::Comment => (0, u16::MAX, u16::MAX),
        };
        let mut attribute = gtk::pango::AttrColor::new_foreground(red, green, blue);
        attribute.set_start_index(span.start as u32);
        attribute.set_end_index(span.end as u32);
        attributes.insert(attribute);
    }
    attributes
}

pub(super) fn render_text(text: &str, size: i32) -> Result<Vec<u8>, String> {
    if text.trim().is_empty() {
        return Err("No readable text".to_owned());
    }
    render_page(text, size, None)
}

fn render_page(text: &str, size: i32, language: Option<CodeLanguage>) -> Result<Vec<u8>, String> {
    let (width, height) = (size * 3 / 4, size);
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height)
        .map_err(|error| error.to_string())?;
    let context = cairo::Context::new(&surface).map_err(|error| error.to_string())?;
    if language.is_some() {
        context.set_operator(cairo::Operator::Clear);
        context.paint().map_err(|error| error.to_string())?;
        context.set_operator(cairo::Operator::Over);
    } else {
        context.set_source_rgb(0.985, 0.99, 1.0);
        context.paint().map_err(|error| error.to_string())?;
    }

    let text_top = MARGIN;
    if !text.trim().is_empty() {
        let layout = pangocairo::functions::create_layout(&context);
        layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(
            if language.is_some() {
                "Monospace 8"
            } else {
                "Sans 9"
            },
        )));
        let inner = |extent: i32| (f64::from(extent) - MARGIN * 2.0) * f64::from(gtk::pango::SCALE);
        layout.set_width(inner(width) as i32);
        layout.set_height(
            ((f64::from(height) - text_top - MARGIN) * f64::from(gtk::pango::SCALE)) as i32,
        );
        layout.set_wrap(gtk::pango::WrapMode::WordChar);
        layout.set_ellipsize(gtk::pango::EllipsizeMode::End);
        layout.set_text(text);
        if let Some(language) = language {
            layout.set_attributes(Some(&syntax_attributes(text, language)));
            context.set_source_rgb(1.0, 1.0, 1.0);
        } else {
            context.set_source_rgb(0.16, 0.19, 0.24);
        }
        context.move_to(MARGIN, text_top);
        pangocairo::functions::show_layout(&context, &layout);
    }

    if language.is_none() {
        context.set_source_rgb(0.76, 0.80, 0.86);
        context.set_line_width(1.0);
        context.rectangle(0.5, 0.5, f64::from(width) - 1.0, f64::from(height) - 1.0);
        context.stroke().map_err(|error| error.to_string())?;
    }

    let mut png = Vec::new();
    surface
        .write_to_png(&mut png)
        .map_err(|error| error.to_string())?;
    Ok(png)
}
