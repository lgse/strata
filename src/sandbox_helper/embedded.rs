// SPDX-License-Identifier: MIT

use std::{
    fs,
    io::{self, Read},
    path::Path,
};

use html5gum::{DefaultEmitter, Token, Tokenizer};

use super::{books, read_limited, scale_embedded_thumbnail, text};

const EMBEDDED_ENTRY_LIMIT: u64 = 16 * 1024 * 1024;
const CONTENT_READ_LIMIT: u64 = 256 * 1024;
const MIN_CONTENT_TEXT: usize = 32;

// OOXML keeps a saved preview at docProps/thumbnail.* and ODF at Thumbnails/thumbnail.png.
const PREFERRED_ENTRIES: &[&str] = &[
    "docProps/thumbnail.jpeg",
    "docProps/thumbnail.png",
    "Thumbnails/thumbnail.png",
];

// Document containers carry their readable content in a fixed entry.
const CONTENT_ENTRIES: &[&str] = &[
    "word/document.xml",
    "ppt/slides/slide1.xml",
    "xl/sharedStrings.xml",
    "content.xml",
];

pub(super) fn render(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    render_zip(path, size)
        .or_else(|_| render_rar(path, size))
        .or_else(|_| books::render(path, size))
}

fn render_zip(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    for name in CONTENT_ENTRIES {
        if let Some(png) = zip_entry_document(&mut archive, name, size) {
            return Ok(png);
        }
    }
    // EPUBs keep chapter content under arbitrary document names.
    let names = archive
        .file_names()
        .filter(|name| content_document(name))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for name in names {
        if let Some(png) = zip_entry_document(&mut archive, &name, size) {
            return Ok(png);
        }
    }
    for name in PREFERRED_ENTRIES {
        if let Some(png) = zip_entry_png(&mut archive, name, size) {
            return Ok(png);
        }
    }
    // EPUB covers and comic pages live under arbitrary names; prefer "cover", else first image.
    let mut names = archive
        .file_names()
        .map(str::to_owned)
        .filter(|name| image_entry(name))
        .collect::<Vec<_>>();
    names.sort_by_cached_key(|name| {
        (
            !file_name(name).to_ascii_lowercase().starts_with("cover"),
            name.to_ascii_lowercase(),
        )
    });
    for name in names {
        if let Some(png) = zip_entry_png(&mut archive, &name, size) {
            return Ok(png);
        }
    }
    Err("Archive contains no embedded thumbnail".to_owned())
}

fn render_rar(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    let mut archive = unrar::Archive::new(path)
        .open_for_processing()
        .map_err(|error| error.to_string())?;
    loop {
        let Some(header) = archive.read_header().map_err(|error| error.to_string())? else {
            return Err("Archive contains no embedded thumbnail".to_owned());
        };
        let name = header.entry().filename.to_string_lossy().into_owned();
        archive = if header.entry().is_file() && image_entry(&name) {
            let (bytes, _rest) = header.read().map_err(|error| error.to_string())?;
            return scale_embedded_thumbnail(&bytes, size);
        } else {
            header.skip().map_err(|error| error.to_string())?
        };
    }
}

fn file_name(entry: &str) -> &str {
    entry.rsplit('/').next().unwrap_or(entry)
}

fn image_entry(name: &str) -> bool {
    if file_name(name).starts_with('.') {
        return false;
    }
    matches!(
        Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp")
    )
}

fn zip_entry_png<R: Read + io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    size: i32,
) -> Option<Vec<u8>> {
    let entry = archive.by_name(name).ok()?;
    if entry.size() > EMBEDDED_ENTRY_LIMIT {
        return None;
    }
    let bytes = read_limited(entry, EMBEDDED_ENTRY_LIMIT).ok()?;
    scale_embedded_thumbnail(&bytes, size).ok()
}

fn content_document(name: &str) -> bool {
    matches!(
        Path::new(name)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("xhtml" | "html" | "htm")
    )
}

fn zip_entry_document<R: Read + io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
    size: i32,
) -> Option<Vec<u8>> {
    let entry = archive.by_name(name).ok()?;
    let bytes = read_limited(entry, CONTENT_READ_LIMIT).ok()?;
    let text = markup_text(&String::from_utf8_lossy(&bytes));
    if text.trim().len() < MIN_CONTENT_TEXT {
        return None;
    }
    text::render_text(&text, size).ok()
}

pub(super) fn markup_text(markup: &str) -> String {
    let mut text = String::with_capacity(markup.len() / 2);
    let mut hidden = false;
    for token in Tokenizer::new_with_emitter(markup.as_bytes(), DefaultEmitter::default()) {
        match token {
            Ok(Token::StartTag(tag)) if matches!(tag.name.as_slice(), b"style" | b"script") => {
                hidden = true
            }
            Ok(Token::EndTag(tag)) if matches!(tag.name.as_slice(), b"style" | b"script") => {
                hidden = false
            }
            _ if hidden => {}
            Ok(Token::String(chunk)) => text.push_str(&String::from_utf8_lossy(&chunk)),
            Ok(Token::EndTag(tag)) if block_tag(&tag.name) => text.push('\n'),
            Ok(Token::StartTag(tag)) if tag.name == b"br" => text.push('\n'),
            _ => {}
        }
    }
    text
}

fn block_tag(name: &[u8]) -> bool {
    let local = name.rsplit(|&byte| byte == b':').next().unwrap_or(name);
    matches!(
        local,
        b"p" | b"h"
            | b"h1"
            | b"h2"
            | b"h3"
            | b"h4"
            | b"h5"
            | b"h6"
            | b"li"
            | b"tr"
            | b"td"
            | b"div"
            | b"section"
            | b"title"
            | b"si"
    )
}
