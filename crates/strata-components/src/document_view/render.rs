//! Rows: one per unit, built by index inside Rune's virtual list.

use std::ops::Range;
use std::rc::Rc;

use rune::{Color, El, Icon, Link, LinkPaint, Span as RSpan, TextStyle, col, el, icon, row, text};
use rune_kit::syntax::Paint as SyntaxPaint;
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::DocumentIntent;
use super::lang::grammar_token;
use super::model::{Cell, ListChildKind, Media, Span, SpanStyle, Unit, UnitKind};
use super::state::{BUTTON_ROOM, DocumentState, MediaState, has_copy_button};
use crate::code_view::highlight::CodeBlocks;
use crate::preview_drawer::CONTENT_MAX_WIDTH;

/// Heading sizes against the body text, levels 1 to 6: Strata's `document-heading-*` tags
/// (`src/ui/virtual_preview/tags.rs`), weight 700. Component constants for now; a `document`
/// slot is the candidate home for them once one is needed.
pub const HEADING_SCALE: [f32; 6] = [1.6, 1.35, 1.18, 1.04, 1.04, 1.04];

/// Strata's row geometry (`src/ui/virtual_preview/rows.rs`), in points: the side margins, a
/// list level's indent, the space after a block and after a list block, the first row's top
/// and the last row's bottom, and a gap unit's height.
const SIDE: f32 = 16.0;
const LIST_FIRST: f32 = 20.0;
const LIST_STEP: f32 = 18.0;
const BLOCK_GAP: f32 = 10.0;
const LIST_GAP: f32 = 4.0;
const DOC_TOP: f32 = 12.0;
const DOC_BOTTOM: f32 = 20.0;
const GAP: f32 = 10.0;
/// Code text starts this far right of the text column (Strata's `CODE_HORIZONTAL_PADDING`).
const CODE_PAD: f32 = 10.0;
/// A quote's bar, and the gap between it and the text.
const QUOTE_BAR: f32 = 3.0;
/// A table cell's widest estimate before it wraps, and its narrowest.
const CELL_MAX: f32 = 320.0;
const CELL_MIN: f32 = 48.0;
/// The height a loading image holds.
const LOADING_HEIGHT: f32 = 120.0;

/// Strata's list indent for a block at `depth`: none outside a list.
fn list_indent(depth: Option<usize>) -> f32 {
    depth.map_or(0.0, |d| LIST_FIRST + d.min(32) as f32 * LIST_STEP)
}

/// The look a row reads, resolved once per build: `Copy` tokens only, so the row closures
/// capture no `Look`.
#[derive(Clone, Copy)]
pub(crate) struct Paint {
    pub body: TextStyle,
    pub mono: TextStyle,
    pub small: TextStyle,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub link_line: Color,
    pub code_fill: Color,
    pub code_border: Color,
    pub quote_rule: Color,
    pub rule: Color,
    pub selection: Color,
    pub head_fill: Color,
    pub table_rule: Color,
    pub placeholder: Color,
    pub hover: Color,
    pub radius: f32,
    pub icon: f32,
    pub copy: &'static Icon,
    pub check: &'static Icon,
}

impl Paint {
    pub fn from_look(look: &Look) -> Paint {
        let k = &look.kit;
        let c = &k.c;
        Paint {
            body: k.ty.body,
            mono: TextStyle {
                size: k.ty.body.size,
                ..k.ty.mono
            },
            small: k.ty.small,
            text: c.text_base,
            muted: c.text_muted,
            accent: c.text_accent,
            link_line: k.alpha(c.text_accent, 0.4),
            code_fill: c.layer_01,
            code_border: c.border_muted,
            quote_rule: c.layer_03,
            rule: c.border_base,
            selection: k.selection(),
            head_fill: look.fill.muted,
            table_rule: c.border_muted,
            placeholder: c.layer_02,
            hover: c.hover,
            radius: look.round(k.radius.sm),
            icon: look.icon_size.sm,
            copy: look.icon(IconName::Copy),
            check: look.icon(IconName::Check),
        }
    }

    fn heading(&self, level: u8) -> TextStyle {
        let size = self.body.size * HEADING_SCALE[usize::from(level.clamp(1, 6)) - 1];
        TextStyle {
            size,
            weight: 700.0,
            ..self.body
        }
        .lh((size * 1.35).round())
    }
}

/// The words the view shows and assistive technology reads; Strata passes translated text.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DocumentLabels {
    /// The document's accessible name.
    pub document: String,
    pub copy_code: String,
    pub code_copied: String,
    pub copy_diagram: String,
    pub copy_equation: String,
    pub loading_image: String,
}

impl Default for DocumentLabels {
    fn default() -> Self {
        DocumentLabels {
            document: "Document".into(),
            copy_code: "Copy code".into(),
            code_copied: "Code copied".into(),
            copy_diagram: "Copy diagram source".into(),
            copy_equation: "Copy equation source".into(),
            loading_image: "Loading image".into(),
        }
    }
}

pub(crate) type IntentFn<S> = Rc<dyn Fn(&mut S, DocumentIntent, &mut rune::Tx)>;

/// What every row's build shares.
pub(crate) struct Ctx<S> {
    pub get: fn(&S) -> &DocumentState,
    pub get_mut: fn(&mut S) -> &mut DocumentState,
    pub paint: Paint,
    pub labels: Rc<DocumentLabels>,
    pub blocks: Option<CodeBlocks>,
    pub intent: Option<IntentFn<S>>,
    /// The viewport's width at the last layout, which tables fit their columns to.
    pub width: Option<f32>,
}

/// The rows around the visible ones that ask for code colours: the viewport and two of its
/// heights either side, Rune's mounted window. Rows outside it (built for a copy, or not yet
/// visible) draw plain, so a copy over the whole document never floods the highlighter.
pub(crate) fn near(visible: &Range<usize>, i: usize) -> bool {
    let reach = (visible.len() * 2).max(8);
    i + reach >= visible.start && i < visible.end + reach
}

pub(crate) fn build_row<S: 'static>(ctx: &Rc<Ctx<S>>, s: &S, i: usize) -> El {
    let st = (ctx.get)(s);
    let units = &st.document().units;
    let u = &units[i];
    let p = &ctx.paint;
    let last_row = i + 1 == units.len();
    let top = if i == 0 { DOC_TOP } else { 0.0 };
    let bottom = if last_row {
        DOC_BOTTOM
    } else if !u.last {
        0.0
    } else if list_block(&u.kind) {
        LIST_GAP
    } else {
        BLOCK_GAP
    };
    let (indent, body) = match &u.kind {
        UnitKind::Gap => return el().w_full().h(GAP).a11y_hidden(),
        UnitKind::Table { rows, list_depth } => {
            let indent = list_indent(*list_depth);
            let room = ctx
                .width
                .map(|w| w.min(CONTENT_MAX_WIDTH) - 2.0 * SIDE - indent);
            (indent, table(ctx, i, rows, room))
        }
        UnitKind::Media { media, list_depth } => {
            (list_indent(*list_depth), media_el(ctx, st, i, u, media))
        }
        UnitKind::Rule { list_depth } => (
            list_indent(*list_depth),
            el().w_full()
                .h(1.0)
                .fill(p.rule)
                .my(p.body.line_px() / 2.0)
                .role("separator"),
        ),
        UnitKind::Code {
            list_depth,
            language,
        } => (
            list_indent(*list_depth),
            code(ctx, st, i, u, language.as_deref()),
        ),
        kind => {
            let (indent, style, color, role, level) = match kind {
                UnitKind::Heading(l) => (0.0, p.heading(*l), p.text, "heading", Some(*l)),
                UnitKind::ListItem { depth } => (
                    list_indent(Some(*depth)),
                    p.body,
                    p.text,
                    "listitem",
                    Some(u8::try_from(*depth + 1).unwrap_or(u8::MAX)),
                ),
                UnitKind::ListChild { depth, kind } => {
                    let (style, color) = match kind {
                        ListChildKind::Heading(l) => (p.heading(*l), p.text),
                        ListChildKind::Quote => (p.body, p.muted),
                        ListChildKind::Code(_) => (p.mono, p.text),
                        ListChildKind::Paragraph => (p.body, p.text),
                    };
                    (list_indent(Some(*depth)), style, color, "paragraph", None)
                }
                UnitKind::Quote => (0.0, p.body, p.muted, "blockquote", None),
                _ => (0.0, p.body, p.text, "paragraph", None),
            };
            let t = unit_text(ctx, st, i, style, color, st.wraps());
            let t = match kind {
                UnitKind::Quote => row()
                    .w_full()
                    .gap(CODE_PAD)
                    .child(el().w(QUOTE_BAR).h_full().fill(p.quote_rule).radius(1.0))
                    .child(t),
                _ => t,
            };
            let mut b = col().w_full().role(role).child(t);
            if let Some(l) = level {
                b = b.level(u32::from(l));
            }
            (indent, b)
        }
    };
    col().w_full().items_center().pt(top).pb(bottom).child(
        col()
            .w_full()
            .max_w(CONTENT_MAX_WIDTH)
            .pl(SIDE + indent)
            .pr(SIDE)
            .child(body),
    )
}

fn list_block(kind: &UnitKind) -> bool {
    matches!(
        kind,
        UnitKind::ListItem { .. }
            | UnitKind::ListChild { .. }
            | UnitKind::Code {
                list_depth: Some(_),
                ..
            }
            | UnitKind::Rule {
                list_depth: Some(_)
            }
            | UnitKind::Table {
                list_depth: Some(_),
                ..
            }
    )
}

/// The model's spans as Rune's: colour, weight, face and underline runs (split where they
/// overlap, so each byte has every style over it), strikes and links.
pub(crate) struct Styled {
    pub spans: Vec<RSpan>,
    pub strikes: Vec<(usize, usize, Option<Color>)>,
    pub links: Vec<(Range<usize>, std::sync::Arc<str>)>,
}

pub(crate) fn styled(spans: &[Span], p: &Paint) -> Styled {
    let mut cuts: Vec<usize> = spans
        .iter()
        .flat_map(|s| [s.range.start, s.range.end])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let over: Vec<&SpanStyle> = spans
            .iter()
            .filter(|s| s.range.start <= a && b <= s.range.end)
            .map(|s| &s.style)
            .collect();
        if over.is_empty() || a == b {
            continue;
        }
        let mut sp = RSpan::new(a, b);
        let mut styled = false;
        for st in over {
            match st {
                SpanStyle::Accent => sp = sp.color(p.accent),
                SpanStyle::Bold => sp = sp.weight(700.0),
                SpanStyle::Italic => sp = sp.italic(),
                SpanStyle::Mono | SpanStyle::Math(_) => sp = sp.mono(),
                SpanStyle::Underline => sp = sp.underline(p.text),
                SpanStyle::Link(_) => sp = sp.color(p.accent).underline(p.link_line),
                SpanStyle::Strike => continue,
            }
            styled = true;
        }
        if styled {
            out.push(sp);
        }
    }
    Styled {
        spans: out,
        strikes: spans
            .iter()
            .filter(|s| s.style == SpanStyle::Strike)
            .map(|s| (s.range.start, s.range.end, None))
            .collect(),
        links: spans
            .iter()
            .filter_map(|s| match &s.style {
                SpanStyle::Link(uri) => Some((s.range.clone(), uri.clone())),
                _ => None,
            })
            .collect(),
    }
}

/// A selectable text with `styled` on it, wrapping or scrolling sideways on its own (D7),
/// keyed `key`, with its links opening through the host.
fn styled_text<S: 'static>(
    ctx: &Rc<Ctx<S>>,
    content: String,
    st: Styled,
    (style, color): (TextStyle, Color),
    key: &str,
    scope: String,
    wrap: bool,
) -> El {
    let p = &ctx.paint;
    let mut t = text(content)
        .spans(st.spans)
        .strikes(st.strikes)
        .key(key.to_owned())
        .font(style)
        .color(color)
        .selectable(true)
        .selection_color(p.selection);
    // A link's id keys its element: `link-{n}` within its text, as a URI's slashes would
    // read as a key path.
    let uris: Vec<std::sync::Arc<str>> = st.links.iter().map(|(_, u)| u.clone()).collect();
    for (n, (r, uri)) in st.links.into_iter().enumerate() {
        t = t.link(
            Link::new(r, format!("link-{n}"))
                .hover(LinkPaint::new().underline(p.accent))
                .help(uri.to_string()),
        );
    }
    if !uris.is_empty()
        && let Some(f) = ctx.intent.clone()
    {
        let get_mut = ctx.get_mut;
        t = t.on_link(move |s: &mut S, id: &str, tx| {
            // Strata opens a link on a plain single click only: a Shift-click extends the
            // selection, and a double-click's second click doesn't open it again.
            if tx.mods().shift {
                return;
            }
            let n: Option<usize> = id.strip_prefix("link-").and_then(|n| n.parse().ok());
            let Some((n, uri)) = n.and_then(|n| Some((n, uris.get(n)?.clone()))) else {
                return;
            };
            if get_mut(s).link_pressed(format!("{scope}#{n}")) {
                f(s, DocumentIntent::OpenLink(uri), tx);
            }
        });
    }
    if wrap {
        t.wrap().w_full()
    } else {
        // Keyed so a script can tell an unwrapped row from a wrapped one.
        col().key("wide").w_full().scroll_x().child(t)
    }
}

fn unit_text<S: 'static>(
    ctx: &Rc<Ctx<S>>,
    st: &DocumentState,
    i: usize,
    style: TextStyle,
    color: Color,
    wrap: bool,
) -> El {
    let (shown, spans) = st.shown(i);
    let styled = styled(spans, &ctx.paint);
    styled_text(
        ctx,
        shown.to_owned(),
        styled,
        (style, color),
        "t",
        format!("{i}/t"),
        wrap,
    )
}

/// A code block's row: its panel (rounded where the block starts and ends), its text
/// coloured by C10a when the block is one unit and near the viewport, and on its first row,
/// Copy code.
fn code<S: 'static>(
    ctx: &Rc<Ctx<S>>,
    st: &DocumentState,
    i: usize,
    u: &Unit,
    language: Option<&str>,
) -> El {
    let p = &ctx.paint;
    let (shown, _) = st.shown(i);
    let mut spans: Vec<RSpan> = Vec::new();
    // Strata colours a block only when it is one unit: one split over 32 KiB, or around a
    // line over 2 KiB, draws plain.
    if u.first
        && u.last
        && near(&st.visible, i)
        && let (Some(lang), Some(blocks)) = (language, &ctx.blocks)
        && let Some(found) = blocks.highlight(Some(grammar_token(lang)), &u.text)
    {
        let found: Vec<(Range<usize>, SyntaxPaint)> = match st.shown_of(i) {
            Some(s) => s.map_ranges(&found),
            None => found,
        };
        spans = found
            .into_iter()
            .filter_map(|(r, paint)| match paint {
                SyntaxPaint::Color(c) => Some(RSpan::new(r.start, r.end).color(c)),
                SyntaxPaint::Kind(_) => None,
            })
            .collect();
    }
    let wrap = st.wraps();
    let mut t = text(shown.to_owned())
        .spans(spans)
        .key("t")
        .font(p.mono)
        .color(p.text)
        .selectable(true)
        .selection_color(p.selection);
    if has_copy_button(u) {
        let at = u.text.find('\n').unwrap_or(u.text.len());
        // Assistive technology reads the code, not the button's room.
        t = t.uncopied(at..at + BUTTON_ROOM.len()).label(u.text.clone());
    }
    let t = if wrap {
        t.wrap().w_full()
    } else {
        // Keyed so a script can tell an unwrapped row from a wrapped one.
        col().key("wide").w_full().scroll_x().child(t)
    };
    let r = p.radius;
    let corners = [
        if u.first { r } else { 0.0 },
        if u.first { r } else { 0.0 },
        if u.last { r } else { 0.0 },
        if u.last { r } else { 0.0 },
    ];
    let mut panel = col()
        .w_full()
        .role("code")
        .corners(corners)
        .fill(p.code_fill)
        // One panel across a block's units: the edges between them draw no border.
        .border_sides(
            [
                if u.first { 1.0 } else { 0.0 },
                1.0,
                if u.last { 1.0 } else { 0.0 },
                1.0,
            ],
            p.code_border,
        )
        .px(CODE_PAD)
        .pt(if u.first { CODE_PAD } else { 0.0 })
        .pb(if u.last { CODE_PAD } else { 0.0 })
        .child(t);
    if has_copy_button(u) {
        let copied = st.copied.is_some_and(|(at, _)| at == i);
        let label = if copied {
            &ctx.labels.code_copied
        } else {
            &ctx.labels.copy_code
        };
        panel = panel.child(copy_button(ctx, i, copied, label, block_code, "copy"));
    }
    panel
}

/// A code block's whole code, from its first unit: every unit's text up to its last.
fn block_code(st: &DocumentState, first: usize) -> String {
    let units = &st.document().units;
    let mut out = String::new();
    for u in &units[first..] {
        out.push_str(&u.copy_text);
        if u.last {
            break;
        }
    }
    out.strip_suffix('\n').map(str::to_owned).unwrap_or(out)
}

/// A Copy button pinned to the top right of its block: copies what `copy` reads from the
/// state for `unit`, when pressed (not on every build), then shows its check and `label` (the
/// copied form) until [`DocumentState::tick`] turns it back.
fn copy_button<S: 'static>(
    ctx: &Rc<Ctx<S>>,
    unit: usize,
    copied: bool,
    label: &str,
    copy: fn(&DocumentState, usize) -> String,
    key: &str,
) -> El {
    let p = &ctx.paint;
    let get_mut = ctx.get_mut;
    let size = p.icon + 2.0 * 4.0;
    el().key(key.to_owned())
        .role("button")
        .label(label.to_owned())
        .help(label.to_owned())
        .focusable(true)
        .pin(None, Some(4.0), Some(4.0), None)
        .w(size)
        .h(size)
        .center()
        .radius(p.radius)
        .hover(p.hover)
        .child(icon(if copied { p.check } else { p.copy }, p.icon).color(p.muted))
        .on_press_tx(move |s: &mut S, tx| {
            let st = get_mut(s);
            tx.copy(copy(st, unit));
            st.mark_copied(unit);
            tx.wake_after(super::state::COPIED_FOR);
        })
}

/// An image, diagram or display equation: its pixels at their size (no wider than the text
/// column), a placeholder while loading, its alt text when it can't be drawn; a diagram and
/// a display equation offer Copy source.
fn media_el<S: 'static>(
    ctx: &Rc<Ctx<S>>,
    st: &DocumentState,
    i: usize,
    u: &Unit,
    media: &Media,
) -> El {
    let p = &ctx.paint;
    let alt = match media {
        Media::Image => u.text.clone(),
        Media::Diagram { source } | Media::Equation { source, .. } => source.to_string(),
    };
    let shown = match st.media(i) {
        MediaState::Ready(px) => {
            let (w, h) = px.size();
            let (w, h) = (w.max(1) as f32, h.max(1) as f32);
            el().pixels(px)
                .w(w)
                .max_w(CONTENT_MAX_WIDTH)
                .aspect_ratio(w / h)
                .role("img")
                .label(alt.clone())
        }
        MediaState::Loading => el()
            .w_full()
            .h(LOADING_HEIGHT)
            .radius(p.radius)
            .fill(p.placeholder)
            .role("img")
            .label(ctx.labels.loading_image.clone())
            .description(alt.clone()),
        MediaState::Failed => {
            return col().w_full().child(styled_text(
                ctx,
                u.text.clone(),
                styled(&[], p),
                (p.body, p.muted),
                "t",
                format!("{i}/t"),
                true,
            ));
        }
    };
    let label = match media {
        Media::Diagram { .. } => Some(&ctx.labels.copy_diagram),
        Media::Equation { display: true, .. } => Some(&ctx.labels.copy_equation),
        _ => None,
    };
    let mut out = col().w_full().child(shown);
    if let Some(label) = label {
        let copied = st.copied.is_some_and(|(at, _)| at == i);
        out = out
            .pr(p.icon + 16.0)
            .child(copy_button(ctx, i, copied, label, media_source, "copy"));
    }
    out
}

/// A diagram's or an equation's source, for Copy source.
fn media_source(st: &DocumentState, unit: usize) -> String {
    match &st.document().units[unit].kind {
        UnitKind::Media {
            media: Media::Diagram { source } | Media::Equation { source, .. },
            ..
        } => source.to_string(),
        _ => String::new(),
    }
}

/// A table drawn static (D1): content-sized columns from an estimate of each column's widest
/// text, wrapping within [`CELL_MAX`]; a table wider than `room` shrinks its columns in
/// proportion, down to [`CELL_MIN`] (rune-kit's `Markdown` sizing), and past that scrolls
/// sideways. The header row sits on the muted fill. Each cell is a selectable text keyed
/// `r{row}c{column}`.
fn table<S: 'static>(ctx: &Rc<Ctx<S>>, i: usize, rows: &[Vec<Cell>], room: Option<f32>) -> El {
    let p = &ctx.paint;
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    // An estimate of the body face's average advance, generous so a short cell doesn't wrap;
    // two characters of slack besides.
    let char_w = p.body.size * 0.62;
    let pad = CODE_PAD;
    let widths: Vec<f32> = (0..columns)
        .map(|c| {
            let chars = rows
                .iter()
                .filter_map(|r| r.get(c))
                .map(|cell| cell.text.chars().count() + 2)
                .max()
                .unwrap_or(0);
            (chars as f32 * char_w + 2.0 * pad).clamp(CELL_MIN, CELL_MAX)
        })
        .collect();
    let total: f32 = widths.iter().sum();
    let widths: Vec<f32> = match room {
        Some(room) if total > room && room > 0.0 => widths
            .iter()
            .map(|w| (w * room / total).floor().max(CELL_MIN))
            .collect(),
        _ => widths,
    };
    let mut grid = col()
        .role("table")
        .border(1.0, p.table_rule)
        .radius(p.radius)
        .clip_soft(0.0);
    for (r, cells) in rows.iter().enumerate() {
        let header = cells.iter().all(|c| c.header) && !cells.is_empty();
        let mut line = row().role("row");
        if header {
            line = line.fill(p.head_fill);
        }
        if r > 0 {
            line = line.border_sides([1.0, 0.0, 0.0, 0.0], p.table_rule);
        }
        for (c, w) in widths.iter().enumerate() {
            let cell = cells.get(c);
            let content = cell.map_or_else(String::new, |c| c.text.clone());
            let st = styled(cell.map_or(&[][..], |c| &c.spans[..]), p);
            let style = if cell.is_some_and(|c| c.header) {
                TextStyle {
                    weight: 700.0,
                    ..p.body
                }
            } else {
                p.body
            };
            let key = format!("r{r}c{c}");
            let t = styled_text(
                ctx,
                content,
                st,
                (style, p.text),
                &key,
                format!("{i}/{key}"),
                true,
            );
            line = line.child(
                col()
                    .w(*w)
                    .px(pad)
                    .py(pad / 2.0)
                    .role(if cell.is_some_and(|c| c.header) {
                        "columnheader"
                    } else {
                        "cell"
                    })
                    .child(t),
            );
        }
        grid = grid.child(line);
    }
    col().w_full().scroll_x().child(grid)
}
