// SPDX-License-Identifier: MIT

use std::ops::Range;
use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    pub units: Vec<Unit>,
    pub notices: Vec<String>,
}

impl Document {
    pub fn new(units: Vec<Unit>) -> Self {
        Document {
            units,
            notices: Vec::new(),
        }
    }

    pub fn notices<T: Into<String>>(mut self, notices: impl IntoIterator<Item = T>) -> Self {
        self.notices = notices.into_iter().map(Into::into).collect();
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    pub kind: UnitKind,
    /// What it shows. A list item's marker ("•", "3.") leads it, with an `Accent` span.
    pub text: String,
    pub copy_text: String,
    pub spans: Vec<Span>,
    /// False when the unit starts or ends inside a line (a line split at 2 KiB).
    pub wrap: bool,
    /// The block's first and last unit.
    pub first: bool,
    pub last: bool,
}

impl Unit {
    /// A whole one-unit block with no spans: `copy_text` is the text and a newline.
    pub fn new(kind: UnitKind, text: impl Into<String>) -> Self {
        let text = text.into();
        Unit {
            kind,
            copy_text: format!("{text}\n"),
            text,
            spans: Vec::new(),
            wrap: true,
            first: true,
            last: true,
        }
    }

    pub fn spans(mut self, spans: impl IntoIterator<Item = Span>) -> Self {
        self.spans = spans.into_iter().collect();
        self
    }

    pub fn copy_text(mut self, copy: impl Into<String>) -> Self {
        self.copy_text = copy.into();
        self
    }

    /// Where it sits in its block, and whether it wraps (see the fields).
    pub fn part(mut self, first: bool, last: bool, wrap: bool) -> Self {
        self.first = first;
        self.last = last;
        self.wrap = wrap;
        self
    }

    pub fn from_char_spans(
        mut self,
        spans: impl IntoIterator<Item = (Range<usize>, SpanStyle)>,
    ) -> Self {
        self.spans = char_spans(&self.text, spans);
        self
    }
}

/// `spans` in character ranges of `text`, as byte ranges.
pub fn char_spans(
    text: &str,
    spans: impl IntoIterator<Item = (Range<usize>, SpanStyle)>,
) -> Vec<Span> {
    // Character index to byte offset, one past the end included.
    let bytes: Vec<usize> = text
        .char_indices()
        .map(|(b, _)| b)
        .chain([text.len()])
        .collect();
    let at = |c: usize| bytes[c.min(bytes.len() - 1)];
    spans
        .into_iter()
        .map(|(r, style)| Span {
            range: at(r.start)..at(r.end),
            style,
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub enum UnitKind {
    /// Levels 1 to 6.
    Heading(u8),
    Paragraph,
    ListItem {
        depth: usize,
    },
    /// A paragraph, heading, quote or code block inside a list item, after its first line.
    ListChild {
        depth: usize,
        kind: ListChildKind,
    },
    Quote,
    Code {
        list_depth: Option<usize>,
        language: Option<String>,
    },
    Rule {
        list_depth: Option<usize>,
    },
    Table {
        list_depth: Option<usize>,
        rows: Vec<Vec<Cell>>,
    },
    Media {
        list_depth: Option<usize>,
        media: Media,
    },
    /// Space between containers.
    Gap,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ListChildKind {
    Paragraph,
    Heading(u8),
    Quote,
    Code(Option<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Media {
    Image,
    /// A Mermaid diagram, rendered by Strata, with its source for Copy diagram source.
    Diagram {
        source: Arc<str>,
    },
    /// An equation rendered by Strata. A display equation offers Copy equation source.
    Equation {
        source: Arc<str>,
        display: bool,
    },
}

/// A table cell.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
    pub header: bool,
    pub text: String,
    pub spans: Vec<Span>,
}

impl Cell {
    pub fn new(text: impl Into<String>) -> Self {
        Cell {
            text: text.into(),
            ..Cell::default()
        }
    }

    pub fn header(text: impl Into<String>) -> Self {
        Cell {
            header: true,
            text: text.into(),
            ..Cell::default()
        }
    }

    pub fn spans(mut self, spans: impl IntoIterator<Item = Span>) -> Self {
        self.spans = spans.into_iter().collect();
        self
    }
}

/// A styled byte range of a unit's or a cell's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub style: SpanStyle,
}

impl Span {
    pub fn new(range: Range<usize>, style: SpanStyle) -> Self {
        Span { range, style }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpanStyle {
    /// A list marker, a heading's accent: the accent colour.
    Accent,
    Bold,
    Italic,
    Strike,
    Mono,
    Underline,
    Link(Arc<str>),
    /// An inline equation: its range is one U+FFFC placeholder, and this is its source.
    Math(Arc<str>),
}
