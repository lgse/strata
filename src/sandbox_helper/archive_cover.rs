// SPDX-License-Identifier: MIT

use std::{fs, io::Read, path::Path};

use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};

use crate::sandbox::CoverFormat;

pub(crate) const MAX_INPUT_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const MAX_ENTRIES: usize = 4096;
pub(super) const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_XML_BYTES: u64 = 256 * 1024;

pub(super) fn image_name(name: &str) -> bool {
    matches!(
        name.rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "gif"
    )
}

pub(super) fn compare_names(left: &str, right: &str) -> std::cmp::Ordering {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    let (mut l, mut r) = (0, 0);
    while l < left.len() && r < right.len() {
        if left[l].is_ascii_digit() && right[r].is_ascii_digit() {
            let start_l = l;
            let start_r = r;
            while l < left.len() && left[l].is_ascii_digit() {
                l += 1;
            }
            while r < right.len() && right[r].is_ascii_digit() {
                r += 1;
            }
            let a = left[start_l..l]
                .iter()
                .position(|c| *c != b'0')
                .map_or(l, |n| start_l + n);
            let b = right[start_r..r]
                .iter()
                .position(|c| *c != b'0')
                .map_or(r, |n| start_r + n);
            let order = (l - a)
                .cmp(&(r - b))
                .then_with(|| left[a..l].cmp(&right[b..r]));
            if !order.is_eq() {
                return order;
            }
        } else {
            let order = left[l]
                .to_ascii_lowercase()
                .cmp(&right[r].to_ascii_lowercase());
            if !order.is_eq() {
                return order;
            }
            l += 1;
            r += 1;
        }
    }
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

pub(crate) fn thumbnail(input: &Path, format: CoverFormat) -> Result<Vec<u8>, String> {
    render(input, format, 256)
}

pub(crate) fn render(input: &Path, format: CoverFormat, edge: i32) -> Result<Vec<u8>, String> {
    if fs::metadata(input).map_err(|e| e.to_string())?.len() > MAX_INPUT_BYTES {
        return Err("Cover archive exceeds the input size limit".into());
    }
    let bytes = match format {
        CoverFormat::Cbr => super::archive_rar::cover_image(input)?,
        CoverFormat::Cbz | CoverFormat::Epub => {
            let mut archive =
                zip::ZipArchive::new(fs::File::open(input).map_err(|e| e.to_string())?)
                    .map_err(|_| "Invalid cover archive")?;
            if archive.len() > MAX_ENTRIES {
                return Err("Cover archive entry limit exceeded".into());
            }
            let name = if format == CoverFormat::Cbz {
                let mut first = None::<String>;
                for i in 0..archive.len() {
                    let file = archive
                        .by_index_raw(i)
                        .map_err(|_| "Invalid cover archive")?;
                    if !file.is_dir()
                        && image_name(file.name())
                        && first
                            .as_deref()
                            .is_none_or(|name| compare_names(file.name(), name).is_lt())
                    {
                        first = Some(file.name().to_owned());
                    }
                }
                first.ok_or("Comic archive has no bounded image")?
            } else {
                epub_cover(&mut archive)?
            };
            read_member(&mut archive, &name, MAX_IMAGE_BYTES)?
        }
    };
    render_image(&bytes, edge)
}

fn read_member(
    archive: &mut zip::ZipArchive<fs::File>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>, String> {
    let file = archive
        .by_name(name)
        .map_err(|_| "Cover archive member is missing")?;
    if file.size() > limit {
        return Err("Cover archive member exceeds the size limit".into());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Invalid cover archive member")?;
    if bytes.len() as u64 > limit {
        return Err("Cover archive member exceeds the size limit".into());
    }
    Ok(bytes)
}

fn attribute(tag: &BytesStart<'_>, key: &[u8]) -> Option<String> {
    tag.attributes()
        .flatten()
        .find(|attr| attr.key.local_name().as_ref() == key)
        .and_then(|attr| {
            attr.decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, tag.decoder())
                .ok()
                .map(|value| value.into_owned())
        })
}

fn rootfile(xml: &[u8]) -> Result<String, String> {
    let mut reader = Reader::from_reader(xml);
    loop {
        match reader.read_event().map_err(|_| "Invalid EPUB container")? {
            Event::Start(tag) | Event::Empty(tag) if tag.local_name().as_ref() == b"rootfile" => {
                return attribute(&tag, b"full-path").ok_or("EPUB has no rootfile".into());
            }
            Event::Eof => return Err("EPUB has no rootfile".into()),
            _ => {}
        }
    }
}

fn cover_href(xml: &[u8]) -> Result<String, String> {
    let mut reader = Reader::from_reader(xml);
    let mut cover_id = None;
    let mut items = Vec::new();
    loop {
        match reader.read_event().map_err(|_| "Invalid EPUB package")? {
            Event::Start(tag) | Event::Empty(tag) => match tag.local_name().as_ref() {
                b"meta" if attribute(&tag, b"name").as_deref() == Some("cover") => {
                    cover_id = attribute(&tag, b"content");
                }
                b"item" => {
                    if let (Some(id), Some(href), Some(media)) = (
                        attribute(&tag, b"id"),
                        attribute(&tag, b"href"),
                        attribute(&tag, b"media-type"),
                    ) {
                        items.push((id, href, media, attribute(&tag, b"properties")));
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    let supported = |href: &str, media: &str| {
        image_name(href)
            && matches!(
                media,
                "image/jpeg" | "image/png" | "image/webp" | "image/gif"
            )
    };
    items
        .iter()
        .find(|(_, href, media, properties)| {
            supported(href, media)
                && properties
                    .as_deref()
                    .is_some_and(|value| value.split_whitespace().any(|p| p == "cover-image"))
        })
        .or_else(|| {
            items.iter().find(|(id, href, media, _)| {
                supported(href, media) && cover_id.as_deref() == Some(id.as_str())
            })
        })
        .map(|(_, href, _, _)| href.clone())
        .ok_or("EPUB has no cover image".into())
}

fn epub_cover(archive: &mut zip::ZipArchive<fs::File>) -> Result<String, String> {
    let container = read_member(archive, "META-INF/container.xml", MAX_XML_BYTES)?;
    let package_path = rootfile(&container)?;
    let package_path = normalize_path("", &package_path).ok_or("Invalid EPUB package path")?;
    let xml = read_member(archive, &package_path, MAX_XML_BYTES)?;
    let href = cover_href(&xml)?;
    let base = package_path.rsplit_once('/').map_or("", |(base, _)| base);
    let relative = normalize_path(base, &href).ok_or("Invalid EPUB cover path")?;
    // Some EPUB 2 generators place a root-relative href in a nested package document.
    if archive.file_names().any(|name| name == relative) {
        return Ok(relative);
    }
    normalize_path("", &href)
        .filter(|name| archive.file_names().any(|entry| entry == name))
        .ok_or("EPUB cover is missing".into())
}

fn normalize_path(base: &str, href: &str) -> Option<String> {
    let href = glib::uri_unescape_string(href.split(['?', '#']).next()?, None::<&str>)?;
    let href = href.as_str();
    if href.starts_with('/') || href.contains('\\') {
        return None;
    }
    let mut parts: Vec<&str> = base.split('/').filter(|part| !part.is_empty()).collect();
    for part in href.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn render_image(bytes: &[u8], edge: i32) -> Result<Vec<u8>, String> {
    use gdk_pixbuf::prelude::*;
    let loader = gdk_pixbuf::PixbufLoader::new();
    let too_large = std::rc::Rc::new(std::cell::Cell::new(false));
    let flag = too_large.clone();
    loader.connect_size_prepared(move |loader, width, height| {
        if width <= 0
            || height <= 0
            || u64::from(width as u32) * u64::from(height as u32) > 16 * 1024 * 1024
        {
            flag.set(true);
        } else {
            let scale = (f64::from(edge) / f64::from(width))
                .min(f64::from(edge) / f64::from(height))
                .min(1.0);
            loader.set_size(
                (f64::from(width) * scale).round().max(1.0) as i32,
                (f64::from(height) * scale).round().max(1.0) as i32,
            );
        }
    });
    for chunk in bytes.chunks(16 * 1024) {
        loader
            .write(chunk)
            .map_err(|error| format!("Invalid cover image (write): {error}"))?;
        if too_large.get() {
            return Err("Cover image exceeds the pixel limit".into());
        }
    }
    loader
        .close()
        .map_err(|error| format!("Invalid cover image (close): {error}"))?;
    let pixbuf = loader.pixbuf().ok_or("Invalid cover image")?;
    let scale = (f64::from(edge) / f64::from(pixbuf.width()))
        .min(f64::from(edge) / f64::from(pixbuf.height()))
        .min(1.0);
    let pixbuf = if scale < 1.0 {
        pixbuf
            .scale_simple(
                (f64::from(pixbuf.width()) * scale).round().max(1.0) as i32,
                (f64::from(pixbuf.height()) * scale).round().max(1.0) as i32,
                gdk_pixbuf::InterpType::Bilinear,
            )
            .ok_or("Cannot scale cover image")?
    } else {
        pixbuf
    };
    pixbuf
        .save_to_bufferv("png", &[("compression", "1")])
        .map_err(|_| "Cannot encode cover image".into())
}

#[cfg(test)]
mod tests;
