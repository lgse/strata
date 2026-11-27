// SPDX-License-Identifier: MIT

use std::rc::Rc;
use std::time::Duration;

use rune::{Align, Cursor, El, Key, KeyEvent, TextEdit, Tx, icon, row, text, text_input};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::render::Ctx;
use super::state::PdfState;

const PILL_RADIUS: f32 = 12.0;
const FIELD_RADIUS: f32 = 6.0;
const BUTTON_RADIUS: f32 = 6.0;
const DISABLED_OPACITY: f32 = 0.45;
const MIN_INPUT_WIDTH: f32 = 24.0;
const NAV_HEIGHT: f32 = 26.0;

#[derive(Clone, Debug)]
pub(crate) struct PageNav {
    pub edit: TextEdit,
    /// True while a field context menu holds focus so losing focus doesn't revert edits.
    held: bool,
    /// True once the page input has taken focus; cleared when blur reconciles or editing finishes.
    seen_focus: bool,
    /// Pending select-all after pointer focus so a first click lets one-shot typing replace the page.
    select_all_pending: bool,
}

impl PageNav {
    pub(crate) fn new(page: usize) -> Self {
        Self {
            edit: TextEdit::new(page.max(1).to_string()),
            held: false,
            seen_focus: false,
            select_all_pending: false,
        }
    }

    pub(crate) fn reset(&mut self, page: usize) {
        self.edit = TextEdit::new(page.max(1).to_string());
        self.held = false;
        self.seen_focus = false;
        self.select_all_pending = false;
    }

    pub(crate) fn is_focused(&self) -> bool {
        self.edit.focused
    }

    pub(crate) fn hold(&mut self, held: bool) {
        self.held = held;
    }

    pub(crate) fn on_focus(&mut self) {
        self.seen_focus = true;
        if self.held {
            return;
        }
        self.select_all_pending = true;
        let len = self.edit.text.len();
        self.edit.set_selection(0, len);
    }

    pub(crate) fn clear_select_all_pending(&mut self) {
        self.select_all_pending = false;
    }

    pub(crate) fn sync_unfocused(&mut self, page: usize) {
        if self.edit.focused || self.held {
            return;
        }
        self.set_page_text(page);
    }

    pub(crate) fn finish_edit(&mut self, page: usize) {
        self.edit.focused = false;
        self.edit.cancel_composition();
        self.seen_focus = false;
        self.select_all_pending = false;
        self.set_page_text(page);
    }

    pub(crate) fn reconcile_focus(&mut self, page: usize) -> bool {
        if self.edit.focused {
            self.seen_focus = true;
            if std::mem::take(&mut self.select_all_pending) {
                let (start, end) = self.edit.range();
                if start == end && !self.edit.text.is_empty() {
                    let len = self.edit.text.len();
                    self.edit.set_selection(0, len);
                    return true;
                }
            }
            return false;
        }
        self.select_all_pending = false;
        if !self.seen_focus || self.held {
            return false;
        }
        self.seen_focus = false;
        self.set_page_text(page)
    }

    pub(crate) fn set_page_text(&mut self, page: usize) -> bool {
        let target = page.max(1).to_string();
        if self.edit.text == target {
            return false;
        }
        self.edit.replace_text(target);
        true
    }
}

pub(crate) fn build<S: 'static>(ctx: &Rc<Ctx<S>>, st: &PdfState, look: &Look) -> El {
    let k = &look.kit;
    let labels = &ctx.labels;
    let total = st.document().pages();
    let current = st.current_page();

    let prev = step_button(
        ctx,
        StepButton {
            key: st.page_prev_key(),
            icon: IconName::ChevronLeft,
            label: &labels.previous_page,
            disabled: current == 0,
            delta: -1,
        },
        look,
    );
    let next = step_button(
        ctx,
        StepButton {
            key: st.page_next_key(),
            icon: IconName::ChevronRight,
            label: &labels.next_page,
            disabled: current + 1 >= total,
            delta: 1,
        },
        look,
    );

    let mut pill = row()
        .key(format!("{}-page-nav", st.key()))
        .role("group")
        .label(labels.page_navigation.clone())
        .items_center()
        .gap(look.space.xs)
        .px(look.space.xs)
        .py(look.space.xxs)
        .fill(look.transparency.opaque(k.c.surface_raised))
        .border(1.0, k.c.border_base)
        .radius(look.round(PILL_RADIUS))
        .cursor(Cursor::Default)
        .keeps_text_selection()
        .on_background_press_tx(|_: &mut S, tx: &mut Tx| tx.live_only())
        .on_drag(|_: &mut S, _, tx: &mut Tx| tx.live_only())
        .on_middle_press(|_: &mut S, tx: &mut Tx| tx.live_only())
        .on_secondary_press(|_: &mut S, _, tx: &mut Tx| tx.live_only())
        .child(prev);

    pill = append_indicator(pill, ctx, st, look).child(next);
    pill = k
        .shadow_layers(&k.elevation.overlay)
        .fold(pill, |e, s| e.box_shadow(s));

    row()
        .pin(Some(0.0), None, Some(0.0), Some(look.space.md))
        .justify_center()
        .child(pill)
}

fn append_indicator<S: 'static>(mut pill: El, ctx: &Rc<Ctx<S>>, st: &PdfState, look: &Look) -> El {
    let k = &look.kit;
    let font = k.ty.ui_compact;
    let total = st.document().pages();
    let template = ctx
        .labels
        .page_indicator
        .replace("{pages}", &total.to_string());
    let (before, after) = match template.split_once("{input}") {
        Some((before, after)) => (before.trim(), after.trim()),
        None => ("", template.trim()),
    };

    if !before.is_empty() {
        pill = pill.child(
            text(before.to_owned())
                .font(font)
                .color(k.c.text_muted)
                .a11y_hidden(),
        );
    }
    pill = pill.child(page_input(ctx, st, look));
    if !after.is_empty() {
        pill = pill.child(
            text(after.to_owned())
                .key(format!("{}-page-total", st.key()))
                .font(font)
                .color(k.c.text_muted)
                .a11y_hidden(),
        );
    }
    pill
}

fn page_input<S: 'static>(ctx: &Rc<Ctx<S>>, st: &PdfState, look: &Look) -> El {
    let k = &look.kit;
    let font = k.ty.ui_compact;
    let get_mut = ctx.get_mut;
    let digits = st.document().pages().max(1).to_string().len();
    let sample = "8".repeat(digits);
    let width = (rune::text::advance(&sample, font).ceil() + look.space.xs).max(MIN_INPUT_WIDTH);
    let focused = st.page_nav.is_focused();

    let field = text_input(&st.page_nav.edit, font)
        .single_line()
        .key(st.page_input_key())
        .label(ctx.labels.go_to_page.clone())
        .w(width)
        .align(Align::Center)
        .color(k.c.text_base)
        .caret(k.c.text_base, k.selection())
        .spellcheck(false)
        .on_edit(move |s: &mut S| &mut get_mut(s).page_nav.edit)
        .on_input_change(move |s: &mut S, _: &mut Tx| {
            get_mut(s).page_nav.clear_select_all_pending();
        })
        .on_focus_change(move |s: &mut S, focused: bool, tx: &mut Tx| {
            let pdf = get_mut(s);
            if focused {
                pdf.page_nav.on_focus();
            }
            // Schedule a poll so pointer focus selects all after caret placement and blur
            // reconciles after the host updates field-menu hold state.
            tx.wake_after(Duration::ZERO);
        })
        .on_input_key(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            handle_input_key(get_mut(s), ev, tx)
        });

    let border = if focused {
        k.c.border_focus
    } else {
        k.c.border_strong
    };
    let mut box_el = row()
        .role("textbox")
        .h(NAV_HEIGHT)
        .px(look.space.xs)
        .items_center()
        .radius(look.round(FIELD_RADIUS))
        .fill(look.fill.muted)
        .border(1.0, border)
        .cursor(Cursor::Text)
        .child(field);
    if focused {
        box_el = box_el.outline(2.0, -2.0, k.c.border_focus);
    }
    box_el
}

fn handle_input_key(st: &mut PdfState, ev: &KeyEvent, tx: &mut Tx) -> bool {
    st.page_nav.clear_select_all_pending();
    if st.page_nav.edit.is_composing() {
        return false;
    }
    let m = ev.mods;
    if ev.key == Key::Escape {
        st.page_nav.finish_edit(st.current_page() + 1);
        tx.focus(st.key(), false);
        return true;
    }
    if ev.key == Key::Enter && !m.ctrl && !m.cmd && !m.alt && !m.logo {
        commit_input(st, tx);
        return true;
    }
    false
}

fn commit_input(st: &mut PdfState, tx: &mut Tx) {
    let trimmed = st.page_nav.edit.text.trim();
    if let Ok(page_1based) = trimmed.parse::<usize>() {
        let target = page_1based.saturating_sub(1);
        st.jump_to_page(target, tx);
    }
    st.page_nav.finish_edit(st.current_page() + 1);
    tx.focus(st.key(), false);
}

struct StepButton<'a> {
    key: String,
    icon: IconName,
    label: &'a str,
    disabled: bool,
    delta: isize,
}

fn step_button<S: 'static>(ctx: &Rc<Ctx<S>>, spec: StepButton<'_>, look: &Look) -> El {
    let k = &look.kit;
    let ink = if spec.disabled {
        k.c.icon_muted
    } else {
        k.c.icon_accent
    };
    let mut button = row()
        .key(spec.key)
        .role("button")
        .label(spec.label.to_owned())
        .help(spec.label.to_owned())
        .size(NAV_HEIGHT, NAV_HEIGHT)
        .center()
        .radius(look.round(BUTTON_RADIUS))
        .child(
            icon(look.icon(spec.icon), look.icon_size.sm)
                .color(ink)
                .a11y_hidden(),
        );
    if spec.disabled {
        return button
            .disabled(true)
            .focusable(false)
            .opacity(DISABLED_OPACITY);
    }
    let get_mut = ctx.get_mut;
    let delta = spec.delta;
    button = button
        .cursor(Cursor::Pointer)
        .hover(k.c.hover)
        .press(k.c.pressed)
        .focusable(true)
        .focus_ring(2.0, -2.0, k.c.border_focus)
        .on_press_tx(move |s: &mut S, tx: &mut Tx| {
            let st = get_mut(s);
            let target = st.current_page().saturating_add_signed(delta);
            st.jump_to_page(target, tx);
        })
        .press_on_return();
    button
}
