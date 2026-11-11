//! [`FooterPrompt`]: the prompt bar in the footer, and its candidate list above it.

use std::rc::Rc;

use rune::{Color, Cx, El, GrowthSpec, KeyEvent, ScrollSpec, Tx, col, icon, row, text};
use rune_kit::{Placement, popover};
use strata_kit::look::Look;

use super::state::{Polled, PromptIntent, PromptState};
use crate::contract::MatchedText;
use crate::filter_bar::field::{self, Handlers, Spec};
use crate::filter_bar::keycap::keycap;

/// The most candidate rows the list shows before it scrolls (Strata's 280 px).
const ROWS_SHOWN: f32 = 8.0;

/// The candidate list's width bounds, around the field's width (Strata's 280 to 560 px).
const LIST_MIN_W: f32 = 280.0;
const LIST_MAX_W: f32 = 560.0;

type IntentFn<S, K> = Rc<dyn Fn(&mut S, PromptIntent<K>, &mut Tx)>;
type PositionFn = Rc<dyn Fn(usize, usize) -> String>;

/// The prompt's words that aren't a [`super::PromptSpec`]'s, in the interface language.
///
/// Used by: hosts.
#[derive(Clone)]
pub struct PromptLabels {
    /// The chosen candidate's place, for a spec with `position`: "2 of 5".
    pub position: PositionFn,
    /// The candidate list's accessible name.
    pub candidates: String,
    /// The keycaps the legend shows: Up and Down, Tab, Enter, Escape.
    pub keys: [String; 4],
}

impl Default for PromptLabels {
    fn default() -> Self {
        Self {
            position: Rc::new(|i, n| format!("{i} of {n}")),
            candidates: "Folders".to_owned(),
            keys: ["\u{2191}\u{2193}", "Tab", "Enter", "Esc"].map(str::to_owned),
        }
    }
}

impl std::fmt::Debug for PromptLabels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptLabels")
            .field("candidates", &self.candidates)
            .field("keys", &self.keys)
            .finish()
    }
}

/// Builds the footer prompt over the [`PromptState`] `get` and `get_mut` reach, for the
/// footer's prompt slot (C19), which it fills while a prompt is open:
///
/// ```ignore
/// let prompt = FooterPrompt::new("prompt", |s: &App| &s.prompt, |s: &mut App| &mut s.prompt)
///     .focus_return("pane")          // the listing's focus stop
///     .on_intent(|s: &mut App, intent, tx| s.prompted(intent, tx));
/// footer = footer.prompt(prompt.build(s, &s.look, cx, &mut layer));
/// // Each poll: s.prompt.tick(), then route its intents; deliver candidates as they come.
/// ```
///
/// Used by: hosts (Strata's 10xer mode), C19.
pub struct FooterPrompt<S, K> {
    key: String,
    get: fn(&S) -> &PromptState<K>,
    get_mut: fn(&mut S) -> &mut PromptState<K>,
    labels: PromptLabels,
    focus_return: Option<String>,
    on_intent: Option<IntentFn<S, K>>,
}

impl<S, K> Clone for FooterPrompt<S, K> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            get: self.get,
            get_mut: self.get_mut,
            labels: self.labels.clone(),
            focus_return: self.focus_return.clone(),
            on_intent: self.on_intent.clone(),
        }
    }
}

impl<S: 'static, K: Copy + PartialEq + std::fmt::Debug + 'static> FooterPrompt<S, K> {
    /// The prompt keyed `key` (its field is [`FooterPrompt::field_key`]).
    pub fn new(
        key: impl Into<String>,
        get: fn(&S) -> &PromptState<K>,
        get_mut: fn(&mut S) -> &mut PromptState<K>,
    ) -> Self {
        Self {
            key: key.into(),
            get,
            get_mut,
            labels: PromptLabels::default(),
            focus_return: None,
            on_intent: None,
        }
    }

    /// The prompt's other words.
    pub fn labels(mut self, labels: PromptLabels) -> Self {
        self.labels = labels;
        self
    }

    /// Where the keyboard goes when the prompt closes with it (the listing's focus stop): the
    /// bar carries `El::focus_return`, which covers every way the prompt can go.
    pub fn focus_return(mut self, key: impl Into<String>) -> Self {
        self.focus_return = Some(key.into());
        self
    }

    /// Hears the prompt's intents.
    pub fn on_intent(mut self, f: impl Fn(&mut S, PromptIntent<K>, &mut Tx) + 'static) -> Self {
        self.on_intent = Some(Rc::new(f));
        self
    }

    /// The field's key.
    pub fn field_key(&self) -> String {
        format!("{}-field", self.key)
    }

    /// Candidate `i`'s row key.
    pub fn option_key(&self, i: usize) -> String {
        format!("{}-option-{i}", self.key)
    }

    /// Sends `polled`'s intents to the host.
    pub fn route(&self, s: &mut S, polled: Polled<K>, tx: &mut Tx) {
        if let Some(f) = &self.on_intent {
            for intent in polled.intents {
                f(s, intent, tx);
            }
        }
    }

    /// The hint beside the field: the host's, else the chosen candidate's place for a spec
    /// that shows it.
    pub fn hint(&self, s: &S) -> Option<String> {
        let st = (self.get)(s);
        if let Some(h) = st.hint() {
            return Some(h.to_owned());
        }
        let spec = st.spec()?;
        let n = st.candidates().len();
        (spec.position && n > 0)
            .then(|| (self.labels.position)(st.chosen_index().unwrap_or(0) + 1, n))
    }

    /// The prompt bar, or `None` with no prompt open: the label in the accent, the field and
    /// the hint, across the whole footer. Its candidates go onto `layer` (the overlays
    /// `rune_kit::root` mounts), above the field at about its width.
    pub fn build(&self, s: &S, look: &Look, cx: &mut Cx<S>, layer: &mut Vec<El>) -> Option<El> {
        let st = (self.get)(s);
        let spec = st.spec()?;
        let k = &look.kit;
        let me = Rc::new(self.clone());
        let handlers = handlers(&me);
        let hint = self.hint(s);
        let field_key = self.field_key();
        let list = !st.candidates().is_empty();
        let mut input = field::input(
            look,
            &st.edit,
            &Spec {
                key: &field_key,
                label: &spec.name,
                placeholder: None,
                description: hint.as_deref(),
                invalid: st.invalid(),
                font: k.ty.ui_compact,
            },
            &handlers,
        );
        if list && let Some(i) = st.chosen_index() {
            input = input.active_descendant(self.option_key(i));
        }
        let framed = field::frame(
            look,
            field::frame_row(look, format!("{}-box", self.key)).child(input),
            st.edit.focused,
            st.invalid(),
            &handlers.menu,
        );
        // A text input is always a text box to Rune's accessibility (Rune draft U-C6-1), so a
        // prompt with candidates is a combobox group around it, as C6's entry is.
        let field_part = if spec.legend.is_some() {
            row()
                .key(format!("{}-combo", self.key))
                .role("combobox")
                .label(spec.name.clone())
                .expanded(list)
                .grow(1.0)
                .min_w(0.0)
                .child(framed)
        } else {
            row().grow(1.0).min_w(0.0).child(framed)
        };
        let mut bar = row()
            .key(self.key.clone())
            .w_full()
            .h_full()
            .px(look.space.sm)
            .gap(look.space.sm)
            .items_center()
            .child(
                // The field's name says it in full.
                text(spec.label.clone())
                    .key(format!("{}-label", self.key))
                    .font(k.ty.ui_medium)
                    .color(k.c.text_accent)
                    .a11y_hidden(),
            )
            .child(field_part);
        if let Some(h) = &hint {
            bar = bar.child(
                // The field's description says it.
                text(h.clone())
                    .key(format!("{}-hint", self.key))
                    .font(k.ty.small)
                    .color(if st.invalid() {
                        k.c.danger.fg
                    } else {
                        k.c.text_muted
                    })
                    .truncate()
                    .shrink()
                    .a11y_hidden(),
            );
        }
        if let Some(to) = &self.focus_return {
            bar = bar.focus_return(to.clone());
        }
        if list {
            let width = cx
                .layout(&format!("{}-box", self.key))
                .map_or(LIST_MIN_W, |r| r[2])
                .clamp(LIST_MIN_W, LIST_MAX_W);
            layer.push(self.candidates(s, look, width));
        }
        Some(bar)
    }

    /// The candidate list, anchored above the field: rows with a folder icon, the name with
    /// its matches in the accent and the parent dim, then the legend's keys.
    fn candidates(&self, s: &S, look: &Look, width: f32) -> El {
        let st = (self.get)(s);
        let k = &look.kit;
        let items = st.candidates();
        let n = items.len();
        let chosen = st.chosen_index();
        let row_h = look.row.list;
        let mut rows = col()
            .key(format!("{}-rows", self.key))
            .w_full()
            .growth(GrowthSpec::default())
            .virtualize(row_h)
            .reveal_selected(true);
        for (i, c) in items.iter().enumerate() {
            let active = chosen == Some(i);
            let me = self.clone();
            let label = MatchedText::new(c.label.clone(), c.matched.iter().cloned())
                .font(k.ty.ui_compact)
                .truncate(true)
                .build(k);
            rows = rows.child(
                row()
                    .key(self.option_key(i))
                    .role("option")
                    .label(c.label.clone())
                    .description(c.hint.clone())
                    .position_in_set(i + 1, n)
                    .focus_selected(active)
                    .w_full()
                    .h(row_h)
                    .px(look.space.sm)
                    .gap(look.space.sm)
                    .items_center()
                    .radius(k.radius.sm)
                    .fill(if active {
                        k.c.hover
                    } else {
                        Color::TRANSPARENT
                    })
                    // A tint only: the pointer never moves the keyboard's choice (C6's rule).
                    .hover(k.c.hover)
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| {
                        let polled = (me.get_mut)(s).press_candidate(i);
                        me.route(s, polled, tx);
                    })
                    .child(
                        icon(look.icon(c.icon), look.icon_size.md)
                            .color(k.c.icon_muted)
                            .a11y_hidden(),
                    )
                    .child(row().grow(1.0).min_w(0.0).a11y_hidden().child(label))
                    .child(
                        text(c.hint.clone())
                            .font(k.ty.small)
                            .color(k.c.text_muted)
                            .truncate()
                            .shrink()
                            .a11y_hidden(),
                    ),
            );
        }
        let legend = st.spec().and_then(|s| s.legend.clone());
        let mut description = Vec::new();
        let mut keys = row()
            .key(format!("{}-legend", self.key))
            .w_full()
            .px(look.space.sm)
            .py(look.space.xs)
            .gap(look.space.md)
            .items_center()
            .a11y_hidden();
        if let Some(l) = &legend {
            let [arrows, tab, enter, esc] = &self.labels.keys;
            let mut pairs = vec![(arrows, &l.choose)];
            if let Some(c) = &l.complete {
                pairs.push((tab, c));
            }
            pairs.push((enter, &l.enter));
            pairs.push((esc, &l.cancel));
            for (cap, word) in pairs {
                description.push(format!("{cap} {word}"));
                keys = keys.child(
                    row()
                        .gap(look.space.xs)
                        .items_center()
                        .child(keycap(look, cap))
                        .child(text(word.clone()).font(k.ty.small).color(k.c.text_muted)),
                );
            }
        }
        let viewport = col()
            .key(format!("{}-candidates", self.key))
            .role("listbox")
            .label(self.labels.candidates.clone())
            .description(description.join(", "))
            .w_full()
            .h((n as f32 * row_h).min(ROWS_SHOWN * row_h))
            .shrink()
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .clip_soft(0.0)
            .child(rows);
        let content = col()
            .key(format!("{}-panel", self.key))
            .role("none")
            .w(width)
            .padding(look.space.xxs)
            // The field keeps the keyboard while the pointer picks a row.
            .preserve_focus_on_press(true)
            .on_press(|_: &mut S| {})
            .instant_changes()
            .child(viewport)
            .child(keys);
        popover::anchored(
            k,
            popover::raised(k, content, k.radius.md),
            &self.field_key(),
            Placement::TopStart,
            look.space.xs,
        )
    }
}

fn handlers<S: 'static, K: Copy + PartialEq + std::fmt::Debug + 'static>(
    p: &Rc<FooterPrompt<S, K>>,
) -> Handlers<S> {
    let (p_edit, p_key, p_change, p_menu) = (p.clone(), p.clone(), p.clone(), p.clone());
    Handlers {
        edit: Rc::new(move |s: &mut S| &mut (p_edit.get_mut)(s).edit),
        key: Rc::new(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| {
            let repeat = tx.key_repeat();
            match (p_key.get_mut)(s).key(ev, repeat) {
                Some(polled) => {
                    p_key.route(s, polled, tx);
                    true
                }
                None => false,
            }
        }),
        change: Rc::new(move |s: &mut S, tx: &mut Tx| {
            let polled = (p_change.get_mut)(s).edited();
            p_change.route(s, polled, tx);
        }),
        menu: Rc::new(move |s: &mut S, at, tx: &mut Tx| {
            let mut polled = Polled::default();
            polled.push(PromptIntent::FieldMenu { at });
            p_menu.route(s, polled, tx);
        }),
    }
}
