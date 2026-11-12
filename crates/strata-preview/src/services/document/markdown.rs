// SPDX-License-Identifier: MIT
//! Markdown to document blocks, through pulldown-cmark's event stream.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use super::{
    ActiveBlock, Document, DocumentBlock, DocumentListChildKind, DocumentTableCell, ParseBudget,
    ParseLimits, ParsedDocument, append_escaped, append_markup, block_markup_bytes, code_language,
    finish_block, finish_parent_list_item, has_visible_markup, has_web_scheme, is_list_block,
    next_list_marker, push_table_row,
};
use crate::sandbox::Cancellation;

#[derive(Clone, Copy)]
pub(super) struct MarkdownProfile {
    /// Tables, math, block quotes and image loading; only file previews.
    document_features: bool,
    /// Drop HTML comments instead of showing them as inert text. GitHub's
    /// generated release notes start with one; previews keep them visible.
    hide_html_comments: bool,
}

impl MarkdownProfile {
    pub(super) const DOCUMENT_PREVIEW: Self = Self {
        document_features: true,
        hide_html_comments: false,
    };
    pub(super) const RELEASE_NOTES: Self = Self {
        document_features: false,
        hide_html_comments: true,
    };
}

fn is_html_comment(html: &str) -> bool {
    html.trim_start().starts_with("<!--")
}

fn markdown_code_language(kind: &CodeBlockKind<'_>) -> Option<&'static str> {
    match kind {
        CodeBlockKind::Indented => None,
        CodeBlockKind::Fenced(info) => info.split_ascii_whitespace().next().and_then(code_language),
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    level as u8
}

/// Only images inside the document's folder are loaded.
fn is_relative_image(destination: &str) -> bool {
    !destination.contains(':') && !destination.starts_with('/')
}

fn follows_table(depth: Option<usize>, previous: Option<&DocumentBlock>) -> bool {
    match (depth, previous) {
        (None, Some(DocumentBlock::TableRow { .. })) => true,
        (
            Some(depth),
            Some(DocumentBlock::ListTableRow {
                depth: previous_depth,
                ..
            }),
        ) => depth == *previous_depth,
        _ => false,
    }
}

fn ends_text_block(event: &Event<'_>) -> bool {
    matches!(
        event,
        Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item)
    )
}

struct MarkdownState<'a> {
    budget: ParseBudget<'a>,
    profile: MarkdownProfile,
    blocks: Vec<DocumentBlock>,
    active: Option<ActiveBlock>,
    links: Vec<bool>,
    lists: Vec<Option<u64>>,
    list_items: Vec<usize>,
    quote_depth: usize,
    table_header: bool,
    table_row: Option<Vec<DocumentTableCell>>,
    table_cell: Option<String>,
    table_depth: Option<usize>,
    raw_html: bool,
    hidden_comment_block: bool,
    blocked_images: bool,
    image: Option<(String, String)>,
    pending_images: Vec<DocumentBlock>,
    completed_markup: usize,
    completed_blocks: usize,
}

impl<'a> MarkdownState<'a> {
    fn new(budget: ParseBudget<'a>, profile: MarkdownProfile) -> Self {
        Self {
            budget,
            profile,
            blocks: Vec::new(),
            active: None,
            links: Vec::new(),
            lists: Vec::new(),
            list_items: Vec::new(),
            quote_depth: 0,
            table_header: false,
            table_row: None,
            table_cell: None,
            table_depth: None,
            raw_html: false,
            hidden_comment_block: false,
            blocked_images: false,
            image: None,
            pending_images: Vec::new(),
            completed_markup: 0,
            completed_blocks: 0,
        }
    }

    fn options(&self) -> Options {
        let mut options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
        if self.profile.document_features {
            options |= Options::ENABLE_TABLES | Options::ENABLE_MATH;
        }
        options
    }

    fn markup(&mut self, markup: &str) {
        append_markup(&mut self.active, &mut self.table_cell, markup);
    }

    fn escaped(&mut self, text: &str) {
        append_escaped(&mut self.active, &mut self.table_cell, text);
    }

    fn wrapped(&mut self, open: &str, text: &str, close: &str) {
        self.markup(open);
        self.escaped(text);
        self.markup(close);
    }

    fn finish_block(&mut self) {
        finish_block(&mut self.active, &mut self.blocks);
    }

    fn finish_parent_list_item(&mut self) {
        finish_parent_list_item(&mut self.active, &mut self.blocks);
    }

    /// A list child of `kind` inside a list item, or `standalone` outside lists.
    fn nested(&self, kind: DocumentListChildKind, standalone: ActiveBlock) -> ActiveBlock {
        match self.list_items.last() {
            Some(depth) => ActiveBlock::ListChild {
                depth: *depth,
                kind,
                markup: String::new(),
            },
            None => standalone,
        }
    }

    /// Whether `event` belongs to an HTML comment the profile hides.
    fn hides(&mut self, event: &Event<'_>, source: &str) -> bool {
        // A comment HTML block spans whole lines and also carries synthesised
        // indentation text, so every event up to its end is dropped.
        if self.hidden_comment_block {
            if matches!(event, Event::End(TagEnd::HtmlBlock)) {
                self.budget.leave();
                self.hidden_comment_block = false;
            }
            return true;
        }
        if !self.profile.hide_html_comments {
            return false;
        }
        match event {
            Event::Start(Tag::HtmlBlock) if is_html_comment(source) => {
                self.hidden_comment_block = true;
                true
            }
            Event::InlineHtml(text) => is_html_comment(text),
            _ => false,
        }
    }

    fn collect_image_alt(&mut self, event: &Event<'_>) {
        let Some((_, alt)) = self.image.as_mut() else {
            return;
        };
        match event {
            Event::Text(text) | Event::Code(text) => alt.push_str(text),
            Event::SoftBreak | Event::HardBreak => alt.push(' '),
            _ => {}
        }
    }

    fn handle(&mut self, event: &Event<'_>, source: &str) -> Result<(), String> {
        self.budget.event()?;
        if matches!(event, Event::Start(_)) {
            self.budget.enter()?;
        }
        if self.hides(event, source) {
            return Ok(());
        }
        self.collect_image_alt(event);
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(*tag),
            _ => self.inline(event),
        }
        if matches!(event, Event::End(_)) {
            self.budget.leave();
        }
        self.flush_pending_images(event);
        self.account()
    }

    fn start(&mut self, tag: &Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => self.start_heading(heading_level(*level)),
            Tag::Paragraph => self.start_paragraph(),
            Tag::BlockQuote(_) => self.start_quote(),
            Tag::List(start) => self.start_list(*start),
            Tag::Item => self.start_item(),
            Tag::Table(_) => self.start_table(),
            Tag::TableHead => {
                self.table_header = true;
                self.table_row = Some(Vec::new());
            }
            Tag::TableRow => self.table_row = Some(Vec::new()),
            Tag::TableCell => self.table_cell = Some(String::new()),
            Tag::Emphasis => self.markup("<i>"),
            Tag::Strong => self.markup("<b>"),
            Tag::Strikethrough => self.markup("<s>"),
            Tag::Link { dest_url, .. } => self.start_link(dest_url),
            Tag::Image { dest_url, .. } => self.start_image(dest_url),
            Tag::CodeBlock(kind) => self.start_code(markdown_code_language(kind)),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => self.finish_block(),
            TagEnd::Paragraph => self.end_paragraph(),
            TagEnd::BlockQuote(_) => self.end_quote(),
            TagEnd::List(_) => {
                self.finish_list_block();
                self.lists.pop();
            }
            TagEnd::Item => {
                self.finish_list_block();
                self.list_items.pop();
            }
            TagEnd::Table => {
                self.finish_block();
                self.table_depth = None;
            }
            TagEnd::TableHead => {
                self.push_table_row();
                self.table_header = false;
            }
            TagEnd::TableRow => self.push_table_row(),
            TagEnd::TableCell => self.end_cell(),
            TagEnd::Emphasis => self.markup("</i>"),
            TagEnd::Strong => self.markup("</b>"),
            TagEnd::Strikethrough => self.markup("</s>"),
            TagEnd::Link => {
                let close = if self.links.pop().unwrap_or(false) {
                    "</a>"
                } else {
                    "</u>"
                };
                self.markup(close);
            }
            TagEnd::Image => self.end_image(),
            TagEnd::CodeBlock => self.end_code(),
            _ => {}
        }
    }

    /// Whether math renders as an equation here rather than as inert source.
    fn renders_math(&self) -> bool {
        self.table_cell.is_none() && self.image.is_none()
    }

    fn inline(&mut self, event: &Event<'_>) {
        match event {
            Event::InlineMath(source)
                if self.renders_math()
                    && source.len() <= crate::services::document_media::MATH_INPUT_LIMIT =>
            {
                self.wrapped("<span font_family=\"strata-math\">", source, "</span>");
            }
            Event::DisplayMath(source) if self.renders_math() => self.display_math(source),
            Event::InlineMath(source) | Event::DisplayMath(source) => {
                self.wrapped("<tt>$", source, "$</tt>");
            }
            Event::Code(text) => self.wrapped("<tt>", text, "</tt>"),
            Event::Text(text) => self.escaped(text),
            Event::SoftBreak | Event::HardBreak => self.markup("\n"),
            Event::Rule => self.rule(),
            Event::Html(text) | Event::InlineHtml(text) => {
                self.raw_html = true;
                self.escaped(text);
            }
            Event::TaskListMarker(checked) => self.markup(if *checked { "☑ " } else { "☐ " }),
            _ => {}
        }
    }

    fn start_heading(&mut self, level: u8) {
        self.finish_parent_list_item();
        let heading = ActiveBlock::Heading {
            level,
            markup: String::new(),
        };
        self.active = Some(self.nested(DocumentListChildKind::Heading(level), heading));
    }

    fn continues_list_paragraph(&self) -> bool {
        self.active.as_ref().is_some_and(|active| {
            matches!(
                active,
                ActiveBlock::ListItem { .. }
                    | ActiveBlock::ListChild {
                        kind: DocumentListChildKind::Paragraph,
                        ..
                    }
            ) && has_visible_markup(active.markup())
        })
    }

    fn start_paragraph(&mut self) {
        if self.continues_list_paragraph() {
            self.finish_block();
        }
        if self.active.is_some() {
            return;
        }
        let quoted = self.quote_depth > 0;
        let (kind, standalone) = if quoted {
            (
                DocumentListChildKind::Quote,
                ActiveBlock::Quote(String::new()),
            )
        } else {
            (
                DocumentListChildKind::Paragraph,
                ActiveBlock::Paragraph(String::new()),
            )
        };
        self.active = Some(self.nested(kind, standalone));
    }

    fn end_paragraph(&mut self) {
        if matches!(
            self.active,
            Some(
                ActiveBlock::Paragraph(_)
                    | ActiveBlock::Quote(_)
                    | ActiveBlock::ListChild {
                        kind: DocumentListChildKind::Paragraph | DocumentListChildKind::Quote,
                        ..
                    }
            )
        ) {
            self.finish_block();
        }
    }

    fn start_quote(&mut self) {
        if !self.profile.document_features {
            return;
        }
        self.finish_parent_list_item();
        self.quote_depth = self.quote_depth.saturating_add(1);
        self.active = Some(self.nested(
            DocumentListChildKind::Quote,
            ActiveBlock::Quote(String::new()),
        ));
    }

    fn end_quote(&mut self) {
        if self.profile.document_features {
            self.finish_block();
            self.quote_depth = self.quote_depth.saturating_sub(1);
        }
    }

    fn start_list(&mut self, start: Option<u64>) {
        self.finish_parent_list_item();
        if self.lists.is_empty() && self.blocks.last().is_some_and(is_list_block) {
            self.blocks.push(DocumentBlock::ContainerBoundary);
        }
        self.lists.push(start);
    }

    fn finish_list_block(&mut self) {
        if matches!(
            self.active,
            Some(ActiveBlock::ListItem { .. } | ActiveBlock::ListChild { .. })
        ) {
            self.finish_block();
        }
    }

    fn start_item(&mut self) {
        self.finish_block();
        let depth = self.lists.len().saturating_sub(1);
        self.list_items.push(depth);
        self.active = Some(ActiveBlock::ListItem {
            marker: next_list_marker(&mut self.lists),
            depth,
            markup: String::new(),
        });
    }

    fn start_table(&mut self) {
        self.finish_parent_list_item();
        self.table_depth = self.list_items.last().copied();
        if follows_table(self.table_depth, self.blocks.last()) {
            self.blocks.push(DocumentBlock::ContainerBoundary);
        }
    }

    fn push_table_row(&mut self) {
        if let Some(cells) = self.table_row.take() {
            push_table_row(&mut self.blocks, self.table_depth, cells);
        }
    }

    fn end_cell(&mut self) {
        let Some(cell) = self.table_cell.take() else {
            return;
        };
        self.table_row
            .get_or_insert_with(Vec::new)
            .push(DocumentTableCell {
                header: self.table_header,
                markup: cell,
            });
    }

    fn start_link(&mut self, destination: &str) {
        let external = has_web_scheme(destination);
        self.links.push(external);
        if external {
            self.wrapped("<a href=\"", destination, "\">");
        } else {
            self.markup("<u>");
        }
    }

    fn start_image(&mut self, destination: &str) {
        if self.profile.document_features && self.table_cell.is_none() {
            if is_relative_image(destination) {
                self.image = Some((destination.to_owned(), String::new()));
            } else {
                self.blocked_images = true;
            }
        }
        self.markup("[Image: ");
    }

    fn end_image(&mut self) {
        self.markup("]");
        if let Some((destination, alt)) = self.image.take() {
            self.pending_images.push(DocumentBlock::Image {
                destination,
                alt,
                list_depth: self.list_items.last().copied(),
            });
        }
    }

    fn in_quote(&self) -> bool {
        matches!(
            self.active,
            Some(
                ActiveBlock::Quote(_)
                    | ActiveBlock::ListChild {
                        kind: DocumentListChildKind::Quote,
                        ..
                    }
            )
        )
    }

    fn start_code(&mut self, language: Option<&'static str>) {
        if matches!(language, Some("mermaid" | "latex")) {
            self.finish_block();
        }
        if self.in_quote() {
            self.markup("<tt>");
            return;
        }
        self.finish_parent_list_item();
        let code = ActiveBlock::Code {
            markup: String::new(),
            language,
        };
        self.active = Some(self.nested(DocumentListChildKind::Code(language), code));
    }

    fn end_code(&mut self) {
        if matches!(
            self.active,
            Some(
                ActiveBlock::Code { .. }
                    | ActiveBlock::ListChild {
                        kind: DocumentListChildKind::Code(_),
                        ..
                    }
            )
        ) {
            self.finish_block();
        } else {
            self.markup("</tt>");
        }
    }

    fn display_math(&mut self, source: &str) {
        self.finish_block();
        let markup = glib::markup_escape_text(source).to_string();
        let block = match self.list_items.last() {
            Some(depth) => DocumentBlock::ListChild {
                depth: *depth,
                kind: DocumentListChildKind::Code(Some("latex")),
                markup,
            },
            None => DocumentBlock::Code {
                markup,
                language: Some("latex"),
            },
        };
        self.blocks.push(block);
    }

    fn rule(&mut self) {
        self.finish_parent_list_item();
        let rule = match self.list_items.last().copied() {
            Some(depth) => DocumentBlock::ListRule { depth },
            None => DocumentBlock::Rule,
        };
        self.blocks.push(rule);
    }

    /// Whether the only pending image already stands alone as its `[Image: alt]` paragraph.
    fn image_placeholder_precedes(&self) -> bool {
        let [DocumentBlock::Image { alt, .. }] = self.pending_images.as_slice() else {
            return false;
        };
        matches!(
            self.blocks.last(),
            Some(DocumentBlock::Paragraph(markup))
                if *markup == format!("[Image: {}]", glib::markup_escape_text(alt))
        )
    }

    fn flush_pending_images(&mut self, event: &Event<'_>) {
        if self.pending_images.is_empty() {
            return;
        }
        let ends_block = ends_text_block(event);
        if !ends_block && self.blocks.len() <= self.completed_blocks {
            return;
        }
        if ends_block {
            self.finish_block();
        }
        if self.image_placeholder_precedes() {
            self.blocks.pop();
        }
        self.blocks.append(&mut self.pending_images);
    }

    fn pending_markup(&self) -> usize {
        let images = self
            .pending_images
            .iter()
            .map(block_markup_bytes)
            .sum::<usize>();
        let row = self.table_row.as_ref().map_or(0, |row| {
            row.iter().map(|cell| cell.markup.len()).sum::<usize>()
        });
        images
            + self.active.as_ref().map_or(0, |block| block.markup().len())
            + self.table_cell.as_ref().map_or(0, String::len)
            + row
    }

    fn account(&mut self) -> Result<(), String> {
        let completed = self.completed_blocks.min(self.blocks.len());
        self.completed_markup += self.blocks[completed..]
            .iter()
            .map(block_markup_bytes)
            .sum::<usize>();
        self.completed_blocks = self.blocks.len();
        let limits: ParseLimits = self.budget.limits;
        if self
            .table_row
            .as_ref()
            .is_some_and(|row| row.len() > limits.table_columns)
        {
            return Err("Rendered preview exceeded the column budget for one table".to_owned());
        }
        if self.completed_markup.saturating_add(self.pending_markup()) > limits.markup {
            return Err("Rendered preview exceeded the 4 MB markup limit".to_owned());
        }
        Ok(())
    }

    fn finish(mut self) -> ParsedDocument {
        self.finish_block();
        ParsedDocument {
            document: Document {
                blocks: self.blocks,
            },
            warnings: [
                self.raw_html
                    .then(|| "Raw HTML is shown as inert text in Markdown previews.".to_owned()),
                self.blocked_images.then(|| "Remote and absolute image URLs are not loaded. Use images inside the document's folder.".to_owned()),
            ]
            .into_iter()
            .flatten()
            .collect(),
        }
    }
}

pub(super) fn parse_markdown_bounded(
    markdown: &str,
    cancellation: &Cancellation,
    limits: ParseLimits,
    profile: MarkdownProfile,
) -> Result<ParsedDocument, String> {
    let budget = ParseBudget::new(cancellation, limits);
    budget.check()?;
    let mut state = MarkdownState::new(budget, profile);
    for (event, range) in Parser::new_ext(markdown, state.options()).into_offset_iter() {
        state.handle(&event, &markdown[range])?;
    }
    Ok(state.finish())
}
