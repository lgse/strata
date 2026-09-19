// SPDX-License-Identifier: MIT

use std::{ffi::OsStr, fmt::Write as _, path::Path};

use docx_rs::{
    Bold, DocumentChild, Docx, Italic, Numberings, Paragraph, ParagraphChild, Run, RunChild,
    RunProperty, Table, TableCellContent, TableChild, TableRowChild, Underline,
};
use serde::{Deserialize, Serialize};

use crate::sandbox::Cancellation;

use super::document::{DOCUMENT_INPUT_LIMIT, DocumentKind, ParsedDocument, parse_document};

pub(crate) const DOCX_BYTE_LIMIT: u64 = 20 * 1024 * 1024;
const HTML_LIMIT: usize = DOCUMENT_INPUT_LIMIT - 4096;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RichTextData {
    pub html: String,
    pub truncated: bool,
}

pub(crate) fn is_document(content_type: &str, name: &OsStr) -> bool {
    content_type == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        || Path::new(name)
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case("docx"))
}

impl RichTextData {
    pub(crate) fn into_document(
        self,
        cancellation: &Cancellation,
    ) -> Result<ParsedDocument, String> {
        let mut parsed = parse_document(DocumentKind::Html, &self.html, cancellation)?;
        if self.truncated {
            parsed.warnings.push(
                "Document preview reached its size budget; open the file to see all content."
                    .to_owned(),
            );
        }
        Ok(parsed)
    }

    pub(crate) fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let data: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        if data.html.len() > DOCUMENT_INPUT_LIMIT {
            return Err("Document preview budget exceeded".into());
        }
        Ok(data)
    }
}

// Called only by the resource-limited sandbox helper: reading a Word file
// decompresses every part into memory before any output budget applies.
pub(crate) fn read_document(path: &Path) -> Result<RichTextData, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > DOCX_BYTE_LIMIT {
        return Err("Document is too large to preview safely".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let docx = docx_rs::read_docx(&bytes).map_err(|e| e.to_string())?;
    Ok(to_html(&docx))
}

#[derive(Default)]
struct Body {
    html: String,
    lists: Vec<&'static str>,
    truncated: bool,
}

impl Body {
    fn paragraph(&mut self, paragraph: &Paragraph, numberings: &Numberings) {
        let markup = inline_markup(&paragraph.children);
        if markup.trim().is_empty() {
            return;
        }
        if !self.fits(markup.len()) {
            return;
        }
        let property = &paragraph.property;
        if let Some(numbering) = &property.numbering_property {
            let level = numbering.level.as_ref().map_or(0, |level| level.val).min(8);
            let ordered = numbering
                .id
                .as_ref()
                .is_some_and(|id| is_ordered(numberings, id.id, level));
            self.open_lists(level + 1, ordered);
            let _ = write!(self.html, "<li>{markup}</li>");
        } else {
            self.close_lists();
            match property.style.as_ref().map(|style| style.val.as_str()) {
                Some(style) => match named_block(style) {
                    Block::Heading(level) => {
                        let _ = write!(self.html, "<h{level}>{markup}</h{level}>");
                    }
                    Block::Quote => {
                        let _ = write!(self.html, "<blockquote>{markup}</blockquote>");
                    }
                    Block::Paragraph => {
                        let _ = write!(self.html, "<p>{markup}</p>");
                    }
                },
                None => {
                    let _ = write!(self.html, "<p>{markup}</p>");
                }
            }
        }
    }

    fn fits(&mut self, addition: usize) -> bool {
        if self.html.len().saturating_add(addition) > HTML_LIMIT {
            self.truncated = true;
            return false;
        }
        true
    }

    fn table(&mut self, table: &Table) {
        self.close_lists();
        self.html.push_str("<table>");
        for (index, child) in table.rows.iter().enumerate() {
            let TableChild::TableRow(row) = child;
            let (open, close) = if index == 0 {
                ("<th>", "</th>")
            } else {
                ("<td>", "</td>")
            };
            let mut html = String::from("<tr>");
            for cell in &row.cells {
                let TableRowChild::TableCell(cell) = cell;
                html.push_str(open);
                // Cells hold block content; the rendered table shows one value per cell.
                let paragraphs = cell.children.iter().filter_map(|content| match content {
                    TableCellContent::Paragraph(paragraph) => {
                        Some(inline_markup(&paragraph.children))
                    }
                    _ => None,
                });
                let mut first = true;
                for markup in paragraphs.filter(|markup| !markup.trim().is_empty()) {
                    if !first {
                        html.push(' ');
                    }
                    first = false;
                    html.push_str(&markup);
                }
                html.push_str(close);
            }
            html.push_str("</tr>");
            if !self.fits(html.len()) {
                break;
            }
            self.html.push_str(&html);
        }
        self.html.push_str("</table>");
    }

    fn open_lists(&mut self, depth: usize, ordered: bool) {
        let tag = if ordered { "ol" } else { "ul" };
        // A list whose numbering format changed has to restart, not continue.
        while self.lists.len() > depth
            || (depth > 0 && self.lists.len() == depth && self.lists[depth - 1] != tag)
        {
            let open = self.lists.pop().expect("open list");
            let _ = write!(self.html, "</{open}>");
        }
        while self.lists.len() < depth {
            let _ = write!(self.html, "<{tag}>");
            self.lists.push(tag);
        }
    }

    fn close_lists(&mut self) {
        self.open_lists(0, false);
    }

    fn finish(mut self) -> RichTextData {
        self.close_lists();
        RichTextData {
            html: self.html,
            truncated: self.truncated,
        }
    }
}

fn to_html(docx: &Docx) -> RichTextData {
    let mut body = Body::default();
    for child in &docx.document.children {
        match child {
            DocumentChild::Paragraph(paragraph) => body.paragraph(paragraph, &docx.numberings),
            DocumentChild::Table(table) => body.table(table),
            _ => {}
        }
        if body.truncated {
            break;
        }
    }
    body.finish()
}

fn inline_markup(children: &[ParagraphChild]) -> String {
    let mut markup = String::new();
    for child in children {
        match child {
            ParagraphChild::Run(run) => markup.push_str(&run_markup(run)),
            ParagraphChild::Hyperlink(link) => markup.push_str(&inline_markup(&link.children)),
            _ => {}
        }
    }
    markup
}

fn run_markup(run: &Run) -> String {
    let mut inner = String::new();
    for child in &run.children {
        match child {
            RunChild::Text(text) => escape(&text.text, &mut inner),
            RunChild::Tab(_) | RunChild::PTab(_) => inner.push(' '),
            RunChild::Break(_) | RunChild::CarriageReturn(_) => inner.push_str("<br>"),
            _ => {}
        }
    }
    if inner.trim().is_empty() {
        return inner;
    }
    let property = &run.run_property;
    let mut markup = String::new();
    let tags = [
        (property.bold.as_ref() == Some(&Bold::new()), "b"),
        (property.italic.as_ref() == Some(&Italic::new()), "i"),
        (underlined(property), "u"),
        (
            property.strike.as_ref().is_some_and(|strike| strike.val),
            "s",
        ),
    ];
    for (applies, tag) in tags {
        if applies {
            let _ = write!(markup, "<{tag}>");
        }
    }
    markup.push_str(&inner);
    for (applies, tag) in tags.iter().rev() {
        if *applies {
            let _ = write!(markup, "</{tag}>");
        }
    }
    markup
}

fn underlined(property: &RunProperty) -> bool {
    property
        .underline
        .as_ref()
        .is_some_and(|underline| *underline != Underline::new("none"))
}

fn is_ordered(numberings: &Numberings, id: usize, level: usize) -> bool {
    numberings
        .numberings
        .iter()
        .find(|numbering| numbering.id == id)
        .and_then(|numbering| {
            numberings
                .abstract_nums
                .iter()
                .find(|abstract_num| abstract_num.id == numbering.abstract_num_id)
        })
        .and_then(|abstract_num| {
            abstract_num
                .levels
                .iter()
                .find(|entry| entry.level == level)
        })
        .is_some_and(|entry| entry.format.val != "bullet")
}

enum Block {
    Paragraph,
    Heading(u8),
    Quote,
}

fn named_block(style: &str) -> Block {
    let style = style.to_ascii_lowercase();
    if let Some(level) = style.strip_prefix("heading") {
        return Block::Heading(level.trim().parse().unwrap_or(6).clamp(1, 6));
    }
    match style.as_str() {
        "title" => Block::Heading(1),
        "subtitle" => Block::Heading(2),
        "quote" | "intensequote" => Block::Quote,
        _ => Block::Paragraph,
    }
}

fn escape(text: &str, out: &mut String) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(character),
        }
    }
}

#[cfg(test)]
mod tests;
