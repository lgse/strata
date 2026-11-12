// SPDX-License-Identifier: MIT
//! HTML (and RTF converted to HTML) to document blocks, keeping only inert structure.

use html5gum::{DefaultEmitter, Token, Tokenizer};

use super::{
    ActiveBlock, Document, DocumentBlock, DocumentListChildKind, DocumentTableCell, ParseBudget,
    ParseLimits, ParsedDocument, append_markup, code_language, escape_document_text, finish_block,
    finish_parent_list_item, has_visible_markup, has_web_scheme, is_list_block, next_list_marker,
    push_table_row,
};
use crate::sandbox::Cancellation;

fn html_code_language(classes: &str) -> Option<&'static str> {
    classes.split_ascii_whitespace().find_map(|class| {
        class
            .to_ascii_lowercase()
            .strip_prefix("language-")
            .and_then(code_language)
    })
}

#[derive(Clone)]
enum HtmlLink {
    External(String),
    Underline,
    Inert,
}

/// One opening tag with the only attributes previews honour.
struct StartTag<'a> {
    name: &'a str,
    href: Option<&'a str>,
    language: Option<&'static str>,
    self_closing: bool,
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

    /// Skips the whole subtree of an omitted element; true while inside one.
    fn skip(&mut self, name: &str, void: bool) -> bool {
        if self.skipped_depth > 0 {
            if !void {
                self.stack.push(name.to_owned());
                self.skipped_depth = self.skipped_depth.saturating_add(1);
            }
            return true;
        }
        self.close_implied_before_start(name);
        if !is_omitted_html_tag(name) {
            return false;
        }
        self.warned = true;
        if !void {
            self.stack.push(name.to_owned());
            self.skipped_depth = 1;
        }
        true
    }

    fn start(&mut self, tag: StartTag<'_>) {
        let void = is_void_html_tag(tag.name) || tag.self_closing;
        if self.skip(tag.name, void) {
            return;
        }
        self.start_supported(&tag);
        if !void {
            self.stack.push(tag.name.to_owned());
        } else if tag.self_closing {
            self.end_supported(tag.name);
        }
    }

    fn start_supported(&mut self, tag: &StartTag<'_>) {
        let name = tag.name;
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.start_heading(name[1..].parse().unwrap_or(6));
            }
            "p" => self.start_paragraph(),
            "blockquote" => self.start_quote(),
            "ul" | "ol" => self.start_list(name == "ol"),
            "li" => self.start_item(),
            "em" | "i" => self.start_inline("<i>"),
            "strong" | "b" => self.start_inline("<b>"),
            "s" | "del" => self.start_inline("<s>"),
            "u" | "ins" => self.start_inline("<u>"),
            "a" => self.start_link(tag.href),
            "pre" => self.start_preformatted(),
            "code" if !self.preformatted => {
                self.start_inline("<tt>");
            }
            "code" => self.set_code_language(tag.language),
            "hr" => self.rule(),
            "br" => self.line_break(),
            "table" => self.start_table(),
            "tr" => self.table_row = Some(Vec::new()),
            "th" | "td" => self.start_cell(name == "th"),
            "main" | "article" | "section" | "header" | "footer" | "nav" | "aside" | "div" => {
                self.finish_list_parent();
            }
            "html" | "body" | "span" | "thead" | "tbody" | "tfoot" => {}
            _ => self.warned = true,
        }
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

    fn start_heading(&mut self, level: u8) {
        self.finish_list_parent();
        let heading = ActiveBlock::Heading {
            level,
            markup: String::new(),
        };
        self.start_active(self.nested(DocumentListChildKind::Heading(level), heading));
    }

    fn quoted_kind(&self) -> DocumentListChildKind {
        if self.quote_depth > 0 {
            DocumentListChildKind::Quote
        } else {
            DocumentListChildKind::Paragraph
        }
    }

    fn standalone_paragraph(&self) -> ActiveBlock {
        if self.quote_depth > 0 {
            ActiveBlock::Quote(String::new())
        } else {
            ActiveBlock::Paragraph(String::new())
        }
    }

    /// Whether a new `kind` paragraph ends the visible list content already open.
    fn closes_list_content(&self, kind: DocumentListChildKind) -> bool {
        self.active.as_ref().is_some_and(|active| {
            let continues = matches!(active, ActiveBlock::ListItem { .. })
                || matches!(
                    active,
                    ActiveBlock::ListChild {
                        kind: active_kind,
                        ..
                    } if *active_kind == kind
                );
            continues && has_visible_markup(active.markup())
        })
    }

    fn start_paragraph(&mut self) {
        let Some(depth) = self.list_items.last().copied() else {
            self.finish_active();
            self.start_active(self.standalone_paragraph());
            return;
        };
        let kind = self.quoted_kind();
        if self.closes_list_content(kind) {
            self.finish_active();
        }
        if self.active.is_none() {
            self.start_active(ActiveBlock::ListChild {
                depth,
                kind,
                markup: String::new(),
            });
        }
    }

    fn start_quote(&mut self) {
        self.finish_list_parent();
        self.quote_depth = self.quote_depth.saturating_add(1);
        self.start_active(self.nested(
            DocumentListChildKind::Quote,
            ActiveBlock::Quote(String::new()),
        ));
    }

    fn start_list(&mut self, ordered: bool) {
        self.finish_list_parent();
        if self.lists.is_empty() && self.blocks.last().is_some_and(is_list_block) {
            self.blocks.push(DocumentBlock::ContainerBoundary);
        }
        self.lists.push(ordered.then_some(1));
    }

    fn start_item(&mut self) {
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

    fn link_for(&mut self, href: Option<&str>) -> HtmlLink {
        if !self.links.is_empty() {
            self.warned = true;
            return HtmlLink::Inert;
        }
        match href {
            Some(href) if has_web_scheme(href) => HtmlLink::External(href.to_owned()),
            Some(_) => {
                self.warned = true;
                HtmlLink::Underline
            }
            None => HtmlLink::Underline,
        }
    }

    fn start_link(&mut self, href: Option<&str>) {
        self.ensure_text_target();
        self.flush_pending_space();
        let link = self.link_for(href);
        self.append_link_open(&link);
        self.links.push(link);
    }

    fn start_preformatted(&mut self) {
        self.finish_list_parent();
        let code = ActiveBlock::Code {
            markup: String::new(),
            language: None,
        };
        self.start_active(self.nested(DocumentListChildKind::Code(None), code));
        self.preformatted = true;
    }

    fn rule(&mut self) {
        self.finish_list_parent();
        let rule = match self.list_items.last().copied() {
            Some(depth) => DocumentBlock::ListRule { depth },
            None => DocumentBlock::Rule,
        };
        self.blocks.push(rule);
    }

    fn line_break(&mut self) {
        self.ensure_text_target();
        self.pending_space = false;
        self.at_line_start = true;
        self.append_markup("\n");
    }

    fn start_table(&mut self) {
        self.finish_list_parent();
        self.table_depth = self.list_items.last().copied();
        if follows_table(self.table_depth, self.blocks.last()) {
            self.blocks.push(DocumentBlock::ContainerBoundary);
        }
    }

    fn start_cell(&mut self, header: bool) {
        self.table_cell_header = header;
        self.table_cell = Some(String::new());
        self.pending_space = false;
        self.at_line_start = true;
        self.append_link_openings();
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

    fn ends_paragraph(&self) -> bool {
        matches!(
            self.active,
            Some(
                ActiveBlock::Paragraph(_)
                    | ActiveBlock::Quote(_)
                    | ActiveBlock::ListChild {
                        kind: DocumentListChildKind::Paragraph | DocumentListChildKind::Quote,
                        ..
                    }
            )
        )
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
            "p" if self.ends_paragraph() => {
                self.finish_active();
            }
            "p" => {}
            "blockquote" => {
                self.finish_active();
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            "ul" | "ol" => self.end_list(),
            "em" | "i" => self.append_markup("</i>"),
            "strong" | "b" => self.append_markup("</b>"),
            "s" | "del" => self.append_markup("</s>"),
            "u" | "ins" => self.append_markup("</u>"),
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
            "th" | "td" => self.end_cell(),
            "main" | "article" | "section" | "header" | "footer" | "nav" | "aside" | "div" => {
                self.finish_active();
            }
            _ => {}
        }
    }

    fn end_list(&mut self) {
        if matches!(
            self.active,
            Some(ActiveBlock::ListItem { .. } | ActiveBlock::ListChild { .. })
        ) {
            self.finish_active();
        }
        self.lists.pop();
    }

    fn end_cell(&mut self) {
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

    /// Whitespace between blocks has no text target to land in.
    fn is_interblock_whitespace(&self, text: &str) -> bool {
        let detached = self.active.is_none() && self.table_cell.is_none();
        detached && text.chars().all(is_html_whitespace)
    }

    fn collapse_whitespace(&mut self, text: &str) -> String {
        let mut collapsed = String::with_capacity(text.len());
        for character in text.chars() {
            if !is_html_whitespace(character) {
                if self.pending_space {
                    collapsed.push(' ');
                    self.pending_space = false;
                }
                collapsed.push(character);
                self.at_line_start = false;
            } else if !self.at_line_start {
                self.pending_space = true;
            }
        }
        collapsed
    }

    fn text(&mut self, text: &str) {
        if self.skipped_depth > 0 || self.is_interblock_whitespace(text) {
            return;
        }
        self.ensure_text_target();
        if self.preformatted {
            self.append_escaped(text);
            return;
        }
        let collapsed = self.collapse_whitespace(text);
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
            self.start_active(self.nested(self.quoted_kind(), self.standalone_paragraph()));
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
        if self.active.is_none() && self.table_cell.is_none() {
            return;
        }
        match link {
            HtmlLink::External(_) => self.append_markup("</a>"),
            HtmlLink::Underline => self.append_markup("</u>"),
            HtmlLink::Inert => {}
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

    /// Reads the `href` of links and the language class of code; other attributes warn.
    fn honoured_attributes(
        &mut self,
        name: &str,
        tag: &html5gum::StartTag<()>,
    ) -> (Option<String>, Option<&'static str>) {
        let mut href = None;
        let mut language = None;
        for (attribute, value) in &tag.attributes {
            let attribute = String::from_utf8_lossy(attribute).to_ascii_lowercase();
            match (name, attribute.as_str()) {
                ("a", "href") => href = Some(String::from_utf8_lossy(&value.value).into_owned()),
                ("code", "class") => {
                    language = html_code_language(&String::from_utf8_lossy(&value.value));
                    self.warned |= language.is_none();
                }
                _ => self.warned = true,
            }
        }
        (href, language)
    }

    fn start_token(
        &mut self,
        tag: &html5gum::StartTag<()>,
        budget: &mut ParseBudget<'_>,
    ) -> Result<(), String> {
        let name = String::from_utf8_lossy(&tag.name).to_ascii_lowercase();
        let (href, language) = self.honoured_attributes(&name, tag);
        self.start(StartTag {
            name: &name,
            href: href.as_deref(),
            language,
            self_closing: tag.self_closing,
        });
        self.check_markup()?;
        budget.nesting(self.stack.len())
    }

    fn token(&mut self, token: Token, budget: &mut ParseBudget<'_>) -> Result<(), String> {
        match token {
            Token::StartTag(tag) => self.start_token(&tag, budget)?,
            Token::EndTag(tag) => {
                let name = String::from_utf8_lossy(&tag.name).to_ascii_lowercase();
                self.end(&name);
                self.check_markup()?;
                budget.nesting(self.stack.len())?;
            }
            Token::String(text) => {
                self.text(&String::from_utf8_lossy(&text.value));
                self.check_markup()?;
            }
            Token::Error(_) => self.malformed = true,
            Token::Comment(_) | Token::Doctype(_) => {}
        }
        Ok(())
    }

    fn into_parsed(self) -> Result<ParsedDocument, String> {
        let state = self.finish();
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
}

pub(super) fn parse_html_bounded(
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
        state.token(token, &mut budget)?;
    }
    state.into_parsed()
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
