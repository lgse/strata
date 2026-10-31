// SPDX-License-Identifier: MIT

use std::{collections::HashMap, ffi::OsStr, fmt::Write as _, fs::File, io::Read, path::Path};

use quick_xml::{
    NsReader,
    escape::resolve_predefined_entity,
    events::{BytesStart, Event},
    name::ResolveResult,
};
use zip::ZipArchive;

use super::{
    document::{DOCUMENT_EVENT_LIMIT, DOCUMENT_INPUT_LIMIT, has_web_scheme},
    docx::RichTextData,
    table::TABLE_COLUMN_LIMIT,
};

pub(crate) const ODT_BYTE_LIMIT: u64 = 20 * 1024 * 1024;
const XML_LIMIT: u64 = 64 * 1024 * 1024;
const HTML_LIMIT: usize = DOCUMENT_INPUT_LIMIT - 4096;
const EVENT_LIMIT: usize = DOCUMENT_EVENT_LIMIT - 512;
const LIST_DEPTH_LIMIT: usize = 8;
const LIST_LEVELS: usize = 10;
const STYLE_DEPTH_LIMIT: usize = 16;
const SPACE_LIMIT: usize = 1024;

const OFFICE: &[u8] = b"urn:oasis:names:tc:opendocument:xmlns:office:1.0";
const STYLE: &[u8] = b"urn:oasis:names:tc:opendocument:xmlns:style:1.0";
const TEXT: &[u8] = b"urn:oasis:names:tc:opendocument:xmlns:text:1.0";
const TABLE: &[u8] = b"urn:oasis:names:tc:opendocument:xmlns:table:1.0";
const DRAW: &[u8] = b"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0";
const FO: &[u8] = b"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0";
const XLINK: &[u8] = b"http://www.w3.org/1999/xlink";

pub(crate) fn is_document(content_type: &str, name: &OsStr) -> bool {
    matches!(
        content_type,
        "application/vnd.oasis.opendocument.text"
            | "application/vnd.oasis.opendocument.text-template"
    ) || Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("odt") || extension.eq_ignore_ascii_case("ott")
        })
}

pub(crate) fn read_document(path: &Path) -> Result<RichTextData, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > ODT_BYTE_LIMIT {
        return Err("Document is too large to preview safely".into());
    }
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut parser = Parser::default();
    if archive.index_for_name("styles.xml").is_some() {
        parser.read(&read_part(&mut archive, "styles.xml")?)?;
    }
    parser.read(&read_part(&mut archive, "content.xml")?)?;
    Ok(parser.finish())
}

fn read_part(archive: &mut ZipArchive<File>, name: &str) -> Result<Vec<u8>, String> {
    let part = archive.by_name(name).map_err(|e| e.to_string())?;
    let mut xml = Vec::new();
    part.take(XML_LIMIT + 1)
        .read_to_end(&mut xml)
        .map_err(|e| e.to_string())?;
    if xml.len() as u64 > XML_LIMIT {
        return Err("Document is too large to preview safely".into());
    }
    Ok(xml)
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
struct Format {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
    monospace: bool,
}

impl Format {
    fn tags(self) -> impl DoubleEndedIterator<Item = &'static str> {
        [
            (self.bold, "b"),
            (self.italic, "i"),
            (self.underline, "u"),
            (self.strike, "s"),
            (self.monospace, "code"),
        ]
        .into_iter()
        .filter_map(|(applies, tag)| applies.then_some(tag))
    }
}

#[derive(Clone, Copy, Default)]
struct Emphasis {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    monospace: Option<bool>,
}

impl Emphasis {
    fn or(self, fallback: Self) -> Self {
        Self {
            bold: self.bold.or(fallback.bold),
            italic: self.italic.or(fallback.italic),
            underline: self.underline.or(fallback.underline),
            strike: self.strike.or(fallback.strike),
            monospace: self.monospace.or(fallback.monospace),
        }
    }

    fn apply(self, format: Format) -> Format {
        Format {
            bold: self.bold.unwrap_or(format.bold),
            italic: self.italic.unwrap_or(format.italic),
            underline: self.underline.unwrap_or(format.underline),
            strike: self.strike.unwrap_or(format.strike),
            monospace: self.monospace.unwrap_or(format.monospace),
        }
    }
}

#[derive(Default)]
struct Style {
    parent: Option<String>,
    emphasis: Emphasis,
}

#[derive(Default)]
struct Styles {
    paragraph: HashMap<String, Style>,
    text: HashMap<String, Style>,
    lists: HashMap<String, [bool; LIST_LEVELS]>,
}

impl Styles {
    fn emphasis(styles: &HashMap<String, Style>, name: &str) -> Emphasis {
        lineage(styles, name).fold(Emphasis::default(), |emphasis, name| {
            let own = styles.get(name).map(|style| style.emphasis);
            emphasis.or(own.unwrap_or_default()).or(Emphasis {
                monospace: is_monospace(name).then_some(true),
                ..Emphasis::default()
            })
        })
    }

    fn block(&self, name: &str) -> Block {
        lineage(&self.paragraph, name)
            .find_map(named_block)
            .unwrap_or(Block::Paragraph)
    }

    fn ordered(&self, name: &str, level: usize) -> bool {
        self.lists
            .get(name)
            .and_then(|levels| levels.get(level))
            .is_some_and(|ordered| *ordered)
    }
}

fn lineage<'a>(
    styles: &'a HashMap<String, Style>,
    name: &'a str,
) -> impl Iterator<Item = &'a str> + 'a {
    std::iter::successors(Some(name), |name| styles.get(*name)?.parent.as_deref())
        .take(STYLE_DEPTH_LIMIT)
}

fn display_name(name: &str) -> String {
    name.replace("_20_", " ")
        .replace('_', " ")
        .to_ascii_lowercase()
}

fn is_monospace(name: &str) -> bool {
    matches!(display_name(name).as_str(), "source text" | "teletype")
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Block {
    Paragraph,
    Heading(u8),
    Quote,
    Code,
}

fn named_block(name: &str) -> Option<Block> {
    let name = display_name(name);
    if let Some(level) = name.strip_prefix("heading ") {
        return level
            .parse::<u8>()
            .ok()
            .map(|level| Block::Heading(level.clamp(1, 6)));
    }
    match name.as_str() {
        "title" => Some(Block::Heading(1)),
        "subtitle" => Some(Block::Heading(2)),
        "quotations" | "quote" | "intense quote" => Some(Block::Quote),
        "preformatted text" => Some(Block::Code),
        _ => None,
    }
}

struct Paragraph {
    block: Block,
    markup: String,
    events: usize,
    open: Format,
    link: bool,
    in_text: bool,
    pending_space: bool,
    visible: bool,
}

impl Paragraph {
    fn new(block: Block) -> Self {
        Self {
            block,
            markup: String::new(),
            events: 0,
            open: Format::default(),
            link: false,
            in_text: false,
            pending_space: false,
            visible: false,
        }
    }

    fn text(&mut self, text: &str, format: Format) {
        let format = if self.block == Block::Code {
            Format::default()
        } else {
            format
        };
        for character in text.chars() {
            if matches!(character, ' ' | '\t' | '\r' | '\n') {
                self.pending_space |= self.visible;
                continue;
            }
            if character.is_control() {
                continue;
            }
            if format != self.open {
                self.close_tags();
                self.flush_space();
                for tag in format.tags() {
                    self.tag(&format!("<{tag}>"));
                }
                self.open = format;
            } else {
                self.flush_space();
            }
            self.characters(|markup| escape_char(character, markup));
            self.visible = true;
        }
    }

    fn spaces(&mut self, count: usize) {
        if self.block == Block::Code {
            self.flush_space();
            self.characters(|markup| markup.extend(std::iter::repeat_n(' ', count)));
        } else {
            self.pending_space |= self.visible;
        }
    }

    fn tab(&mut self) {
        if self.block == Block::Code {
            self.flush_space();
            self.characters(|markup| markup.push('\t'));
        } else {
            self.pending_space |= self.visible;
        }
    }

    fn line_break(&mut self) {
        self.pending_space = false;
        if self.block == Block::Code {
            self.characters(|markup| markup.push('\n'));
        } else {
            self.tag("<br>");
        }
    }

    fn open_link(&mut self, href: &str) {
        self.close_tags();
        self.flush_space();
        let mut tag = String::from("<a href=\"");
        escape_attribute(href, &mut tag);
        tag.push_str("\">");
        self.tag(&tag);
        self.link = true;
    }

    fn close_link(&mut self) {
        self.close_tags();
        self.tag("</a>");
        self.link = false;
    }

    fn finish(mut self) -> Option<(String, usize)> {
        self.close_tags();
        if self.link {
            self.tag("</a>");
        }
        self.visible.then_some((self.markup, self.events))
    }

    fn close_tags(&mut self) {
        let open = std::mem::take(&mut self.open);
        for tag in open.tags().rev() {
            self.tag(&format!("</{tag}>"));
        }
    }

    fn flush_space(&mut self) {
        if std::mem::take(&mut self.pending_space) {
            self.characters(|markup| markup.push(' '));
        }
    }

    fn tag(&mut self, tag: &str) {
        self.markup.push_str(tag);
        self.events += 1;
        self.in_text = false;
    }

    fn characters(&mut self, write: impl FnOnce(&mut String)) {
        write(&mut self.markup);
        if !self.in_text {
            self.events += 1;
            self.in_text = true;
        }
    }
}

#[derive(Default)]
struct Cell {
    markup: String,
    events: usize,
}

#[derive(Default)]
struct Row {
    html: String,
    events: usize,
    cells: usize,
}

#[derive(Default)]
struct CodeBlock {
    held_blank_lines: usize,
}

#[derive(Default)]
struct Body {
    html: String,
    events: usize,
    truncated: bool,
    code_block: Option<CodeBlock>,
}

impl Body {
    fn fits(&mut self, bytes: usize, events: usize) -> bool {
        if self.truncated
            || self.html.len().saturating_add(bytes) > HTML_LIMIT
            || self.events.saturating_add(events) > EVENT_LIMIT
        {
            self.truncated = true;
            return false;
        }
        true
    }

    fn open(&mut self, markup: &str, events: usize) -> bool {
        self.close_code_block();
        if !self.fits(markup.len(), events) {
            return false;
        }
        self.html.push_str(markup);
        self.events += events;
        true
    }

    fn close(&mut self, markup: &str) {
        self.close_code_block();
        self.html.push_str(markup);
        self.events += 1;
    }

    fn close_code_block(&mut self) {
        if self.code_block.take().is_some() {
            self.html.push_str("</pre>");
            self.events += 1;
        }
    }

    fn block(&mut self, block: Block, markup: &str, events: usize) {
        match block {
            Block::Code => {
                if let Some(code_block) = &self.code_block {
                    let separator = "\n".repeat(code_block.held_blank_lines + 1);
                    if self.fits(separator.len() + markup.len(), events) {
                        self.html.push_str(&separator);
                        self.html.push_str(markup);
                        self.events += events;
                        self.code_block = Some(CodeBlock::default());
                    }
                } else if self.open(&format!("<pre>{markup}"), events + 1) {
                    self.code_block = Some(CodeBlock::default());
                }
            }
            Block::Heading(level) => {
                self.open(&format!("<h{level}>{markup}</h{level}>"), events + 2);
            }
            Block::Quote => {
                self.open(&format!("<blockquote>{markup}</blockquote>"), events + 2);
            }
            Block::Paragraph => {
                self.open(&format!("<p>{markup}</p>"), events + 2);
            }
        }
    }

    fn hold_blank_code_line(&mut self) {
        if let Some(code_block) = &mut self.code_block {
            code_block.held_blank_lines += 1;
        }
    }
}

enum Open {
    Other,
    Body,
    Style,
    ListStyle,
    Paragraph(Format),
    Span(Format),
    Link,
    List(Option<&'static str>),
    Item(bool),
    Table(bool),
    HeaderRows,
    Row,
    Cell,
}

struct PendingStyle {
    paragraph: bool,
    name: String,
    style: Style,
}

#[derive(Default)]
struct Parser {
    styles: Styles,
    style: Option<PendingStyle>,
    list_style: Option<(String, [bool; LIST_LEVELS])>,
    body: Body,
    stack: Vec<Open>,
    skipped: usize,
    in_text: bool,
    format: Format,
    paragraph: Option<Paragraph>,
    lists: Vec<Option<String>>,
    tables: usize,
    header: bool,
    row: Option<Row>,
    cell: Option<Cell>,
}

impl Parser {
    fn read(&mut self, xml: &[u8]) -> Result<(), String> {
        let mut reader = NsReader::from_reader(xml);
        reader.config_mut().expand_empty_elements = true;
        while !self.body.truncated {
            match reader.read_event().map_err(|e| e.to_string())? {
                Event::Start(element) => self.start(&reader, &element)?,
                Event::End(_) => self.end(),
                Event::Text(text) => self.text(&text.xml10_content().map_err(|e| e.to_string())?),
                Event::CData(text) => self.text(&text.decode().map_err(|e| e.to_string())?),
                Event::GeneralRef(reference) => {
                    if let Some(character) =
                        reference.resolve_char_ref().map_err(|e| e.to_string())?
                    {
                        self.text(character.encode_utf8(&mut [0; 4]));
                    } else if let Some(text) =
                        resolve_predefined_entity(&reference.decode().map_err(|e| e.to_string())?)
                    {
                        self.text(text);
                    }
                }
                Event::DocType(_) => {
                    return Err("Document XML declarations with DTDs are unsupported".into());
                }
                Event::Eof if self.stack.is_empty() && self.skipped == 0 => break,
                Event::Eof => return Err("Unclosed document XML element".into()),
                _ => {}
            }
            self.truncate_when_pending_blocks_overflow();
        }
        Ok(())
    }

    fn start(&mut self, reader: &NsReader<&[u8]>, element: &BytesStart<'_>) -> Result<(), String> {
        if self.skipped > 0 {
            self.skipped += 1;
            return Ok(());
        }
        let (namespace, local) = reader.resolver().resolve_element(element.name());
        let namespace = match namespace {
            ResolveResult::Bound(namespace) => [OFFICE, STYLE, TEXT, TABLE, DRAW]
                .into_iter()
                .find(|known| *known == namespace.as_ref())
                .unwrap_or_default(),
            _ => &[],
        };
        let local = local.as_ref();
        if is_skipped(namespace, local) {
            self.skipped = 1;
            return Ok(());
        }
        let value = |namespace: &[u8], local: &[u8]| attribute(reader, element, namespace, local);
        let open = match (namespace, local) {
            (OFFICE, b"text") => {
                self.in_text = true;
                Open::Body
            }
            (STYLE, b"style") => {
                let family = value(STYLE, b"family")?;
                match (family.as_deref(), value(STYLE, b"name")?) {
                    (Some(family @ ("paragraph" | "text")), Some(name)) => {
                        self.style = Some(PendingStyle {
                            paragraph: family == "paragraph",
                            name,
                            style: Style {
                                parent: value(STYLE, b"parent-style-name")?,
                                emphasis: Emphasis::default(),
                            },
                        });
                        Open::Style
                    }
                    _ => Open::Other,
                }
            }
            (STYLE, b"text-properties") => {
                if let Some(pending) = &mut self.style {
                    pending.style.emphasis = text_properties(reader, element)?;
                }
                Open::Other
            }
            (TEXT, b"list-style") => match value(STYLE, b"name")? {
                Some(name) => {
                    self.list_style = Some((name, [false; LIST_LEVELS]));
                    Open::ListStyle
                }
                None => Open::Other,
            },
            (TEXT, b"list-level-style-number") => {
                let level = value(TEXT, b"level")?
                    .and_then(|level| level.parse::<usize>().ok())
                    .filter(|level| (1..=LIST_LEVELS).contains(level));
                if let (Some((_, levels)), Some(level)) = (&mut self.list_style, level) {
                    levels[level - 1] =
                        value(STYLE, b"num-format")?.is_some_and(|format| !format.is_empty());
                }
                Open::Other
            }
            _ if !self.in_text => Open::Other,
            (TEXT, b"p" | b"h") => {
                if let Some(paragraph) = &mut self.paragraph {
                    paragraph.pending_space |= paragraph.visible;
                    Open::Other
                } else {
                    let style = value(TEXT, b"style-name")?;
                    let block = if self.cell.is_some() {
                        Block::Paragraph
                    } else if local == b"h" {
                        let level = value(TEXT, b"outline-level")?
                            .and_then(|level| level.parse::<u8>().ok())
                            .unwrap_or(1);
                        Block::Heading(level.clamp(1, 6))
                    } else {
                        style
                            .as_deref()
                            .map_or(Block::Paragraph, |style| self.styles.block(style))
                    };
                    let previous = std::mem::take(&mut self.format);
                    if matches!(block, Block::Paragraph | Block::Quote)
                        && let Some(style) = &style
                    {
                        self.format = Styles::emphasis(&self.styles.paragraph, style)
                            .apply(Format::default());
                    }
                    self.paragraph = Some(Paragraph::new(block));
                    Open::Paragraph(previous)
                }
            }
            (TEXT, b"span") => {
                let previous = self.format;
                if let Some(style) = value(TEXT, b"style-name")? {
                    self.format = Styles::emphasis(&self.styles.text, &style).apply(self.format);
                }
                Open::Span(previous)
            }
            (TEXT, b"a") => match (&mut self.paragraph, value(XLINK, b"href")?) {
                (Some(paragraph), Some(href))
                    if paragraph.block != Block::Code
                        && !paragraph.link
                        && has_web_scheme(&href) =>
                {
                    paragraph.open_link(&href);
                    Open::Link
                }
                _ => Open::Other,
            },
            (TEXT, b"s") => {
                let count = value(TEXT, b"c")?
                    .and_then(|count| count.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, SPACE_LIMIT);
                if let Some(paragraph) = &mut self.paragraph {
                    paragraph.spaces(count);
                }
                Open::Other
            }
            (TEXT, b"tab") => {
                if let Some(paragraph) = &mut self.paragraph {
                    paragraph.tab();
                }
                Open::Other
            }
            (TEXT, b"line-break") => {
                if let Some(paragraph) = &mut self.paragraph {
                    paragraph.line_break();
                }
                Open::Other
            }
            (TEXT, b"list") if self.cell.is_none() && self.paragraph.is_none() => {
                let style =
                    value(TEXT, b"style-name")?.or_else(|| self.lists.last().cloned().flatten());
                let level = self.lists.len();
                let closer = if level < LIST_DEPTH_LIMIT {
                    let ordered = style
                        .as_deref()
                        .is_some_and(|style| self.styles.ordered(style, level));
                    let (open, close) = if ordered {
                        ("<ol>", "</ol>")
                    } else {
                        ("<ul>", "</ul>")
                    };
                    self.body.open(open, 1).then_some(close)
                } else {
                    None
                };
                self.lists.push(style);
                Open::List(closer)
            }
            (TEXT, b"list-item" | b"list-header")
                if self.cell.is_none() && self.paragraph.is_none() =>
            {
                let listed = matches!(self.stack.last(), Some(Open::List(Some(_))));
                Open::Item(listed && self.body.open("<li>", 1))
            }
            (TABLE, b"table") if self.paragraph.is_none() => {
                self.tables += 1;
                Open::Table(self.tables == 1 && self.body.open("<table>", 1))
            }
            (TABLE, b"table-header-rows") if self.tables == 1 => {
                self.header = true;
                Open::HeaderRows
            }
            (TABLE, b"table-row") if self.tables == 1 && self.row.is_none() => {
                self.row = Some(Row::default());
                Open::Row
            }
            (TABLE, b"table-cell" | b"covered-table-cell")
                if self.tables == 1 && self.row.is_some() && self.cell.is_none() =>
            {
                self.cell = Some(Cell::default());
                Open::Cell
            }
            _ => Open::Other,
        };
        self.stack.push(open);
        Ok(())
    }

    fn end(&mut self) {
        if self.skipped > 0 {
            self.skipped -= 1;
            return;
        }
        let Some(open) = self.stack.pop() else {
            return;
        };
        match open {
            Open::Other => {}
            Open::Body => self.in_text = false,
            Open::Style => {
                if let Some(pending) = self.style.take() {
                    let styles = if pending.paragraph {
                        &mut self.styles.paragraph
                    } else {
                        &mut self.styles.text
                    };
                    styles.insert(pending.name, pending.style);
                }
            }
            Open::ListStyle => {
                if let Some((name, levels)) = self.list_style.take() {
                    self.styles.lists.insert(name, levels);
                }
            }
            Open::Paragraph(previous) => {
                self.format = previous;
                let Some(paragraph) = self.paragraph.take() else {
                    return;
                };
                let block = paragraph.block;
                match (paragraph.finish(), &mut self.cell) {
                    (Some((markup, events)), Some(cell)) => {
                        if !cell.markup.is_empty() {
                            cell.markup.push(' ');
                            cell.events += 1;
                        }
                        cell.markup.push_str(&markup);
                        cell.events += events;
                    }
                    (Some((markup, events)), None) => self.body.block(block, &markup, events),
                    (None, _) if block == Block::Code => self.body.hold_blank_code_line(),
                    (None, _) => {}
                }
            }
            Open::Span(previous) => self.format = previous,
            Open::Link => {
                if let Some(paragraph) = &mut self.paragraph {
                    paragraph.close_link();
                }
            }
            Open::List(closer) => {
                self.lists.pop();
                if let Some(closer) = closer {
                    self.body.close(closer);
                }
            }
            Open::Item(opened) => {
                if opened {
                    self.body.close("</li>");
                }
            }
            Open::Table(opened) => {
                self.tables -= 1;
                if opened {
                    self.body.close("</table>");
                }
            }
            Open::HeaderRows => self.header = false,
            Open::Row => {
                if let Some(row) = self.row.take()
                    && row.cells > 0
                {
                    self.body
                        .open(&format!("<tr>{}</tr>", row.html), row.events + 2);
                }
            }
            Open::Cell => {
                let cell = self.cell.take().unwrap_or_default();
                let Some(row) = &mut self.row else {
                    return;
                };
                if row.cells == TABLE_COLUMN_LIMIT {
                    self.body.truncated = true;
                    return;
                }
                let tag = if self.header { "th" } else { "td" };
                let _ = write!(row.html, "<{tag}>{}</{tag}>", cell.markup);
                row.events += cell.events + 2;
                row.cells += 1;
            }
        }
    }

    fn text(&mut self, text: &str) {
        if self.skipped == 0
            && let Some(paragraph) = &mut self.paragraph
        {
            paragraph.text(text, self.format);
        }
    }

    fn truncate_when_pending_blocks_overflow(&mut self) {
        let (bytes, events) = [
            self.paragraph
                .as_ref()
                .map(|paragraph| (paragraph.markup.len(), paragraph.events)),
            self.cell
                .as_ref()
                .map(|cell| (cell.markup.len(), cell.events)),
            self.row.as_ref().map(|row| (row.html.len(), row.events)),
        ]
        .into_iter()
        .flatten()
        .fold((0, 0), |(bytes, events), (more_bytes, more_events)| {
            (bytes + more_bytes, events + more_events)
        });
        if self.body.html.len() + bytes > HTML_LIMIT || self.body.events + events > EVENT_LIMIT {
            self.body.truncated = true;
        }
    }

    fn finish(mut self) -> RichTextData {
        self.body.close_code_block();
        while let Some(open) = self.stack.pop() {
            match open {
                Open::List(Some(closer)) => self.body.close(closer),
                Open::Item(true) => self.body.close("</li>"),
                Open::Table(true) => self.body.close("</table>"),
                _ => {}
            }
        }
        RichTextData {
            html: self.body.html,
            truncated: self.body.truncated,
        }
    }
}

fn is_skipped(namespace: &[u8], local: &[u8]) -> bool {
    match namespace {
        DRAW => true,
        OFFICE => matches!(local, b"annotation" | b"forms"),
        TEXT => {
            matches!(
                local,
                b"tracked-changes"
                    | b"note"
                    | b"number"
                    | b"ruby-text"
                    | b"sequence-decls"
                    | b"variable-decls"
                    | b"user-field-decls"
                    | b"dde-connection-decls"
            ) || local.ends_with(b"-source")
        }
        TABLE => local == b"content-validations",
        _ => false,
    }
}

fn attribute(
    reader: &NsReader<&[u8]>,
    element: &BytesStart<'_>,
    namespace: &[u8],
    local: &[u8],
) -> Result<Option<String>, String> {
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|e| e.to_string())?;
        let (resolved, name) = reader.resolver().resolve_attribute(attribute.key);
        if name.as_ref() == local
            && matches!(resolved, ResolveResult::Bound(bound) if bound.as_ref() == namespace)
        {
            let value = attribute
                .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
                .map_err(|e| e.to_string())?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

fn text_properties(reader: &NsReader<&[u8]>, element: &BytesStart<'_>) -> Result<Emphasis, String> {
    let property = |namespace: &[u8], local: &[u8]| attribute(reader, element, namespace, local);
    Ok(Emphasis {
        bold: property(FO, b"font-weight")?.map(|weight| {
            weight == "bold" || weight.parse::<u16>().is_ok_and(|weight| weight >= 600)
        }),
        italic: property(FO, b"font-style")?.map(|style| style == "italic" || style == "oblique"),
        underline: property(STYLE, b"text-underline-style")?.map(|style| style != "none"),
        strike: property(STYLE, b"text-line-through-style")?.map(|style| style != "none"),
        monospace: None,
    })
}

fn escape_char(character: char, out: &mut String) {
    match character {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        _ => out.push(character),
    }
}

fn escape_attribute(value: &str, out: &mut String) {
    for character in value.chars() {
        if character == '"' {
            out.push_str("&quot;");
        } else {
            escape_char(character, out);
        }
    }
}

#[cfg(test)]
mod tests;
