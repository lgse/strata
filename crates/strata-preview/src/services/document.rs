// SPDX-License-Identifier: MIT

use std::{
    ffi::OsStr,
    ops::Range,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::sandbox::Cancellation;

mod html;
mod layout;
mod markdown;

use html::parse_html_bounded;
pub use layout::layout_document;
#[cfg(test)]
use layout::{LayoutBudget, append_styled_text};
use markdown::{MarkdownProfile, parse_markdown_bounded};

pub const DOCUMENT_INPUT_LIMIT: usize = 1024 * 1024;
pub const DOCUMENT_EVENT_LIMIT: usize = 20_000;
pub const DOCUMENT_DEPTH_LIMIT: usize = 32;
pub const DOCUMENT_TABLE_COLUMN_LIMIT: usize = super::table::TABLE_COLUMN_LIMIT;
pub const DOCUMENT_MARKUP_LIMIT: usize = 4 * 1024 * 1024;
pub const DOCUMENT_TIME_LIMIT: Duration = Duration::from_millis(500);
pub const DOCUMENT_UNIT_TARGET: usize = 32 * 1024;
pub const DOCUMENT_UNIT_LINE_TARGET: usize = 2 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentKind {
    Markdown,
    Html,
    Rtf,
    Csv,
    Tsv,
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
    Image {
        destination: String,
        alt: String,
        list_depth: Option<usize>,
    },
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
    Media {
        source: DocumentMedia,
        list_depth: Option<usize>,
    },
    Gap,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DocumentMedia {
    Image(String),
    Mermaid(String),
    Math { source: String, display: bool },
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
    Math(Arc<str>),
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
    table_columns: usize,
    markup: usize,
    time: Duration,
}

impl Default for ParseLimits {
    fn default() -> Self {
        Self {
            events: DOCUMENT_EVENT_LIMIT,
            depth: DOCUMENT_DEPTH_LIMIT,
            table_columns: DOCUMENT_TABLE_COLUMN_LIMIT,
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
        "application/rtf" | "text/rtf" => return Some(DocumentKind::Rtf),
        "text/csv" => return Some(DocumentKind::Csv),
        "text/tab-separated-values" => return Some(DocumentKind::Tsv),
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
        Some("rtf") => Some(DocumentKind::Rtf),
        Some("csv") => Some(DocumentKind::Csv),
        Some("tsv") => Some(DocumentKind::Tsv),
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
        DocumentKind::Markdown => parse_markdown_bounded(
            source,
            cancellation,
            limits,
            MarkdownProfile::DOCUMENT_PREVIEW,
        ),
        DocumentKind::Html => parse_html_bounded(source, cancellation, limits),
        DocumentKind::Rtf => parse_html_bounded(
            &super::rtf::to_html(source, cancellation)?,
            cancellation,
            limits,
        ),
        DocumentKind::Csv => super::table::parse_delimited(source, b',', cancellation),
        DocumentKind::Tsv => super::table::parse_delimited(source, b'\t', cancellation),
    }?;
    validate_document(parsed, limits)
}

// Release notes retain their legacy unbounded parser limits and hide HTML comments.
pub fn parse_markdown(markdown: &str) -> Document {
    let cancellation = Cancellation::default();
    let limits = ParseLimits {
        events: usize::MAX,
        depth: usize::MAX,
        table_columns: usize::MAX,
        markup: usize::MAX,
        time: Duration::MAX,
    };
    parse_markdown_bounded(
        markdown,
        &cancellation,
        limits,
        MarkdownProfile::RELEASE_NOTES,
    )
    .map(|parsed| parsed.document)
    .unwrap_or(Document { blocks: Vec::new() })
}

fn validate_document(
    parsed: ParsedDocument,
    limits: ParseLimits,
) -> Result<ParsedDocument, String> {
    if parsed.document.blocks.is_empty() {
        return Err("Rendered preview found no supported document content".to_owned());
    }
    if !tables_within_column_limit(&parsed.document.blocks, limits.table_columns) {
        return Err("Rendered preview exceeded the column budget for one table".to_owned());
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

fn tables_within_column_limit(blocks: &[DocumentBlock], limit: usize) -> bool {
    blocks.iter().all(|block| match block {
        DocumentBlock::TableRow { cells } | DocumentBlock::ListTableRow { cells, .. } => {
            cells.len() <= limit
        }
        _ => true,
    })
}

fn block_has_balanced_markup(block: &DocumentBlock) -> bool {
    match block {
        DocumentBlock::Heading { markup, .. }
        | DocumentBlock::Paragraph(markup)
        | DocumentBlock::ListItem { markup, .. }
        | DocumentBlock::ListChild { markup, .. }
        | DocumentBlock::Quote(markup)
        | DocumentBlock::Code { markup, .. } => has_balanced_markup(markup),
        DocumentBlock::Image { .. } => true,
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
            } else if tag == "span font_family=\"strata-math\"" {
                "span"
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
        DocumentBlock::Image {
            destination, alt, ..
        } => destination.len() + alt.len(),
        DocumentBlock::TableRow { cells } | DocumentBlock::ListTableRow { cells, .. } => {
            cells.iter().map(|cell| cell.markup.len()).sum()
        }
        DocumentBlock::Rule | DocumentBlock::ListRule { .. } | DocumentBlock::ContainerBoundary => {
            0
        }
    }
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
        "mermaid" => Some("mermaid"),
        "latex" | "math" => Some("latex"),
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

pub fn has_web_scheme(uri: &str) -> bool {
    uri.split_once(':').is_some_and(|(scheme, _)| {
        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
    })
}

#[cfg(test)]
mod tests;
