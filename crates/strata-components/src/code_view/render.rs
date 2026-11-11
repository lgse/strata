//! One row of the code view: its gutter (the line's number, or "↳" for a later part of a
//! split line) and its text, with the line's colours once they land and find's matches.

use std::ops::Range;
use std::rc::Rc;

use rune::{Align, Color, El, Span, TextHighlight, TextStyle, Tx, col, rich, row, text};
use strata_kit::look::Look;

use super::state::CodeState;

/// How the view draws, from the look's Kit and strata-kit tokens.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Paint {
    pub mono: TextStyle,
    pub small: TextStyle,
    pub text: Color,
    pub muted: Color,
    pub number: Color,
    pub surface: Color,
    pub selection: Color,
    pub matched: Color,
    pub focus: Color,
    /// Padding left and right of the code, and the gutter's gap after its numbers.
    pub pad: f32,
}

impl Paint {
    pub fn from_look(look: &Look) -> Paint {
        let k = &look.kit;
        let c = &k.c;
        Paint {
            mono: TextStyle {
                size: k.ty.body.size,
                ..k.ty.mono
            },
            small: k.ty.small,
            text: c.text_base,
            muted: c.text_muted,
            number: c.text_weak,
            surface: c.bg_base,
            selection: k.selection(),
            matched: look.fill.highlight,
            focus: c.border_focus,
            pad: look.space.md,
        }
    }

    pub fn line(&self) -> f32 {
        self.mono.line_px()
    }
}

/// What every row of one build shares.
pub(crate) struct Ctx<S> {
    pub get: fn(&S) -> &CodeState,
    pub paint: Paint,
    /// The gutter's width, or none with line numbers off.
    pub gutter: Option<f32>,
    pub wrap: bool,
    /// An empty line's accessible name.
    pub blank: String,
}

/// The digits' width for `lines` lines in `style`, with the gutter's gap.
pub(crate) fn gutter_width(lines: usize, style: TextStyle, pad: f32) -> f32 {
    let digits = lines.max(1).to_string().len().max(2);
    let w = rune::text::advance(&"0".repeat(digits), style);
    (w + 2.0 * pad).ceil()
}

/// Line `line`'s colours cut to the row's bytes `row` (relative to the line), relative to the
/// row.
pub(crate) fn row_spans(spans: Vec<Span>, row: Range<usize>) -> Vec<Span> {
    let (a, b) = (row.start as u32, row.end as u32);
    spans
        .into_iter()
        .filter(|s| s.end > a && s.start < b)
        .map(|mut s| {
            s.start = s.start.max(a) - a;
            s.end = s.end.min(b) - a;
            s
        })
        .collect()
}

pub(crate) fn build_row<S: 'static>(ctx: &Rc<Ctx<S>>, s: &S, i: usize) -> El {
    let st = (ctx.get)(s);
    let rows = st.rows();
    let r = rows.row(i);
    let p = &ctx.paint;
    let src = rows.source(i);
    let line_start = rows.line_start(r.line);
    // Plain until the line's colours land: the same text and font with no spans, so they
    // land without moving anything.
    let spans = st
        .doc()
        .and_then(|d| d.line_spans(r.line))
        .map(|sp| row_spans(sp, r.start - line_start..r.end - line_start))
        .unwrap_or_default();
    let current = st.current_match();
    let highlights: Vec<TextHighlight> = st
        .found()
        .in_row(rows, i)
        .map(|(n, m)| {
            let h = TextHighlight::new(m, p.matched);
            if Some(n) == current {
                h.key("current")
            } else {
                h
            }
        })
        .collect();
    let t = rich(src.to_owned(), spans)
        .font(p.mono)
        .color(p.text)
        .key("t")
        .selectable(true)
        .selection_color(p.selection)
        .text_highlights(highlights);
    let t = if src.is_empty() {
        t.label(ctx.blank.clone())
    } else {
        t
    };
    let t = if ctx.wrap {
        // Code breaks inside a token too long for the pane, not past its edge.
        t.wrap_anywhere().w_full()
    } else {
        t.single_line()
    };
    let code = col().grow(1.0).min_w(0.0).px(p.pad).child(t);
    let mut out = row().w_full().items_start();
    if let Some(w) = ctx.gutter {
        let label = if r.cont {
            "↳".to_owned()
        } else {
            (r.line + 1).to_string()
        };
        let mut n = text(label)
            .font(p.mono)
            .color(p.number)
            .align(Align::End)
            .key("n")
            .w(w)
            .pr(p.pad)
            .fill(p.surface)
            .a11y_hidden();
        if !ctx.wrap {
            // The numbers stay while the code scrolls sideways under them.
            n = n.sticky_left(0.0);
        }
        out = out.child(n);
    }
    let out = out.child(code);
    let pos = (ctx.get)(s).position.clone();
    out.on_layout_tx(move |_: &mut S, rect, tx: &mut Tx| {
        tx.live_only();
        pos.row(i, rect, tx);
    })
}
