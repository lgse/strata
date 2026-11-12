// SPDX-License-Identifier: MIT
//! Document blocks to styled, selectable text units.

use std::{
    iter::Peekable,
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};

use super::{
    DOCUMENT_TIME_LIMIT, DOCUMENT_UNIT_LINE_TARGET, DOCUMENT_UNIT_TARGET, Document, DocumentBlock,
    DocumentLayout, DocumentListChildKind, DocumentMedia, DocumentSpan, DocumentSpanStyle,
    DocumentTableCell, DocumentTableCellLayout, DocumentUnit, DocumentUnitKind, has_web_scheme,
};
use crate::sandbox::Cancellation;

const UNSUPPORTED_STRUCTURE: &str = "Rendered preview contains unsupported document structure";
const MATH_SPAN: &str = "span font_family=\"strata-math\"";

pub(super) struct LayoutBudget<'a> {
    cancellation: &'a Cancellation,
    started: Instant,
    time_limit: Duration,
}

impl<'a> LayoutBudget<'a> {
    pub(super) fn new(cancellation: &'a Cancellation, time_limit: Duration) -> Self {
        Self {
            cancellation,
            started: Instant::now(),
            time_limit,
        }
    }

    pub(super) fn check(&self) -> Result<(), String> {
        if self.cancellation.is_cancelled() {
            Err("Rendered preview was cancelled".to_owned())
        } else if self.started.elapsed() >= self.time_limit {
            Err("Rendered preview exceeded the 500 ms layout limit".to_owned())
        } else {
            Ok(())
        }
    }
}

struct StyledText {
    text: String,
    spans: Vec<DocumentSpan>,
}

type ActiveStyles = Vec<(&'static str, DocumentSpanStyle)>;

/// Plain text decoded from markup, with the spans of the styles open around it.
struct StyledBuilder {
    text: String,
    spans: Vec<DocumentSpan>,
    characters: usize,
}

fn style_repeats(active: &[(&str, DocumentSpanStyle)], index: usize) -> bool {
    let style = &active[index].1;
    active[..index]
        .iter()
        .any(|(_, previous)| previous == style)
}

impl StyledBuilder {
    fn extend_span(&mut self, style: &DocumentSpanStyle, range: Range<usize>) {
        match self.spans.last_mut() {
            Some(previous) if previous.style == *style && previous.range.end == range.start => {
                previous.range.end = range.end;
            }
            _ => self.spans.push(DocumentSpan {
                range,
                style: style.clone(),
            }),
        }
    }

    fn append(
        &mut self,
        active: &[(&str, DocumentSpanStyle)],
        escaped: &str,
        budget: &LayoutBudget<'_>,
    ) -> Result<(), String> {
        if escaped.is_empty() {
            return Ok(());
        }
        budget.check()?;
        let decoded = decode_markup_text(escaped)?;
        budget.check()?;
        let start = self.characters;
        self.text.push_str(&decoded);
        let end = start + decoded.chars().count();
        budget.check()?;
        self.characters = end;
        for (index, (_, style)) in active.iter().enumerate() {
            budget.check()?;
            if start == end || style_repeats(active, index) {
                continue;
            }
            self.extend_span(style, start..end);
        }
        Ok(())
    }

    /// Appends one equation placeholder and returns the markup after its span.
    fn append_math<'m>(
        &mut self,
        active: &[(&str, DocumentSpanStyle)],
        body: &'m str,
        budget: &LayoutBudget<'_>,
    ) -> Result<&'m str, String> {
        let close = body.find("</span>").ok_or("Unclosed equation span")?;
        let source = decode_markup_text(&body[..close])?;
        let offset = self.characters;
        self.append(active, "\u{fffc}", budget)?;
        self.spans.push(DocumentSpan {
            range: offset..self.characters,
            style: DocumentSpanStyle::Math(Arc::from(source)),
        });
        Ok(&body[close + "</span>".len()..])
    }
}

#[cfg(test)]
pub(super) fn append_styled_text(
    output: &mut String,
    spans: &mut Vec<DocumentSpan>,
    active: &[(&str, DocumentSpanStyle)],
    escaped: &str,
    characters: &mut usize,
    budget: &LayoutBudget<'_>,
) -> Result<(), String> {
    let mut builder = StyledBuilder {
        text: std::mem::take(output),
        spans: std::mem::take(spans),
        characters: *characters,
    };
    let result = builder.append(active, escaped, budget);
    *output = builder.text;
    *spans = builder.spans;
    *characters = builder.characters;
    result
}

fn link_style(tag: &str) -> Result<(&'static str, DocumentSpanStyle), String> {
    let Some(uri) = tag
        .strip_prefix("a href=\"")
        .and_then(|value| value.strip_suffix('"'))
        .map(decode_markup_text)
        .transpose()?
        .filter(|uri| has_web_scheme(uri))
    else {
        return Err(UNSUPPORTED_STRUCTURE.to_owned());
    };
    Ok(("a", DocumentSpanStyle::Link(Arc::from(uri))))
}

fn opening_style(tag: &str) -> Result<(&'static str, DocumentSpanStyle), String> {
    match tag {
        "b" => Ok(("b", DocumentSpanStyle::Bold)),
        "i" => Ok(("i", DocumentSpanStyle::Italic)),
        "s" => Ok(("s", DocumentSpanStyle::Strikethrough)),
        "tt" => Ok(("tt", DocumentSpanStyle::Monospace)),
        "u" => Ok(("u", DocumentSpanStyle::Underline)),
        _ => link_style(tag),
    }
}

fn apply_tag(active: &mut ActiveStyles, tag: &str) -> Result<(), String> {
    let Some(closing) = tag.strip_prefix('/') else {
        active.push(opening_style(tag)?);
        return Ok(());
    };
    if active.pop().map(|(name, _)| name) != Some(closing) {
        return Err(UNSUPPORTED_STRUCTURE.to_owned());
    }
    Ok(())
}

fn decode_document_markup(markup: &str, budget: &LayoutBudget<'_>) -> Result<StyledText, String> {
    let mut builder = StyledBuilder {
        text: String::with_capacity(markup.len()),
        spans: Vec::new(),
        characters: 0,
    };
    let mut active = ActiveStyles::new();
    let mut remaining = markup;
    while let Some(start) = remaining.find('<') {
        budget.check()?;
        builder.append(&active, &remaining[..start], budget)?;
        let Some(end) = remaining[start + 1..].find('>') else {
            return Err(UNSUPPORTED_STRUCTURE.to_owned());
        };
        let tag = &remaining[start + 1..start + 1 + end];
        let after = &remaining[start + end + 2..];
        remaining = if tag == MATH_SPAN {
            builder.append_math(&active, after, budget)?
        } else {
            apply_tag(&mut active, tag)?;
            after
        };
    }
    builder.append(&active, remaining, budget)?;
    if !active.is_empty() {
        return Err(UNSUPPORTED_STRUCTURE.to_owned());
    }
    Ok(StyledText {
        text: builder.text,
        spans: builder.spans,
    })
}

fn decode_markup_text(markup: &str) -> Result<String, String> {
    pango::parse_markup(markup, '\0')
        .map(|(_, text, _)| text.to_string())
        .map_err(|_| UNSUPPORTED_STRUCTURE.to_owned())
}

fn apply_prefix(
    styled: &mut StyledText,
    prefix: &str,
    accent_chars: usize,
    budget: &LayoutBudget<'_>,
) -> Result<(), String> {
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
    styled.text.insert_str(0, prefix);
    Ok(())
}

fn prefixed_markup(
    markup: &str,
    prefix: Option<(String, usize)>,
    budget: &LayoutBudget<'_>,
) -> Result<StyledText, String> {
    let mut styled = decode_document_markup(markup, budget)?;
    if let Some((prefix, accent_chars)) = prefix {
        apply_prefix(&mut styled, &prefix, accent_chars, budget)?;
    }
    styled
        .spans
        .sort_by_key(|span| (span.range.start, span.range.end));
    budget.check()?;
    Ok(styled)
}

/// One unit of `text` over `range`, with its displayed character count and whether a
/// newline ended it.
fn text_unit(
    kind: &DocumentUnitKind,
    text: &str,
    range: Range<usize>,
    (index, count): (usize, usize),
) -> (DocumentUnit, usize, bool) {
    let bytes = text.as_bytes();
    let starts_midline = range.start > 0 && bytes[range.start - 1] != b'\n';
    let ends_midline = range.end < text.len() && bytes[range.end.saturating_sub(1)] != b'\n';
    let ended_with_newline = text[range.clone()].ends_with('\n');
    let end = range.end - usize::from(ended_with_newline);
    let display = &text[range.start..end];
    let last = index + 1 == count;
    let mut copy_text = text[range].to_owned();
    if last && !copy_text.ends_with('\n') {
        copy_text.push('\n');
    }
    let unit = DocumentUnit {
        kind: kind.clone(),
        text: display.to_owned(),
        copy_text,
        spans: Vec::new(),
        wrap: !(starts_midline || ends_midline),
        first: index == 0,
        last,
    };
    (unit, display.chars().count(), ended_with_newline)
}

/// Splits spans across the units whose character ranges they overlap.
struct SpanPlacement<'u, 'b, 'a> {
    units: &'u mut [DocumentUnit],
    ranges: &'u [Range<usize>],
    budget: &'b LayoutBudget<'a>,
    first_overlap: usize,
    work: usize,
}

impl SpanPlacement<'_, '_, '_> {
    fn skip_ended(&mut self, span: &DocumentSpan) {
        while self.first_overlap < self.ranges.len()
            && self.ranges[self.first_overlap].end <= span.range.start
        {
            self.first_overlap += 1;
        }
    }

    fn place(&mut self, span: &DocumentSpan) -> Result<(), String> {
        let ranges = self.ranges;
        for (index, range) in ranges.iter().enumerate().skip(self.first_overlap) {
            if range.start >= span.range.end {
                break;
            }
            let start = span.range.start.max(range.start);
            let end = span.range.end.min(range.end);
            if start < end {
                self.units[index].spans.push(DocumentSpan {
                    range: start - range.start..end - range.start,
                    style: span.style.clone(),
                });
            }
            self.work = self.work.saturating_add(1);
            if self.work.is_multiple_of(256) {
                self.budget.check()?;
            }
        }
        Ok(())
    }

    fn place_all(mut self, spans: Vec<DocumentSpan>) -> Result<(), String> {
        for span in spans {
            if self.work.is_multiple_of(256) {
                self.budget.check()?;
            }
            self.skip_ended(&span);
            self.place(&span)?;
        }
        Ok(())
    }
}

/// Equations copy as their `$source$`, not as the placeholder they display.
fn restore_math_copy_text(units: &mut [DocumentUnit]) {
    for unit in units {
        let has_math = unit
            .spans
            .iter()
            .any(|span| matches!(span.style, DocumentSpanStyle::Math(_)));
        if !has_math {
            continue;
        }
        let newline = unit.copy_text.ends_with('\n');
        unit.copy_text = unit.copy_range(0..unit.text.chars().count());
        if newline {
            unit.copy_text.push('\n');
        }
    }
}

impl DocumentUnit {
    pub fn copy_range(&self, range: Range<usize>) -> String {
        let mut output = String::new();
        for (index, character) in self
            .text
            .chars()
            .enumerate()
            .skip(range.start)
            .take(range.end.saturating_sub(range.start))
        {
            if let Some(source) = self.spans.iter().find_map(|span| match &span.style {
                DocumentSpanStyle::Math(source) if span.range.contains(&index) => Some(source),
                _ => None,
            }) {
                output.push('$');
                output.push_str(source);
                output.push('$');
            } else {
                output.push(character);
            }
        }
        output
    }
}

fn document_word_boundary(text: &str) -> Option<usize> {
    if let Some(space) = text.rfind(' ') {
        return Some(space + 1);
    }
    if text.is_ascii() {
        ['\t', '\r', '\u{b}', '\u{c}']
            .into_iter()
            .filter_map(|whitespace| text.rfind(whitespace))
            .max()
            .map(|offset| offset + 1)
    } else {
        text.char_indices()
            .rev()
            .find(|(_, character)| character.is_whitespace())
            .map(|(offset, whitespace)| offset + whitespace.len_utf8())
    }
}

fn next_chunk_end(
    text: &str,
    chunk_start: usize,
    content_end: usize,
    has_word_boundaries: bool,
) -> usize {
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
    // Keep prose words intact, but scan unbroken tokens only once per line.
    if has_word_boundaries && let Some(end) = document_word_boundary(&text[chunk_start..chunk_end])
    {
        chunk_end = chunk_start + end;
    }
    chunk_end
}

/// Chunks one overlong line, ending with the remainder through `next`.
fn long_line_ranges(
    text: &str,
    content: Range<usize>,
    next: usize,
    budget: &LayoutBudget<'_>,
) -> Result<Vec<Range<usize>>, String> {
    let content_end = content.end;
    let has_word_boundaries = document_word_boundary(&text[content.clone()]).is_some();
    let mut ranges = Vec::new();
    let mut chunk_start = content.start;
    while content_end - chunk_start > DOCUMENT_UNIT_LINE_TARGET {
        budget.check()?;
        let chunk_end = next_chunk_end(text, chunk_start, content_end, has_word_boundaries);
        ranges.push(chunk_start..chunk_end);
        chunk_start = chunk_end;
    }
    if chunk_start < next {
        ranges.push(chunk_start..next);
    }
    Ok(ranges)
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
            ranges.extend(long_line_ranges(
                text,
                line_start..content_end,
                next,
                budget,
            )?);
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

fn image_unit(destination: String, alt: String, list_depth: Option<usize>) -> DocumentUnit {
    DocumentUnit {
        kind: DocumentUnitKind::Media {
            source: DocumentMedia::Image(destination),
            list_depth,
        },
        text: alt.clone(),
        copy_text: format!("[Image: {alt}]\n"),
        spans: Vec::new(),
        wrap: true,
        first: true,
        last: true,
    }
}

fn gap_unit() -> DocumentUnit {
    DocumentUnit {
        kind: DocumentUnitKind::Gap,
        text: String::new(),
        copy_text: String::new(),
        spans: Vec::new(),
        wrap: false,
        first: true,
        last: true,
    }
}

fn list_child_unit_kind(depth: usize, kind: DocumentListChildKind) -> DocumentUnitKind {
    match kind {
        DocumentListChildKind::Code(language) => DocumentUnitKind::Code {
            list_depth: Some(depth),
            language,
        },
        _ => DocumentUnitKind::ListChild { depth, kind },
    }
}

/// Whether `block` continues the table whose rows sit at `list_depth`.
fn continues_table(block: &DocumentBlock, list_depth: Option<usize>) -> bool {
    match (block, list_depth) {
        (DocumentBlock::TableRow { .. }, None) => true,
        (DocumentBlock::ListTableRow { depth, .. }, Some(list_depth)) => *depth == list_depth,
        _ => false,
    }
}

fn table_row_cells(block: DocumentBlock) -> Vec<DocumentTableCell> {
    match block {
        DocumentBlock::TableRow { cells } | DocumentBlock::ListTableRow { cells, .. } => cells,
        _ => Vec::new(),
    }
}

type Blocks = Peekable<std::vec::IntoIter<DocumentBlock>>;

struct Layout<'b, 'a> {
    budget: &'b LayoutBudget<'a>,
    units: Vec<DocumentUnit>,
}

impl Layout<'_, '_> {
    fn styled(
        &mut self,
        kind: DocumentUnitKind,
        markup: &str,
        prefix: Option<(String, usize)>,
    ) -> Result<(), String> {
        let budget = self.budget;
        let styled = prefixed_markup(markup, prefix, budget)?;
        let ranges = document_unit_ranges(&styled.text, budget)?;
        let count = ranges.len();
        let first_unit = self.units.len();
        let mut unit_char_ranges = Vec::with_capacity(count);
        let mut start_char = 0;
        for (index, range) in ranges.into_iter().enumerate() {
            budget.check()?;
            let (unit, display_chars, ended_with_newline) =
                text_unit(&kind, &styled.text, range, (index, count));
            let end_char = start_char + display_chars;
            unit_char_ranges.push(start_char..end_char);
            self.units.push(unit);
            start_char = end_char + usize::from(ended_with_newline);
        }
        let units = &mut self.units[first_unit..];
        SpanPlacement {
            units,
            ranges: &unit_char_ranges,
            budget,
            first_overlap: 0,
            work: 0,
        }
        .place_all(styled.spans)?;
        restore_math_copy_text(&mut self.units[first_unit..]);
        budget.check()
    }

    fn gap(&mut self) {
        if !matches!(
            self.units.last().map(|unit| &unit.kind),
            Some(DocumentUnitKind::Gap)
        ) {
            self.units.push(gap_unit());
        }
    }

    fn table(
        &mut self,
        list_depth: Option<usize>,
        cells: Vec<DocumentTableCell>,
        rest: &mut Blocks,
    ) -> Result<(), String> {
        let mut rows = vec![layout_table_row(cells, self.budget)?];
        while let Some(block) = rest.next_if(|block| continues_table(block, list_depth)) {
            rows.push(layout_table_row(table_row_cells(block), self.budget)?);
            self.budget.check()?;
        }
        self.units.push(table_unit(list_depth, rows));
        Ok(())
    }

    fn block(&mut self, block: DocumentBlock, rest: &mut Blocks) -> Result<(), String> {
        match block {
            DocumentBlock::Heading { level, markup } => {
                self.styled(DocumentUnitKind::Heading(level), &markup, None)
            }
            DocumentBlock::Paragraph(markup) => {
                self.styled(DocumentUnitKind::Paragraph, &markup, None)
            }
            DocumentBlock::ListItem {
                marker,
                depth,
                markup,
            } => {
                let marker_chars = marker.chars().count();
                let prefix = Some((format!("{marker} "), marker_chars));
                self.styled(DocumentUnitKind::ListItem { depth }, &markup, prefix)
            }
            DocumentBlock::ListChild {
                depth,
                kind,
                markup,
            } => self.styled(list_child_unit_kind(depth, kind), &markup, None),
            DocumentBlock::Quote(markup) => {
                self.styled(DocumentUnitKind::Quote, &markup, Some(("│ ".to_owned(), 0)))
            }
            DocumentBlock::Code { markup, language } => {
                let kind = DocumentUnitKind::Code {
                    list_depth: None,
                    language,
                };
                self.styled(kind, &markup, None)
            }
            DocumentBlock::Image {
                destination,
                alt,
                list_depth,
            } => {
                self.units.push(image_unit(destination, alt, list_depth));
                Ok(())
            }
            DocumentBlock::Rule => {
                self.units.push(rule_unit(None));
                Ok(())
            }
            DocumentBlock::ListRule { depth } => {
                self.units.push(rule_unit(Some(depth)));
                Ok(())
            }
            DocumentBlock::ContainerBoundary => {
                self.gap();
                Ok(())
            }
            DocumentBlock::TableRow { cells } => self.table(None, cells, rest),
            DocumentBlock::ListTableRow { depth, cells } => self.table(Some(depth), cells, rest),
        }
    }
}

/// A code unit that previews as a diagram or equation, with its list depth and language.
fn diagram_language(kind: &DocumentUnitKind) -> Option<(Option<usize>, &'static str)> {
    match kind {
        DocumentUnitKind::Code {
            list_depth,
            language: Some(language @ ("mermaid" | "latex")),
        } => Some((*list_depth, *language)),
        _ => None,
    }
}

/// Joins a code block's continuation units, keeping the originals to fall back to.
fn join_diagram(
    mut unit: DocumentUnit,
    units: &mut impl Iterator<Item = DocumentUnit>,
) -> (DocumentUnit, Vec<DocumentUnit>) {
    let mut source_units = vec![unit.clone()];
    while !unit.last {
        let Some(continuation) = units.next() else {
            break;
        };
        unit.text.push_str(&continuation.text);
        unit.copy_text.push_str(&continuation.copy_text);
        unit.last = continuation.last;
        source_units.push(continuation);
    }
    (unit, source_units)
}

fn exceeds_diagram_limit(unit: &DocumentUnit, language: &str) -> bool {
    let limit = if language == "latex" {
        crate::services::document_media::MATH_INPUT_LIMIT
    } else {
        crate::services::document_media::DIAGRAM_INPUT_LIMIT
    };
    unit.copy_text.len() > limit || unit.copy_text.lines().count() > 256
}

fn diagram_media(source: String, language: &str) -> DocumentMedia {
    if language == "latex" {
        DocumentMedia::Math {
            source,
            display: true,
        }
    } else {
        DocumentMedia::Mermaid(source)
    }
}

fn render_diagrams(
    units: Vec<DocumentUnit>,
    budget: &LayoutBudget<'_>,
) -> Result<Vec<DocumentUnit>, String> {
    let mut rendered_units = Vec::with_capacity(units.len());
    let mut units = units.into_iter().peekable();
    while let Some(unit) = units.next() {
        let Some((list_depth, language)) = diagram_language(&unit.kind) else {
            rendered_units.push(unit);
            budget.check()?;
            continue;
        };
        let (mut unit, source_units) = join_diagram(unit, &mut units);
        if exceeds_diagram_limit(&unit, language) {
            rendered_units.extend(source_units);
            continue;
        }
        unit.kind = DocumentUnitKind::Media {
            source: diagram_media(unit.copy_text.clone(), language),
            list_depth,
        };
        rendered_units.push(unit);
        budget.check()?;
    }
    Ok(rendered_units)
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
    let mut layout = Layout {
        budget: &budget,
        units: Vec::new(),
    };
    let mut blocks = document.blocks.into_iter().peekable();
    while let Some(block) = blocks.next() {
        budget.check()?;
        layout.block(block, &mut blocks)?;
    }
    let units = render_diagrams(layout.units, &budget)?;
    budget.check()?;
    Ok(DocumentLayout { units })
}
