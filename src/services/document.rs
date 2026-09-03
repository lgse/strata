// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    ffi::OsStr,
    ops::Range,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use html5gum::{DefaultEmitter, Token, Tokenizer};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::sandbox::Cancellation;

pub const DOCUMENT_INPUT_LIMIT: usize = 1024 * 1024;
pub const DOCUMENT_EVENT_LIMIT: usize = 20_000;
pub const DOCUMENT_DEPTH_LIMIT: usize = 32;
pub const DOCUMENT_TABLE_CELL_LIMIT: usize = 512;
pub const DOCUMENT_MARKUP_LIMIT: usize = 4 * 1024 * 1024;
pub const DOCUMENT_TIME_LIMIT: Duration = Duration::from_millis(500);
pub const DOCUMENT_UNIT_TARGET: usize = 32 * 1024;
pub const DOCUMENT_UNIT_LINE_TARGET: usize = 2 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentKind {
    Markdown,
    Html,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DocumentBlock {
    Heading {
        level: u8,
        markup: String,
    },
    Paragraph(String),
    ListItem {
        marker: String,
        depth: usize,
        markup: String,
    },
    ListChild {
        depth: usize,
        kind: DocumentListChildKind,
        markup: String,
    },
    ListRule {
        depth: usize,
    },
    ListTableRow {
        depth: usize,
        cells: Vec<DocumentTableCell>,
    },
    Quote(String),
    Code {
        markup: String,
        language: Option<&'static str>,
    },
    Rule,
    ContainerBoundary,
    TableRow {
        cells: Vec<DocumentTableCell>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentTableCell {
    pub header: bool,
    pub markup: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentListChildKind {
    Paragraph,
    Heading(u8),
    Quote,
    Code(Option<&'static str>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Document {
    pub blocks: Vec<DocumentBlock>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentLayout {
    pub units: Vec<DocumentUnit>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentUnit {
    pub kind: DocumentUnitKind,
    pub text: String,
    pub copy_text: String,
    pub spans: Vec<DocumentSpan>,
    pub wrap: bool,
    pub first: bool,
    pub last: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DocumentUnitKind {
    Heading(u8),
    Paragraph,
    ListItem {
        depth: usize,
    },
    ListChild {
        depth: usize,
        kind: DocumentListChildKind,
    },
    Quote,
    Code {
        list_depth: Option<usize>,
        language: Option<&'static str>,
    },
    Rule {
        list_depth: Option<usize>,
    },
    Table {
        list_depth: Option<usize>,
        rows: Vec<Vec<DocumentTableCellLayout>>,
    },
    Gap,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentTableCellLayout {
    pub header: bool,
    pub text: String,
    pub spans: Vec<DocumentSpan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentSpan {
    pub range: Range<usize>,
    pub style: DocumentSpanStyle,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DocumentSpanStyle {
    Accent,
    Bold,
    Italic,
    Strikethrough,
    Monospace,
    Underline,
    Link(Arc<str>),
}

struct StyledText {
    text: String,
    spans: Vec<DocumentSpan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedDocument {
    pub document: Document,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy)]
struct ParseLimits {
    events: usize,
    depth: usize,
    table_cells: usize,
    markup: usize,
    time: Duration,
}

impl Default for ParseLimits {
    fn default() -> Self {
        Self {
            events: DOCUMENT_EVENT_LIMIT,
            depth: DOCUMENT_DEPTH_LIMIT,
            table_cells: DOCUMENT_TABLE_CELL_LIMIT,
            markup: DOCUMENT_MARKUP_LIMIT,
            time: DOCUMENT_TIME_LIMIT,
        }
    }
}

#[derive(Debug)]
enum ActiveBlock {
    Heading {
        level: u8,
        markup: String,
    },
    Paragraph(String),
    ListItem {
        marker: String,
        depth: usize,
        markup: String,
    },
    ListChild {
        depth: usize,
        kind: DocumentListChildKind,
        markup: String,
    },
    Quote(String),
    Code {
        markup: String,
        language: Option<&'static str>,
    },
}

impl ActiveBlock {
    fn markup(&self) -> &str {
        match self {
            Self::Heading { markup, .. }
            | Self::Paragraph(markup)
            | Self::ListItem { markup, .. }
            | Self::ListChild { markup, .. }
            | Self::Quote(markup)
            | Self::Code { markup, .. } => markup,
        }
    }

    fn markup_mut(&mut self) -> &mut String {
        match self {
            Self::Heading { markup, .. }
            | Self::Paragraph(markup)
            | Self::ListItem { markup, .. }
            | Self::ListChild { markup, .. }
            | Self::Quote(markup)
            | Self::Code { markup, .. } => markup,
        }
    }
}

struct ParseBudget<'a> {
    cancellation: &'a Cancellation,
    limits: ParseLimits,
    started: Instant,
    events: usize,
    depth: usize,
}

impl<'a> ParseBudget<'a> {
    fn new(cancellation: &'a Cancellation, limits: ParseLimits) -> Self {
        Self {
            cancellation,
            limits,
            started: Instant::now(),
            events: 0,
            depth: 0,
        }
    }

    fn check(&self) -> Result<(), String> {
        if self.cancellation.is_cancelled() {
            return Err("Rendered preview was cancelled".to_owned());
        }
        if self.started.elapsed() >= self.limits.time {
            return Err("Rendered preview exceeded the 500 ms parsing limit".to_owned());
        }
        Ok(())
    }

    fn event(&mut self) -> Result<(), String> {
        self.check()?;
        self.events = self.events.saturating_add(1);
        if self.events > self.limits.events {
            return Err("Rendered preview exceeded the 20,000 parser-event limit".to_owned());
        }
        Ok(())
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth = self.depth.saturating_add(1);
        if self.depth > self.limits.depth {
            return Err("Rendered preview exceeded the nesting-depth limit of 32".to_owned());
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn nesting(&mut self, depth: usize) -> Result<(), String> {
        self.depth = depth;
        if self.depth > self.limits.depth {
            return Err("Rendered preview exceeded the nesting-depth limit of 32".to_owned());
        }
        Ok(())
    }
}

pub fn document_kind(content_type: &str, name: &OsStr, is_native: bool) -> Option<DocumentKind> {
    if !is_native {
        return None;
    }
    match content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "text/markdown" | "text/x-markdown" => return Some(DocumentKind::Markdown),
        "text/html" | "application/xhtml+xml" => return Some(DocumentKind::Html),
        _ => {}
    }
    match Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("md" | "markdown" | "mdown" | "mkd" | "mkdn" | "mdwn") => Some(DocumentKind::Markdown),
        Some("html" | "htm" | "xhtml") => Some(DocumentKind::Html),
        _ => None,
    }
}

pub fn parse_document(
    kind: DocumentKind,
    source: &str,
    cancellation: &Cancellation,
) -> Result<ParsedDocument, String> {
    if source.len() > DOCUMENT_INPUT_LIMIT {
        return Err("Rendered preview is limited to documents of 1 MB or less".to_owned());
    }
    parse_document_with_limits(kind, source, cancellation, ParseLimits::default())
}

fn parse_document_with_limits(
    kind: DocumentKind,
    source: &str,
    cancellation: &Cancellation,
    limits: ParseLimits,
) -> Result<ParsedDocument, String> {
    if cancellation.is_cancelled() {
        return Err("Rendered preview was cancelled".to_owned());
    }
    if source.contains('\0') {
        return Err(
            "Rendered preview is unavailable because the document contains NUL bytes".to_owned(),
        );
    }
    let parsed = match kind {
        DocumentKind::Markdown => parse_markdown_bounded(source, cancellation, limits, true),
        DocumentKind::Html => parse_html_bounded(source, cancellation, limits),
    }?;
    validate_document(parsed, limits)
}

/// Parses release notes through the same Markdown model without changing their legacy limits.
pub fn parse_markdown(markdown: &str) -> Document {
    let cancellation = Cancellation::default();
    let limits = ParseLimits {
        events: usize::MAX,
        depth: usize::MAX,
        table_cells: usize::MAX,
        markup: usize::MAX,
        time: Duration::MAX,
    };
    parse_markdown_bounded(markdown, &cancellation, limits, false)
        .map(|parsed| parsed.document)
        .unwrap_or(Document { blocks: Vec::new() })
}

fn parse_markdown_bounded(
    markdown: &str,
    cancellation: &Cancellation,
    limits: ParseLimits,
    document_features: bool,
) -> Result<ParsedDocument, String> {
    let mut budget = ParseBudget::new(cancellation, limits);
    budget.check()?;
    let mut blocks = Vec::new();
    let mut active = None;
    let mut links = Vec::new();
    let mut lists = Vec::<Option<u64>>::new();
    let mut list_items = Vec::<usize>::new();
    let mut quote_depth = 0usize;
    let mut table_header = false;
    let mut table_row: Option<Vec<DocumentTableCell>> = None;
    let mut table_cell: Option<String> = None;
    let mut table_depth = None;
    let mut raw_html = false;
    let mut options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    if document_features {
        options |= Options::ENABLE_TABLES;
    }

    for event in Parser::new_ext(markdown, options) {
        budget.event()?;
        if matches!(event, Event::Start(_)) {
            budget.enter()?;
        }
        match &event {
            Event::Start(Tag::Heading { level, .. }) => {
                finish_parent_list_item(&mut active, &mut blocks);
                let level = heading_level(*level);
                active = Some(if let Some(depth) = list_items.last() {
                    ActiveBlock::ListChild {
                        depth: *depth,
                        kind: DocumentListChildKind::Heading(level),
                        markup: String::new(),
                    }
                } else {
                    ActiveBlock::Heading {
                        level,
                        markup: String::new(),
                    }
                });
            }
            Event::End(TagEnd::Heading(_)) => finish_block(&mut active, &mut blocks),
            Event::Start(Tag::Paragraph) => {
                let starts_another_list_paragraph = active.as_ref().is_some_and(|active| {
                    matches!(
                        active,
                        ActiveBlock::ListItem { .. }
                            | ActiveBlock::ListChild {
                                kind: DocumentListChildKind::Paragraph,
                                ..
                            }
                    ) && has_visible_markup(active.markup())
                });
                if starts_another_list_paragraph {
                    finish_block(&mut active, &mut blocks);
                }
                if active.is_none() {
                    active = Some(if let Some(depth) = list_items.last() {
                        ActiveBlock::ListChild {
                            depth: *depth,
                            kind: if quote_depth > 0 {
                                DocumentListChildKind::Quote
                            } else {
                                DocumentListChildKind::Paragraph
                            },
                            markup: String::new(),
                        }
                    } else if quote_depth > 0 {
                        ActiveBlock::Quote(String::new())
                    } else {
                        ActiveBlock::Paragraph(String::new())
                    });
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if matches!(
                    active,
                    Some(
                        ActiveBlock::Paragraph(_)
                            | ActiveBlock::Quote(_)
                            | ActiveBlock::ListChild {
                                kind: DocumentListChildKind::Paragraph
                                    | DocumentListChildKind::Quote,
                                ..
                            }
                    )
                ) {
                    finish_block(&mut active, &mut blocks);
                }
            }
            Event::Start(Tag::BlockQuote(_)) => {
                if document_features {
                    finish_parent_list_item(&mut active, &mut blocks);
                    quote_depth = quote_depth.saturating_add(1);
                    active = Some(if let Some(depth) = list_items.last() {
                        ActiveBlock::ListChild {
                            depth: *depth,
                            kind: DocumentListChildKind::Quote,
                            markup: String::new(),
                        }
                    } else {
                        ActiveBlock::Quote(String::new())
                    });
                }
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                if document_features {
                    finish_block(&mut active, &mut blocks);
                    quote_depth = quote_depth.saturating_sub(1);
                }
            }
            Event::Start(Tag::List(start)) => {
                finish_parent_list_item(&mut active, &mut blocks);
                if lists.is_empty() && blocks.last().is_some_and(is_list_block) {
                    blocks.push(DocumentBlock::ContainerBoundary);
                }
                lists.push(*start);
            }
            Event::End(TagEnd::List(_)) => {
                if matches!(
                    active,
                    Some(ActiveBlock::ListItem { .. } | ActiveBlock::ListChild { .. })
                ) {
                    finish_block(&mut active, &mut blocks);
                }
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                finish_block(&mut active, &mut blocks);
                let depth = lists.len().saturating_sub(1);
                list_items.push(depth);
                active = Some(ActiveBlock::ListItem {
                    marker: next_list_marker(&mut lists),
                    depth,
                    markup: String::new(),
                });
            }
            Event::End(TagEnd::Item) => {
                if matches!(
                    active,
                    Some(ActiveBlock::ListItem { .. } | ActiveBlock::ListChild { .. })
                ) {
                    finish_block(&mut active, &mut blocks);
                }
                list_items.pop();
            }
            Event::Start(Tag::Table(_)) => {
                finish_parent_list_item(&mut active, &mut blocks);
                table_depth = list_items.last().copied();
                let follows_table = match (table_depth, blocks.last()) {
                    (None, Some(DocumentBlock::TableRow { .. })) => true,
                    (
                        Some(depth),
                        Some(DocumentBlock::ListTableRow {
                            depth: previous_depth,
                            ..
                        }),
                    ) => depth == *previous_depth,
                    _ => false,
                };
                if follows_table {
                    blocks.push(DocumentBlock::ContainerBoundary);
                }
            }
            Event::End(TagEnd::Table) => {
                finish_block(&mut active, &mut blocks);
                table_depth = None;
            }
            Event::Start(Tag::TableHead) => {
                table_header = true;
                table_row = Some(Vec::new());
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(cells) = table_row.take() {
                    push_table_row(&mut blocks, table_depth, cells);
                }
                table_header = false;
            }
            Event::Start(Tag::TableRow) => table_row = Some(Vec::new()),
            Event::End(TagEnd::TableRow) => {
                if let Some(cells) = table_row.take() {
                    push_table_row(&mut blocks, table_depth, cells);
                }
            }
            Event::Start(Tag::TableCell) => table_cell = Some(String::new()),
            Event::End(TagEnd::TableCell) => {
                if let Some(cell) = table_cell.take() {
                    table_row
                        .get_or_insert_with(Vec::new)
                        .push(DocumentTableCell {
                            header: table_header,
                            markup: cell,
                        });
                }
            }
            Event::Start(Tag::Emphasis) => append_markup(&mut active, &mut table_cell, "<i>"),
            Event::End(TagEnd::Emphasis) => append_markup(&mut active, &mut table_cell, "</i>"),
            Event::Start(Tag::Strong) => append_markup(&mut active, &mut table_cell, "<b>"),
            Event::End(TagEnd::Strong) => append_markup(&mut active, &mut table_cell, "</b>"),
            Event::Start(Tag::Strikethrough) => {
                append_markup(&mut active, &mut table_cell, "<s>");
            }
            Event::End(TagEnd::Strikethrough) => {
                append_markup(&mut active, &mut table_cell, "</s>");
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                let destination = dest_url.as_ref();
                let external = has_web_scheme(destination);
                links.push(external);
                if external {
                    append_markup(&mut active, &mut table_cell, "<a href=\"");
                    append_escaped(&mut active, &mut table_cell, destination);
                    append_markup(&mut active, &mut table_cell, "\">");
                } else {
                    append_markup(&mut active, &mut table_cell, "<u>");
                }
            }
            Event::End(TagEnd::Link) => append_markup(
                &mut active,
                &mut table_cell,
                if links.pop().unwrap_or(false) {
                    "</a>"
                } else {
                    "</u>"
                },
            ),
            Event::Start(Tag::Image { .. }) => {
                append_markup(&mut active, &mut table_cell, "[Image: ");
            }
            Event::End(TagEnd::Image) => append_markup(&mut active, &mut table_cell, "]"),
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = markdown_code_language(kind);
                if matches!(
                    active,
                    Some(
                        ActiveBlock::Quote(_)
                            | ActiveBlock::ListChild {
                                kind: DocumentListChildKind::Quote,
                                ..
                            }
                    )
                ) {
                    append_markup(&mut active, &mut table_cell, "<tt>");
                } else {
                    finish_parent_list_item(&mut active, &mut blocks);
                    active = Some(if let Some(depth) = list_items.last() {
                        ActiveBlock::ListChild {
                            depth: *depth,
                            kind: DocumentListChildKind::Code(language),
                            markup: String::new(),
                        }
                    } else {
                        ActiveBlock::Code {
                            markup: String::new(),
                            language,
                        }
                    });
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if matches!(
                    active,
                    Some(
                        ActiveBlock::Code { .. }
                            | ActiveBlock::ListChild {
                                kind: DocumentListChildKind::Code(_),
                                ..
                            }
                    )
                ) {
                    finish_block(&mut active, &mut blocks);
                } else {
                    append_markup(&mut active, &mut table_cell, "</tt>");
                }
            }
            Event::Code(text) => {
                append_markup(&mut active, &mut table_cell, "<tt>");
                append_escaped(&mut active, &mut table_cell, text);
                append_markup(&mut active, &mut table_cell, "</tt>");
            }
            Event::Text(text) => append_escaped(&mut active, &mut table_cell, text),
            Event::SoftBreak | Event::HardBreak => {
                append_markup(&mut active, &mut table_cell, "\n");
            }
            Event::Rule => {
                finish_parent_list_item(&mut active, &mut blocks);
                if let Some(depth) = list_items.last().copied() {
                    blocks.push(DocumentBlock::ListRule { depth });
                } else {
                    blocks.push(DocumentBlock::Rule);
                }
            }
            Event::Html(text) | Event::InlineHtml(text) => {
                raw_html = true;
                append_escaped(&mut active, &mut table_cell, text);
            }
            Event::TaskListMarker(checked) => append_markup(
                &mut active,
                &mut table_cell,
                if *checked { "☑ " } else { "☐ " },
            ),
            Event::Start(_)
            | Event::End(_)
            | Event::FootnoteReference(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_) => {}
        }
        if matches!(event, Event::End(_)) {
            budget.leave();
        }
    }
    finish_block(&mut active, &mut blocks);
    Ok(ParsedDocument {
        document: Document { blocks },
        warnings: raw_html
            .then(|| "Raw HTML is shown as inert text in Markdown previews.".to_owned())
            .into_iter()
            .collect(),
    })
}

#[derive(Clone)]
enum HtmlLink {
    External(String),
    Underline,
    Inert,
}

struct HtmlState {
    blocks: Vec<DocumentBlock>,
    active: Option<ActiveBlock>,
    lists: Vec<Option<u64>>,
    list_items: Vec<usize>,
    links: Vec<HtmlLink>,
    stack: Vec<String>,
    skipped_depth: usize,
    quote_depth: usize,
    table_row: Option<Vec<DocumentTableCell>>,
    table_cell: Option<String>,
    table_cell_header: bool,
    table_depth: Option<usize>,
    markup_remaining: usize,
    markup_exceeded: bool,
    preformatted: bool,
    pending_space: bool,
    at_line_start: bool,
    warned: bool,
    malformed: bool,
}

impl HtmlState {
    fn new(markup_limit: usize) -> Self {
        Self {
            blocks: Vec::new(),
            active: None,
            lists: Vec::new(),
            list_items: Vec::new(),
            links: Vec::new(),
            stack: Vec::new(),
            skipped_depth: 0,
            quote_depth: 0,
            table_row: None,
            table_cell: None,
            table_cell_header: false,
            table_depth: None,
            markup_remaining: markup_limit,
            markup_exceeded: false,
            preformatted: false,
            pending_space: false,
            at_line_start: true,
            warned: false,
            malformed: false,
        }
    }

    fn start(
        &mut self,
        name: &str,
        href: Option<&str>,
        language: Option<&'static str>,
        self_closing: bool,
    ) {
        let void = is_void_html_tag(name) || self_closing;
        if self.skipped_depth > 0 {
            if !void {
                self.stack.push(name.to_owned());
                self.skipped_depth = self.skipped_depth.saturating_add(1);
            }
            return;
        }

        self.close_implied_before_start(name);
        if is_omitted_html_tag(name) {
            self.warned = true;
            if !void {
                self.stack.push(name.to_owned());
                self.skipped_depth = 1;
            }
            return;
        }

        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.finish_list_parent();
                let level = name[1..].parse().unwrap_or(6);
                self.start_active(if let Some(depth) = self.list_items.last() {
                    ActiveBlock::ListChild {
                        depth: *depth,
                        kind: DocumentListChildKind::Heading(level),
                        markup: String::new(),
                    }
                } else {
                    ActiveBlock::Heading {
                        level,
                        markup: String::new(),
                    }
                });
            }
            "p" => {
                if self.list_items.is_empty() {
                    self.finish_active();
                    self.start_active(if self.quote_depth > 0 {
                        ActiveBlock::Quote(String::new())
                    } else {
                        ActiveBlock::Paragraph(String::new())
                    });
                } else {
                    let kind = if self.quote_depth > 0 {
                        DocumentListChildKind::Quote
                    } else {
                        DocumentListChildKind::Paragraph
                    };
                    if self.active.as_ref().is_some_and(|active| {
                        (matches!(active, ActiveBlock::ListItem { .. })
                            || matches!(
                                active,
                                ActiveBlock::ListChild {
                                    kind: active_kind,
                                    ..
                                } if *active_kind == kind
                            ))
                            && has_visible_markup(active.markup())
                    }) {
                        self.finish_active();
                    }
                    if self.active.is_none() {
                        let depth = *self.list_items.last().expect("list item checked above");
                        self.start_active(ActiveBlock::ListChild {
                            depth,
                            kind,
                            markup: String::new(),
                        });
                    }
                }
            }
            "blockquote" => {
                self.finish_list_parent();
                self.quote_depth = self.quote_depth.saturating_add(1);
                self.start_active(if let Some(depth) = self.list_items.last() {
                    ActiveBlock::ListChild {
                        depth: *depth,
                        kind: DocumentListChildKind::Quote,
                        markup: String::new(),
                    }
                } else {
                    ActiveBlock::Quote(String::new())
                });
            }
            "ul" | "ol" => {
                self.finish_list_parent();
                if self.lists.is_empty() && self.blocks.last().is_some_and(is_list_block) {
                    self.blocks.push(DocumentBlock::ContainerBoundary);
                }
                self.lists.push((name == "ol").then_some(1));
            }
            "li" => {
                self.finish_active();
                let marker = next_list_marker(&mut self.lists);
                let depth = self.lists.len().saturating_sub(1);
                self.list_items.push(depth);
                self.start_active(ActiveBlock::ListItem {
                    marker,
                    depth,
                    markup: String::new(),
                });
            }
            "em" | "i" => self.start_inline("<i>"),
            "strong" | "b" => self.start_inline("<b>"),
            "s" | "del" => self.start_inline("<s>"),
            "a" => {
                self.ensure_text_target();
                self.flush_pending_space();
                let link = if self.links.is_empty() {
                    match href {
                        Some(href) if has_web_scheme(href) => HtmlLink::External(href.to_owned()),
                        Some(_) => {
                            self.warned = true;
                            HtmlLink::Underline
                        }
                        None => HtmlLink::Underline,
                    }
                } else {
                    self.warned = true;
                    HtmlLink::Inert
                };
                self.append_link_open(&link);
                self.links.push(link);
            }
            "pre" => {
                self.finish_list_parent();
                self.start_active(if let Some(depth) = self.list_items.last() {
                    ActiveBlock::ListChild {
                        depth: *depth,
                        kind: DocumentListChildKind::Code(None),
                        markup: String::new(),
                    }
                } else {
                    ActiveBlock::Code {
                        markup: String::new(),
                        language: None,
                    }
                });
                self.preformatted = true;
            }
            "code" if !self.preformatted => {
                self.start_inline("<tt>");
            }
            "code" => self.set_code_language(language),
            "hr" => {
                self.finish_list_parent();
                if let Some(depth) = self.list_items.last().copied() {
                    self.blocks.push(DocumentBlock::ListRule { depth });
                } else {
                    self.blocks.push(DocumentBlock::Rule);
                }
            }
            "br" => {
                self.ensure_text_target();
                self.pending_space = false;
                self.at_line_start = true;
                self.append_markup("\n");
            }
            "table" => {
                self.finish_list_parent();
                self.table_depth = self.list_items.last().copied();
                let follows_table = match (self.table_depth, self.blocks.last()) {
                    (None, Some(DocumentBlock::TableRow { .. })) => true,
                    (
                        Some(depth),
                        Some(DocumentBlock::ListTableRow {
                            depth: previous_depth,
                            ..
                        }),
                    ) => depth == *previous_depth,
                    _ => false,
                };
                if follows_table {
                    self.blocks.push(DocumentBlock::ContainerBoundary);
                }
            }
            "tr" => self.table_row = Some(Vec::new()),
            "th" | "td" => {
                self.table_cell_header = name == "th";
                self.table_cell = Some(String::new());
                self.pending_space = false;
                self.at_line_start = true;
                self.append_link_openings();
            }
            "main" | "article" | "section" | "header" | "footer" | "nav" | "aside" | "div" => {
                self.finish_list_parent();
            }
            "html" | "body" | "span" | "thead" | "tbody" | "tfoot" => {}
            _ => self.warned = true,
        }

        if !void {
            self.stack.push(name.to_owned());
        } else if self_closing {
            self.end_supported(name);
        }
    }

    fn end(&mut self, name: &str) {
        let Some(position) = self.stack.iter().rposition(|open| open == name) else {
            self.malformed = true;
            return;
        };
        while self.stack.len() > position {
            self.close_top();
        }
    }

    fn end_supported(&mut self, name: &str) {
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.finish_active();
            }
            "li" => {
                self.finish_active();
                self.list_items.pop();
            }
            "p" if matches!(
                self.active,
                Some(
                    ActiveBlock::Paragraph(_)
                        | ActiveBlock::Quote(_)
                        | ActiveBlock::ListChild {
                            kind: DocumentListChildKind::Paragraph | DocumentListChildKind::Quote,
                            ..
                        }
                )
            ) =>
            {
                self.finish_active();
            }
            "p" => {}
            "blockquote" => {
                self.finish_active();
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            "ul" | "ol" => {
                if matches!(
                    self.active,
                    Some(ActiveBlock::ListItem { .. } | ActiveBlock::ListChild { .. })
                ) {
                    self.finish_active();
                }
                self.lists.pop();
            }
            "em" | "i" => self.append_markup("</i>"),
            "strong" | "b" => self.append_markup("</b>"),
            "s" | "del" => self.append_markup("</s>"),
            "a" => {
                if let Some(link) = self.links.pop() {
                    self.append_link_close(&link);
                }
            }
            "pre" => {
                self.finish_active();
                self.preformatted = false;
            }
            "code" if !self.preformatted => {
                self.append_markup("</tt>");
            }
            "table" => self.table_depth = None,
            "tr" => {
                if let Some(cells) = self.table_row.take() {
                    push_table_row(&mut self.blocks, self.table_depth, cells);
                }
            }
            "th" | "td" => {
                self.append_link_closings();
                if let Some(cell) = self.table_cell.take() {
                    self.table_row
                        .get_or_insert_with(Vec::new)
                        .push(DocumentTableCell {
                            header: self.table_cell_header,
                            markup: cell,
                        });
                }
            }
            "main" | "article" | "section" | "header" | "footer" | "nav" | "aside" | "div" => {
                self.finish_active();
            }
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        if self.skipped_depth > 0
            || (self.active.is_none()
                && self.table_cell.is_none()
                && text.chars().all(is_html_whitespace))
        {
            return;
        }
        self.ensure_text_target();
        if self.preformatted {
            self.append_escaped(text);
            return;
        }

        let mut collapsed = String::with_capacity(text.len());
        for character in text.chars() {
            if is_html_whitespace(character) {
                if !self.at_line_start {
                    self.pending_space = true;
                }
            } else {
                if self.pending_space {
                    collapsed.push(' ');
                    self.pending_space = false;
                }
                collapsed.push(character);
                self.at_line_start = false;
            }
        }
        self.append_escaped(&collapsed);
    }

    fn start_active(&mut self, active: ActiveBlock) {
        self.active = Some(active);
        self.pending_space = false;
        self.at_line_start = true;
        self.append_link_openings();
    }

    fn set_code_language(&mut self, language: Option<&'static str>) {
        let Some(language) = language else {
            return;
        };
        match self.active.as_mut() {
            Some(ActiveBlock::Code {
                language: current, ..
            }) => *current = Some(language),
            Some(ActiveBlock::ListChild {
                kind: DocumentListChildKind::Code(current),
                ..
            }) => *current = Some(language),
            _ => {}
        }
    }

    fn ensure_text_target(&mut self) {
        if self.active.is_none() && self.table_cell.is_none() {
            self.start_active(if let Some(depth) = self.list_items.last().copied() {
                ActiveBlock::ListChild {
                    depth,
                    kind: if self.quote_depth > 0 {
                        DocumentListChildKind::Quote
                    } else {
                        DocumentListChildKind::Paragraph
                    },
                    markup: String::new(),
                }
            } else if self.quote_depth > 0 {
                ActiveBlock::Quote(String::new())
            } else {
                ActiveBlock::Paragraph(String::new())
            });
        }
    }

    fn finish_active(&mut self) {
        if self.active.is_some() {
            self.pending_space = false;
            self.at_line_start = true;
            self.append_link_closings();
            finish_block(&mut self.active, &mut self.blocks);
        }
    }

    fn finish_list_parent(&mut self) {
        if self.active.is_some() {
            self.pending_space = false;
            self.at_line_start = true;
            self.append_link_closings();
            finish_parent_list_item(&mut self.active, &mut self.blocks);
        }
    }

    fn append_link_openings(&mut self) {
        for link in self.links.clone() {
            self.append_link_open(&link);
            if self.markup_exceeded {
                break;
            }
        }
    }

    fn append_link_closings(&mut self) {
        for index in (0..self.links.len()).rev() {
            let link = self.links[index].clone();
            self.append_link_close(&link);
            if self.markup_exceeded {
                break;
            }
        }
    }

    fn append_markup(&mut self, markup: &str) {
        if self.markup_exceeded || markup.len() > self.markup_remaining {
            self.markup_exceeded = true;
            return;
        }
        self.markup_remaining -= markup.len();
        append_markup(&mut self.active, &mut self.table_cell, markup);
    }

    fn start_inline(&mut self, markup: &str) {
        self.ensure_text_target();
        self.flush_pending_space();
        self.append_markup(markup);
    }

    fn flush_pending_space(&mut self) {
        if self.pending_space && !self.at_line_start {
            self.pending_space = false;
            self.append_escaped(" ");
        }
    }

    fn append_escaped(&mut self, text: &str) {
        self.append_markup(&escape_document_text(text));
    }

    fn append_link_open(&mut self, link: &HtmlLink) {
        match link {
            HtmlLink::External(destination) => {
                self.append_markup("<a href=\"");
                self.append_escaped(destination);
                self.append_markup("\">");
            }
            HtmlLink::Underline => self.append_markup("<u>"),
            HtmlLink::Inert => {}
        }
    }

    fn append_link_close(&mut self, link: &HtmlLink) {
        if self.active.is_some() || self.table_cell.is_some() {
            match link {
                HtmlLink::External(_) => self.append_markup("</a>"),
                HtmlLink::Underline => self.append_markup("</u>"),
                HtmlLink::Inert => {}
            }
        }
    }

    fn check_markup(&self) -> Result<(), String> {
        if self.markup_exceeded {
            Err("Rendered preview exceeded the 4 MB markup limit".to_owned())
        } else {
            Ok(())
        }
    }

    fn close_top(&mut self) {
        let Some(open) = self.stack.pop() else {
            return;
        };
        if self.skipped_depth > 0 {
            self.skipped_depth -= 1;
        } else {
            self.end_supported(&open);
        }
    }

    fn close_implied_before_start(&mut self, incoming: &str) {
        match incoming {
            "th" | "td" => self.close_current_table_element(&["th", "td"]),
            "tr" => self.close_current_table_element(&["tr"]),
            "thead" | "tbody" | "tfoot" => {
                self.close_current_table_element(&["tr"]);
                self.close_current_table_element(&["thead", "tbody", "tfoot"]);
            }
            _ => {}
        }
        while self
            .stack
            .last()
            .is_some_and(|open| html_start_implies_end(open, incoming))
        {
            self.close_top();
        }
    }

    fn close_current_table_element(&mut self, names: &[&str]) {
        let Some(table) = self.stack.iter().rposition(|open| open == "table") else {
            return;
        };
        let Some(position) = self.stack[table + 1..]
            .iter()
            .rposition(|open| names.contains(&open.as_str()))
            .map(|position| table + 1 + position)
        else {
            return;
        };
        while self.stack.len() > position {
            self.close_top();
        }
    }

    fn finish(mut self) -> Self {
        while !self.stack.is_empty() {
            self.close_top();
        }
        self.finish_active();
        self
    }
}

fn parse_html_bounded(
    html: &str,
    cancellation: &Cancellation,
    limits: ParseLimits,
) -> Result<ParsedDocument, String> {
    let mut budget = ParseBudget::new(cancellation, limits);
    budget.check()?;
    let mut state = HtmlState::new(limits.markup);
    let mut emitter = DefaultEmitter::default();
    emitter.naively_switch_states(true);
    for token in Tokenizer::new_with_emitter(html, emitter) {
        budget.event()?;
        let token = token.map_err(|_| "HTML tokenization failed".to_owned())?;
        match token {
            Token::StartTag(tag) => {
                let name = String::from_utf8_lossy(&tag.name).to_ascii_lowercase();
                let mut href = None;
                let mut language = None;
                for (attribute, value) in &tag.attributes {
                    let attribute = String::from_utf8_lossy(attribute).to_ascii_lowercase();
                    if name == "a" && attribute == "href" {
                        href = Some(String::from_utf8_lossy(&value.value).into_owned());
                    } else if name == "code" && attribute == "class" {
                        language = html_code_language(&String::from_utf8_lossy(&value.value));
                        if language.is_none() {
                            state.warned = true;
                        }
                    } else {
                        state.warned = true;
                    }
                }
                state.start(&name, href.as_deref(), language, tag.self_closing);
                state.check_markup()?;
                budget.nesting(state.stack.len())?;
            }
            Token::EndTag(tag) => {
                let name = String::from_utf8_lossy(&tag.name).to_ascii_lowercase();
                state.end(&name);
                state.check_markup()?;
                budget.nesting(state.stack.len())?;
            }
            Token::String(text) => {
                state.text(&String::from_utf8_lossy(&text.value));
                state.check_markup()?;
            }
            Token::Error(_) => state.malformed = true,
            Token::Comment(_) | Token::Doctype(_) => {}
        }
    }
    let state = state.finish();
    state.check_markup()?;
    if state.malformed {
        return Err("Rendered preview is unavailable because the HTML is malformed".to_owned());
    }
    Ok(ParsedDocument {
        document: Document {
            blocks: state.blocks,
        },
        warnings: state
            .warned
            .then(|| "Unsupported or active HTML content was omitted.".to_owned())
            .into_iter()
            .collect(),
    })
}

fn validate_document(
    parsed: ParsedDocument,
    limits: ParseLimits,
) -> Result<ParsedDocument, String> {
    if parsed.document.blocks.is_empty() {
        return Err("Rendered preview found no supported document content".to_owned());
    }
    if !tables_within_cell_limit(&parsed.document.blocks, limits.table_cells) {
        return Err("Rendered preview exceeded the 512-cell limit for one table".to_owned());
    }
    let markup = parsed
        .document
        .blocks
        .iter()
        .map(block_markup_bytes)
        .sum::<usize>();
    if markup > limits.markup {
        return Err("Rendered preview exceeded the 4 MB markup limit".to_owned());
    }
    if parsed
        .document
        .blocks
        .iter()
        .any(|block| !block_has_balanced_markup(block))
    {
        return Err("Rendered preview contains unsupported document structure".to_owned());
    }
    Ok(parsed)
}

pub fn layout_document(
    document: Document,
    cancellation: &Cancellation,
) -> Result<DocumentLayout, String> {
    layout_document_bounded(document, cancellation, DOCUMENT_TIME_LIMIT)
}

fn layout_document_bounded(
    document: Document,
    cancellation: &Cancellation,
    time_limit: Duration,
) -> Result<DocumentLayout, String> {
    let budget = LayoutBudget::new(cancellation, time_limit);
    let mut units = Vec::new();
    let mut blocks = document.blocks.into_iter().peekable();

    while let Some(block) = blocks.next() {
        budget.check()?;
        match block {
            DocumentBlock::Heading { level, markup } => push_styled_units(
                &mut units,
                DocumentUnitKind::Heading(level),
                &markup,
                None,
                &budget,
            )?,
            DocumentBlock::Paragraph(markup) => push_styled_units(
                &mut units,
                DocumentUnitKind::Paragraph,
                &markup,
                None,
                &budget,
            )?,
            DocumentBlock::ListItem {
                marker,
                depth,
                markup,
            } => {
                let marker_chars = marker.chars().count();
                push_styled_units(
                    &mut units,
                    DocumentUnitKind::ListItem { depth },
                    &markup,
                    Some((format!("{marker} "), marker_chars)),
                    &budget,
                )?;
            }
            DocumentBlock::ListChild {
                depth,
                kind,
                markup,
            } => push_styled_units(
                &mut units,
                match kind {
                    DocumentListChildKind::Code(language) => DocumentUnitKind::Code {
                        list_depth: Some(depth),
                        language,
                    },
                    _ => DocumentUnitKind::ListChild { depth, kind },
                },
                &markup,
                None,
                &budget,
            )?,
            DocumentBlock::Quote(markup) => push_styled_units(
                &mut units,
                DocumentUnitKind::Quote,
                &markup,
                Some(("│ ".to_owned(), 0)),
                &budget,
            )?,
            DocumentBlock::Code { markup, language } => push_styled_units(
                &mut units,
                DocumentUnitKind::Code {
                    list_depth: None,
                    language,
                },
                &markup,
                None,
                &budget,
            )?,
            DocumentBlock::Rule => units.push(rule_unit(None)),
            DocumentBlock::ListRule { depth } => units.push(rule_unit(Some(depth))),
            DocumentBlock::ContainerBoundary => {
                if !matches!(
                    units.last().map(|unit| &unit.kind),
                    Some(DocumentUnitKind::Gap)
                ) {
                    units.push(DocumentUnit {
                        kind: DocumentUnitKind::Gap,
                        text: String::new(),
                        copy_text: String::new(),
                        spans: Vec::new(),
                        wrap: false,
                        first: true,
                        last: true,
                    });
                }
            }
            DocumentBlock::TableRow { cells } => {
                let mut rows = vec![layout_table_row(cells, &budget)?];
                while matches!(blocks.peek(), Some(DocumentBlock::TableRow { .. })) {
                    let Some(DocumentBlock::TableRow { cells }) = blocks.next() else {
                        unreachable!();
                    };
                    rows.push(layout_table_row(cells, &budget)?);
                    budget.check()?;
                }
                units.push(table_unit(None, rows));
            }
            DocumentBlock::ListTableRow { depth, cells } => {
                let mut rows = vec![layout_table_row(cells, &budget)?];
                while matches!(
                    blocks.peek(),
                    Some(DocumentBlock::ListTableRow { depth: next, .. }) if *next == depth
                ) {
                    let Some(DocumentBlock::ListTableRow { cells, .. }) = blocks.next() else {
                        unreachable!();
                    };
                    rows.push(layout_table_row(cells, &budget)?);
                    budget.check()?;
                }
                units.push(table_unit(Some(depth), rows));
            }
        }
    }

    budget.check()?;
    Ok(DocumentLayout { units })
}

struct LayoutBudget<'a> {
    cancellation: &'a Cancellation,
    started: Instant,
    time_limit: Duration,
}

impl<'a> LayoutBudget<'a> {
    fn new(cancellation: &'a Cancellation, time_limit: Duration) -> Self {
        Self {
            cancellation,
            started: Instant::now(),
            time_limit,
        }
    }

    fn check(&self) -> Result<(), String> {
        if self.cancellation.is_cancelled() {
            Err("Rendered preview was cancelled".to_owned())
        } else if self.started.elapsed() >= self.time_limit {
            Err("Rendered preview exceeded the 500 ms layout limit".to_owned())
        } else {
            Ok(())
        }
    }
}

fn decode_document_markup(markup: &str, budget: &LayoutBudget<'_>) -> Result<StyledText, String> {
    let mut text = String::with_capacity(markup.len());
    let mut spans = Vec::new();
    let mut active = Vec::<(&str, DocumentSpanStyle)>::new();
    let mut characters = 0;
    let mut remaining = markup;

    while let Some(start) = remaining.find('<') {
        budget.check()?;
        append_styled_text(
            &mut text,
            &mut spans,
            &active,
            &remaining[..start],
            &mut characters,
            budget,
        )?;
        let Some(end) = remaining[start + 1..].find('>') else {
            return Err("Rendered preview contains unsupported document structure".to_owned());
        };
        let tag = &remaining[start + 1..start + 1 + end];
        if let Some(closing) = tag.strip_prefix('/') {
            if active.pop().map(|(name, _)| name) != Some(closing) {
                return Err("Rendered preview contains unsupported document structure".to_owned());
            }
        } else {
            let (name, style) = match tag {
                "b" => ("b", DocumentSpanStyle::Bold),
                "i" => ("i", DocumentSpanStyle::Italic),
                "s" => ("s", DocumentSpanStyle::Strikethrough),
                "tt" => ("tt", DocumentSpanStyle::Monospace),
                "u" => ("u", DocumentSpanStyle::Underline),
                _ => {
                    let Some(uri) = tag
                        .strip_prefix("a href=\"")
                        .and_then(|value| value.strip_suffix('"'))
                        .map(decode_markup_text)
                        .transpose()?
                        .filter(|uri| has_web_scheme(uri))
                    else {
                        return Err(
                            "Rendered preview contains unsupported document structure".to_owned()
                        );
                    };
                    ("a", DocumentSpanStyle::Link(Arc::from(uri)))
                }
            };
            active.push((name, style));
        }
        remaining = &remaining[start + end + 2..];
    }
    append_styled_text(
        &mut text,
        &mut spans,
        &active,
        remaining,
        &mut characters,
        budget,
    )?;
    if !active.is_empty() {
        return Err("Rendered preview contains unsupported document structure".to_owned());
    }
    Ok(StyledText { text, spans })
}

fn decode_markup_text(markup: &str) -> Result<String, String> {
    gtk::pango::parse_markup(markup, '\0')
        .map(|(_, text, _)| text.to_string())
        .map_err(|_| "Rendered preview contains unsupported document structure".to_owned())
}

fn append_styled_text(
    output: &mut String,
    spans: &mut Vec<DocumentSpan>,
    active: &[(&str, DocumentSpanStyle)],
    escaped: &str,
    characters: &mut usize,
    budget: &LayoutBudget<'_>,
) -> Result<(), String> {
    if escaped.is_empty() {
        return Ok(());
    }
    budget.check()?;
    let decoded = decode_markup_text(escaped)?;
    budget.check()?;
    let start = *characters;
    output.push_str(&decoded);
    let end = start + decoded.chars().count();
    budget.check()?;
    *characters = end;
    for (index, (_, style)) in active.iter().enumerate() {
        budget.check()?;
        if start == end
            || active[..index]
                .iter()
                .any(|(_, previous)| previous == style)
        {
            continue;
        }
        if let Some(previous) = spans.last_mut()
            && previous.style == *style
            && previous.range.end == start
        {
            previous.range.end = end;
        } else {
            spans.push(DocumentSpan {
                range: start..end,
                style: style.clone(),
            });
        }
    }
    Ok(())
}

fn push_styled_units(
    units: &mut Vec<DocumentUnit>,
    kind: DocumentUnitKind,
    markup: &str,
    prefix: Option<(String, usize)>,
    budget: &LayoutBudget<'_>,
) -> Result<(), String> {
    let mut styled = decode_document_markup(markup, budget)?;
    if let Some((prefix, accent_chars)) = prefix {
        let shift = prefix.chars().count();
        for (index, span) in styled.spans.iter_mut().enumerate() {
            if index % 256 == 0 {
                budget.check()?;
            }
            span.range = span.range.start + shift..span.range.end + shift;
        }
        if accent_chars > 0 {
            styled.spans.push(DocumentSpan {
                range: 0..accent_chars,
                style: DocumentSpanStyle::Accent,
            });
        }
        styled.text.insert_str(0, &prefix);
    }

    styled
        .spans
        .sort_by_key(|span| (span.range.start, span.range.end));
    budget.check()?;
    let ranges = document_unit_ranges(&styled.text, budget)?;
    let count = ranges.len();
    let first_unit = units.len();
    let mut unit_char_ranges = Vec::with_capacity(count);
    let mut start_char = 0;
    for (index, range) in ranges.into_iter().enumerate() {
        budget.check()?;
        let starts_midline = range.start > 0 && styled.text.as_bytes()[range.start - 1] != b'\n';
        let ends_midline = range.end < styled.text.len()
            && styled.text.as_bytes()[range.end.saturating_sub(1)] != b'\n';
        let mut end = range.end;
        let ended_with_newline = styled.text[range.clone()].ends_with('\n');
        if ended_with_newline {
            end -= 1;
        }
        let display = &styled.text[range.start..end];
        let end_char = start_char + display.chars().count();
        unit_char_ranges.push(start_char..end_char);
        let last = index + 1 == count;
        let mut copy_text = styled.text[range].to_owned();
        if last && !copy_text.ends_with('\n') {
            copy_text.push('\n');
        }
        units.push(DocumentUnit {
            kind: kind.clone(),
            text: display.to_owned(),
            copy_text,
            spans: Vec::new(),
            wrap: !(starts_midline || ends_midline),
            first: index == 0,
            last,
        });
        start_char = end_char + usize::from(ended_with_newline);
    }

    let mut first_overlap = 0;
    let mut work = 0usize;
    for span in styled.spans {
        if work.is_multiple_of(256) {
            budget.check()?;
        }
        while first_overlap < unit_char_ranges.len()
            && unit_char_ranges[first_overlap].end <= span.range.start
        {
            first_overlap += 1;
        }
        for (index, range) in unit_char_ranges.iter().enumerate().skip(first_overlap) {
            if range.start >= span.range.end {
                break;
            }
            let start = span.range.start.max(range.start);
            let end = span.range.end.min(range.end);
            if start < end {
                units[first_unit + index].spans.push(DocumentSpan {
                    range: start - range.start..end - range.start,
                    style: span.style.clone(),
                });
            }
            work = work.saturating_add(1);
            if work.is_multiple_of(256) {
                budget.check()?;
            }
        }
    }
    budget.check()
}

fn document_unit_ranges(
    text: &str,
    budget: &LayoutBudget<'_>,
) -> Result<Vec<Range<usize>>, String> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut end = 0;
    for line in text.split_inclusive('\n') {
        budget.check()?;
        let line_start = end;
        let next = line_start + line.len();
        let content_end = next - usize::from(line.ends_with('\n'));
        if content_end - line_start > DOCUMENT_UNIT_LINE_TARGET {
            if end > start {
                ranges.push(start..end);
            }
            let mut chunk_start = line_start;
            while content_end - chunk_start > DOCUMENT_UNIT_LINE_TARGET {
                budget.check()?;
                let remaining = content_end - chunk_start;
                let chunk_bytes = if remaining < DOCUMENT_UNIT_LINE_TARGET * 2 {
                    remaining.div_ceil(2)
                } else {
                    DOCUMENT_UNIT_LINE_TARGET
                };
                let mut chunk_end = chunk_start + chunk_bytes;
                while !text.is_char_boundary(chunk_end) {
                    chunk_end -= 1;
                }
                ranges.push(chunk_start..chunk_end);
                chunk_start = chunk_end;
            }
            if chunk_start < next {
                ranges.push(chunk_start..next);
            }
            start = next;
            end = next;
            continue;
        }
        if end > start && next - start > DOCUMENT_UNIT_TARGET {
            ranges.push(start..end);
            start = end;
        }
        end = next;
    }
    if start < text.len() {
        ranges.push(start..text.len());
    } else if ranges.is_empty() {
        ranges.push(0..text.len());
    }
    Ok(ranges)
}

fn layout_table_row(
    cells: Vec<DocumentTableCell>,
    budget: &LayoutBudget<'_>,
) -> Result<Vec<DocumentTableCellLayout>, String> {
    cells
        .into_iter()
        .map(|cell| {
            budget.check()?;
            let styled = decode_document_markup(&cell.markup, budget)?;
            Ok(DocumentTableCellLayout {
                header: cell.header,
                text: styled.text,
                spans: styled.spans,
            })
        })
        .collect()
}

fn table_unit(list_depth: Option<usize>, rows: Vec<Vec<DocumentTableCellLayout>>) -> DocumentUnit {
    let copy_text = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| cell.text.as_str())
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    DocumentUnit {
        kind: DocumentUnitKind::Table { list_depth, rows },
        text: String::new(),
        copy_text,
        spans: Vec::new(),
        wrap: false,
        first: true,
        last: true,
    }
}

fn rule_unit(list_depth: Option<usize>) -> DocumentUnit {
    let text = "────────────────────────".to_owned();
    DocumentUnit {
        kind: DocumentUnitKind::Rule { list_depth },
        copy_text: format!("{text}\n"),
        text,
        spans: Vec::new(),
        wrap: false,
        first: true,
        last: true,
    }
}

fn tables_within_cell_limit(blocks: &[DocumentBlock], limit: usize) -> bool {
    let mut table = None::<Option<usize>>;
    let mut cells = 0usize;
    for block in blocks {
        let next = match block {
            DocumentBlock::TableRow { cells } => Some((None, cells.len())),
            DocumentBlock::ListTableRow { depth, cells } => Some((Some(*depth), cells.len())),
            _ => None,
        };
        if let Some((depth, row_cells)) = next {
            if table != Some(depth) {
                cells = 0;
                table = Some(depth);
            }
            cells = cells.saturating_add(row_cells);
            if cells > limit {
                return false;
            }
        } else {
            table = None;
        }
    }
    true
}

fn block_has_balanced_markup(block: &DocumentBlock) -> bool {
    match block {
        DocumentBlock::Heading { markup, .. }
        | DocumentBlock::Paragraph(markup)
        | DocumentBlock::ListItem { markup, .. }
        | DocumentBlock::ListChild { markup, .. }
        | DocumentBlock::Quote(markup)
        | DocumentBlock::Code { markup, .. } => has_balanced_markup(markup),
        DocumentBlock::TableRow { cells } | DocumentBlock::ListTableRow { cells, .. } => {
            cells.iter().all(|cell| has_balanced_markup(&cell.markup))
        }
        DocumentBlock::Rule | DocumentBlock::ListRule { .. } | DocumentBlock::ContainerBoundary => {
            true
        }
    }
}

fn has_balanced_markup(markup: &str) -> bool {
    let mut tags = Vec::new();
    let mut remaining = markup;
    while let Some(start) = remaining.find('<') {
        let Some(end) = remaining[start + 1..].find('>') else {
            return false;
        };
        let tag = &remaining[start + 1..start + 1 + end];
        if let Some(closing) = tag.strip_prefix('/') {
            if tags.pop() != Some(closing) {
                return false;
            }
        } else {
            let name = if tag.starts_with("a href=\"") && tag.ends_with('"') {
                "a"
            } else if matches!(tag, "i" | "b" | "s" | "tt" | "u") {
                tag
            } else {
                return false;
            };
            tags.push(name);
        }
        remaining = &remaining[start + end + 2..];
    }
    tags.is_empty()
}

fn block_markup_bytes(block: &DocumentBlock) -> usize {
    match block {
        DocumentBlock::Heading { markup, .. }
        | DocumentBlock::Paragraph(markup)
        | DocumentBlock::ListItem { markup, .. }
        | DocumentBlock::ListChild { markup, .. }
        | DocumentBlock::Quote(markup)
        | DocumentBlock::Code { markup, .. } => markup.len(),
        DocumentBlock::TableRow { cells } | DocumentBlock::ListTableRow { cells, .. } => {
            cells.iter().map(|cell| cell.markup.len()).sum()
        }
        DocumentBlock::Rule | DocumentBlock::ListRule { .. } | DocumentBlock::ContainerBoundary => {
            0
        }
    }
}

fn markdown_code_language(kind: &CodeBlockKind<'_>) -> Option<&'static str> {
    match kind {
        CodeBlockKind::Indented => None,
        CodeBlockKind::Fenced(info) => info.split_ascii_whitespace().next().and_then(code_language),
    }
}

fn html_code_language(classes: &str) -> Option<&'static str> {
    classes.split_ascii_whitespace().find_map(|class| {
        class
            .to_ascii_lowercase()
            .strip_prefix("language-")
            .and_then(code_language)
    })
}

fn code_language(hint: &str) -> Option<&'static str> {
    match hint.trim().to_ascii_lowercase().as_str() {
        "bash" | "shell" | "sh" => Some("sh"),
        "c" => Some("c"),
        "c++" | "cpp" => Some("cpp"),
        "c#" | "csharp" | "cs" => Some("c-sharp"),
        "css" => Some("css"),
        "dart" => Some("dart"),
        "diff" | "patch" => Some("diff"),
        "docker" | "dockerfile" => Some("docker"),
        "go" | "golang" => Some("go"),
        "html" => Some("html"),
        "java" => Some("java"),
        "javascript" | "js" => Some("js"),
        "json" => Some("json"),
        "jsx" => Some("jsx"),
        "kotlin" | "kt" => Some("kotlin"),
        "lua" => Some("lua"),
        "make" | "makefile" => Some("makefile"),
        "markdown" | "md" => Some("markdown"),
        "php" => Some("php"),
        "powershell" | "ps1" => Some("powershell"),
        "py" | "python" | "python3" => Some("python3"),
        "rb" | "ruby" => Some("ruby"),
        "rs" | "rust" => Some("rust"),
        "sql" => Some("sql"),
        "swift" => Some("swift"),
        "toml" => Some("toml"),
        "ts" | "typescript" => Some("typescript"),
        "tsx" => Some("typescript-jsx"),
        "vala" => Some("vala"),
        "xml" => Some("xml"),
        "yaml" | "yml" => Some("yaml"),
        _ => None,
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    level as u8
}

fn next_list_marker(lists: &mut [Option<u64>]) -> String {
    match lists.last_mut() {
        Some(Some(next)) => {
            let marker = format!("{next}.");
            *next = next.saturating_add(1);
            marker
        }
        _ => "•".to_owned(),
    }
}

fn is_list_block(block: &DocumentBlock) -> bool {
    matches!(
        block,
        DocumentBlock::ListItem { .. }
            | DocumentBlock::ListChild { .. }
            | DocumentBlock::ListRule { .. }
            | DocumentBlock::ListTableRow { .. }
    )
}

fn push_table_row(
    blocks: &mut Vec<DocumentBlock>,
    list_depth: Option<usize>,
    cells: Vec<DocumentTableCell>,
) {
    if cells.is_empty() {
        return;
    }
    blocks.push(if let Some(depth) = list_depth {
        DocumentBlock::ListTableRow { depth, cells }
    } else {
        DocumentBlock::TableRow { cells }
    });
}

fn finish_block(active: &mut Option<ActiveBlock>, blocks: &mut Vec<DocumentBlock>) {
    let Some(active) = active.take() else {
        return;
    };
    if !has_visible_markup(active.markup()) {
        return;
    }
    blocks.push(match active {
        ActiveBlock::Heading { level, markup } => DocumentBlock::Heading { level, markup },
        ActiveBlock::Paragraph(markup) => DocumentBlock::Paragraph(markup),
        ActiveBlock::ListItem {
            marker,
            depth,
            markup,
        } => DocumentBlock::ListItem {
            marker,
            depth,
            markup,
        },
        ActiveBlock::ListChild {
            depth,
            kind,
            markup,
        } => DocumentBlock::ListChild {
            depth,
            kind,
            markup,
        },
        ActiveBlock::Quote(markup) => DocumentBlock::Quote(markup),
        ActiveBlock::Code { markup, language } => DocumentBlock::Code { markup, language },
    });
}

fn finish_parent_list_item(active: &mut Option<ActiveBlock>, blocks: &mut Vec<DocumentBlock>) {
    match active.take() {
        Some(ActiveBlock::ListItem {
            marker,
            depth,
            markup,
        }) => blocks.push(DocumentBlock::ListItem {
            marker,
            depth,
            markup,
        }),
        other => {
            *active = other;
            finish_block(active, blocks);
        }
    }
}

fn has_visible_markup(markup: &str) -> bool {
    let mut in_tag = false;
    markup.chars().any(|character| match character {
        '<' => {
            in_tag = true;
            false
        }
        '>' if in_tag => {
            in_tag = false;
            false
        }
        _ => !in_tag && !character.is_whitespace(),
    })
}

fn append_markup(active: &mut Option<ActiveBlock>, cell: &mut Option<String>, markup: &str) {
    if let Some(cell) = cell {
        cell.push_str(markup);
    } else {
        active
            .get_or_insert_with(|| ActiveBlock::Paragraph(String::new()))
            .markup_mut()
            .push_str(markup);
    }
}

fn append_escaped(active: &mut Option<ActiveBlock>, cell: &mut Option<String>, text: &str) {
    append_markup(active, cell, &escape_document_text(text));
}

fn escape_document_text(text: &str) -> glib::GString {
    if text.contains('\0') {
        glib::markup_escape_text(&text.replace('\0', "�"))
    } else {
        glib::markup_escape_text(text)
    }
}

fn html_start_implies_end(open: &str, incoming: &str) -> bool {
    (open == "li" && incoming == "li")
        || (open == "p"
            && matches!(
                incoming,
                "address"
                    | "article"
                    | "aside"
                    | "blockquote"
                    | "div"
                    | "dl"
                    | "fieldset"
                    | "footer"
                    | "form"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "header"
                    | "hr"
                    | "menu"
                    | "nav"
                    | "ol"
                    | "p"
                    | "pre"
                    | "section"
                    | "table"
                    | "ul"
            ))
}

fn is_void_html_tag(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn is_html_whitespace(character: char) -> bool {
    matches!(character, '\t' | '\n' | '\u{000C}' | '\r' | ' ')
}

fn is_omitted_html_tag(name: &str) -> bool {
    matches!(
        name,
        "head"
            | "title"
            | "script"
            | "style"
            | "form"
            | "input"
            | "button"
            | "select"
            | "option"
            | "textarea"
            | "iframe"
            | "frame"
            | "frameset"
            | "object"
            | "embed"
            | "img"
            | "picture"
            | "audio"
            | "video"
            | "source"
            | "track"
            | "canvas"
            | "svg"
            | "math"
            | "link"
            | "meta"
            | "base"
    )
}

pub fn has_web_scheme(uri: &str) -> bool {
    uri.split_once(':').is_some_and(|(scheme, _)| {
        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
    })
}

#[cfg(test)]
mod tests;
