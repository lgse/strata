// SPDX-License-Identifier: MIT

use std::{
    collections::BTreeMap,
    io::{Cursor, Read, Write},
};

use quick_xml::{
    NsReader, Writer,
    events::{BytesEnd, BytesStart, Event},
    name::ResolveResult,
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

const XML_LIMIT: u64 = 64 * 1024 * 1024;

// docx-rs 0.4.22 dispatches runs by literal prefix rather than namespace URI.
pub(super) fn normalize_package(bytes: &[u8]) -> Result<Vec<u8>, String> {
    normalize_package_with_limit(bytes, XML_LIMIT)
}

fn normalize_package_with_limit(bytes: &[u8], limit: u64) -> Result<Vec<u8>, String> {
    let mut input = ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut output = ZipWriter::new(Cursor::new(Vec::new()));
    let mut remaining = limit;
    let mut output_remaining = limit;
    for index in 0..input.len() {
        let mut part = input.by_index(index).map_err(|e| e.to_string())?;
        if !part.name().ends_with(".xml") && !part.name().ends_with(".rels") {
            output.raw_copy_file(part).map_err(|e| e.to_string())?;
            continue;
        }
        let mut xml = Vec::new();
        part.by_ref()
            .take(remaining + 1)
            .read_to_end(&mut xml)
            .map_err(|e| e.to_string())?;
        if xml.len() as u64 > remaining {
            return Err("Document XML exceeds the 64 MiB preview limit".into());
        }
        remaining -= xml.len() as u64;
        let normalized = normalize_xml(&xml)?;
        output_remaining = output_remaining
            .checked_sub(normalized.len() as u64)
            .ok_or("Normalized document XML exceeds the preview limit")?;
        output
            .start_file(part.name(), SimpleFileOptions::default())
            .map_err(|e| e.to_string())?;
        output.write_all(&normalized).map_err(|e| e.to_string())?;
    }
    output
        .finish()
        .map(|file| file.into_inner())
        .map_err(|e| e.to_string())
}

fn normalize_xml(xml: &[u8]) -> Result<Vec<u8>, String> {
    let mut declarations = BTreeMap::new();
    // Local xmlns:w attributes are mistaken for table widths by docx-rs.
    normalize_xml_pass(xml, &mut declarations, false)?;
    normalize_xml_pass(xml, &mut declarations, true)
}

fn normalize_xml_pass(
    xml: &[u8],
    declarations: &mut BTreeMap<String, String>,
    emit: bool,
) -> Result<Vec<u8>, String> {
    let mut reader = NsReader::from_reader(xml);
    let mut writer = Writer::new(Vec::new());
    let mut prefixes = BTreeMap::new();
    let mut open = Vec::new();
    loop {
        if writer.get_ref().len() as u64 > XML_LIMIT {
            return Err("Normalized document XML exceeds the 64 MiB preview limit".into());
        }
        let event = reader.read_event().map_err(|e| e.to_string())?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let (namespace, local) = reader.resolver().resolve_element(element.name());
                let name = qualified_name(namespace, local.as_ref(), &mut prefixes, declarations)?;
                let mut normalized = BytesStart::new(name.as_str());
                let mut attributes = Vec::new();
                for attribute in element.attributes() {
                    let attribute = attribute.map_err(|e| e.to_string())?;
                    if attribute.key.as_ref() == b"xmlns"
                        || attribute.key.as_ref().starts_with(b"xmlns:")
                    {
                        continue;
                    }
                    let (namespace, local) = reader.resolver().resolve_attribute(attribute.key);
                    let key =
                        qualified_name(namespace, local.as_ref(), &mut prefixes, declarations)?;
                    let value = attribute
                        .decoded_and_normalized_value(
                            quick_xml::XmlVersion::Implicit1_0,
                            reader.decoder(),
                        )
                        .map_err(|e| e.to_string())?;
                    attributes.push((key, value.into_owned()));
                }
                for (key, value) in &attributes {
                    normalized.push_attribute((key.as_str(), value.as_str()));
                }
                if emit && open.is_empty() {
                    for (prefix, uri) in declarations.iter() {
                        normalized
                            .push_attribute((format!("xmlns:{prefix}").as_str(), uri.as_str()));
                    }
                }
                if matches!(event, Event::Empty(_)) {
                    writer.write_event(Event::Empty(normalized))
                } else {
                    open.push(name.clone());
                    writer.write_event(Event::Start(normalized))
                }
                .map_err(|e| e.to_string())?;
            }
            Event::End(_) => {
                let name = open.pop().ok_or("Unbalanced document XML")?;
                writer
                    .write_event(Event::End(BytesEnd::new(name)))
                    .map_err(|e| e.to_string())?;
            }
            Event::DocType(_) => {
                return Err("Document XML declarations with DTDs are unsupported".into());
            }
            Event::Eof => {
                if !open.is_empty() {
                    return Err("Unclosed document XML element".into());
                }
                return Ok(writer.into_inner());
            }
            event => writer.write_event(event).map_err(|e| e.to_string())?,
        }
    }
}

fn qualified_name(
    namespace: ResolveResult<'_>,
    local: &[u8],
    prefixes: &mut BTreeMap<String, String>,
    declarations: &mut BTreeMap<String, String>,
) -> Result<String, String> {
    let local = std::str::from_utf8(local).map_err(|e| e.to_string())?;
    let uri = match namespace {
        ResolveResult::Unbound => return Ok(local.to_owned()),
        ResolveResult::Unknown(_) => return Err("Unbound document XML namespace prefix".into()),
        ResolveResult::Bound(namespace) => std::str::from_utf8(namespace.as_ref())
            .map_err(|e| e.to_string())?
            .to_owned(),
    };
    let prefix = match uri.as_str() {
        "http://schemas.openxmlformats.org/wordprocessingml/2006/main" => "w".to_owned(),
        "http://schemas.openxmlformats.org/drawingml/2006/main" => "a".to_owned(),
        "http://schemas.openxmlformats.org/markup-compatibility/2006" => "mc".to_owned(),
        "urn:schemas-microsoft-com:vml" => "v".to_owned(),
        "http://www.w3.org/XML/1998/namespace" => "xml".to_owned(),
        _ => {
            let next = prefixes.len();
            prefixes
                .entry(uri.clone())
                .or_insert_with(|| format!("ns{next}"))
                .clone()
        }
    };
    if prefix != "xml" {
        declarations.insert(prefix.clone(), uri);
    }
    Ok(format!("{prefix}:{local}"))
}

#[cfg(test)]
mod tests;
