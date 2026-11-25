// SPDX-License-Identifier: MIT

//! [`FilterBar`]: the bar's element and the field's keys.

use std::rc::Rc;

use rune::{El, Key, KeyEvent, LiveRegion, Mods, Tx, icon, row, text};
use strata_kit::icons::IconName;
use strata_kit::look::Look;

use super::state::{FilterIntent, FilterState, Polled};
use super::target::Results;
use crate::collection::selection::Position;
use crate::filter_bar::field::{self, Handlers, Spec};

#[derive(Clone, Debug, PartialEq)]
pub struct FilterLabels {
    pub field: String,
    pub placeholder: String,
}

impl Default for FilterLabels {
    fn default() -> Self {
        Self {
            field: "Filter (Ctrl+F)".to_owned(),
            placeholder: "Filter…".to_owned(),
        }
    }
}

type ResultsFn<S> = for<'a> fn(&'a S) -> Results<'a>;
type IntentFn<S> = Rc<dyn Fn(&mut S, FilterIntent, &mut Tx)>;
type PreviewsFn<S> = Rc<dyn Fn(&S) -> bool>;

pub struct FilterBar<S> {
    key: String,
    get: fn(&S) -> &FilterState,
    get_mut: fn(&mut S) -> &mut FilterState,
    labels: FilterLabels,
    results: Option<ResultsFn<S>>,
    previews: Option<PreviewsFn<S>>,
    on_intent: Option<IntentFn<S>>,
    tab_to_results: bool,
    focus_return: Option<String>,
}

impl<S> Clone for FilterBar<S> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            get: self.get,
            get_mut: self.get_mut,
            labels: self.labels.clone(),
            results: self.results,
            previews: self.previews.clone(),
            on_intent: self.on_intent.clone(),
            tab_to_results: self.tab_to_results,
            focus_return: self.focus_return.clone(),
        }
    }
}

impl<S: 'static> FilterBar<S> {
    /// The bar keyed `key` (its field is [`FilterBar::field_key`]).
    pub fn new(
        key: impl Into<String>,
        get: fn(&S) -> &FilterState,
        get_mut: fn(&mut S) -> &mut FilterState,
    ) -> Self {
        Self {
            key: key.into(),
            get,
            get_mut,
            labels: FilterLabels::default(),
            results: None,
            previews: None,
            on_intent: None,
            tab_to_results: true,
            focus_return: None,
        }
    }

    /// The bar's words.
    pub fn labels(mut self, labels: FilterLabels) -> Self {
        self.labels = labels;
        self
    }

    pub fn results(mut self, f: ResultsFn<S>) -> Self {
        self.results = Some(f);
        self
    }

    pub fn previews(mut self, f: impl Fn(&S) -> bool + 'static) -> Self {
        self.previews = Some(Rc::new(f));
        self
    }

    /// Hears the bar's intents.
    pub fn on_intent(mut self, f: impl Fn(&mut S, FilterIntent, &mut Tx) + 'static) -> Self {
        self.on_intent = Some(Rc::new(f));
        self
    }

    pub fn tab_to_results(mut self, on: bool) -> Self {
        self.tab_to_results = on;
        self
    }

    pub fn focus_return(mut self, key: impl Into<String>) -> Self {
        self.focus_return = Some(key.into());
        self
    }

    /// The field's key.
    pub fn field_key(&self) -> String {
        format!("{}-field", self.key)
    }

    /// Sends `polled`'s intents to the host.
    pub fn route(&self, s: &mut S, polled: Polled, tx: &mut Tx) {
        if let Some(f) = &self.on_intent {
            for intent in polled.intents {
                f(s, intent, tx);
            }
        }
    }

    /// The bar, or `None` while it is closed: the funnel and the field in a text box frame.
    pub fn build(&self, s: &S, look: &Look) -> Option<El> {
        let st = (self.get)(s);
        if !st.is_open() {
            return None;
        }
        let me = Rc::new(self.clone());
        let handlers = handlers(&me);
        let field_key = self.field_key();
        let spec = Spec {
            key: &field_key,
            label: &self.labels.field,
            placeholder: Some(&self.labels.placeholder),
            description: None,
            invalid: false,
            font: look.kit.ty.ui_compact,
        };
        let input = field::input(look, &st.edit, &spec, &handlers);
        let inner = field::frame_row(look, format!("{}-box", self.key))
            .child(
                icon(look.icon(IconName::Filter), look.icon_size.sm)
                    .color(look.kit.c.icon_accent)
                    .a11y_hidden(),
            )
            .child(input);
        let mut bar = row()
            .key(self.key.clone())
            .w_full()
            .px(look.space.sm)
            .py(look.space.xs)
            .child(field::frame(look, inner, st.edit.focused, false));
        if let Some(to) = &self.focus_return {
            bar = bar.focus_return(to.clone());
        }
        Some(bar)
    }
}

pub fn status(look: &Look, key: impl Into<String>, words: impl Into<String>) -> El {
    let words = words.into();
    row()
        .key(key.into())
        .role("status")
        .label(words.clone())
        .live_region(LiveRegion::Polite)
        .w_full()
        .h_full()
        .center()
        .child(
            text(words)
                .font(look.kit.ty.ui_compact)
                .color(look.kit.c.text_muted)
                .a11y_hidden(),
        )
}

fn handlers<S: 'static>(bar: &Rc<FilterBar<S>>) -> Handlers<S> {
    let (b_edit, b_key, b_change, b_menu) = (bar.clone(), bar.clone(), bar.clone(), bar.clone());
    Handlers {
        edit: Rc::new(move |s: &mut S| &mut (b_edit.get_mut)(s).edit),
        key: Rc::new(move |s: &mut S, ev: &KeyEvent, tx: &mut Tx| key(&b_key, s, ev, tx)),
        change: Rc::new(move |s: &mut S, tx: &mut Tx| {
            // Reading the selection costs a pass over the listing: only when a query starts.
            let before = match b_change.results {
                Some(f) if (b_change.get)(s).needs_save() => {
                    let r = f(s);
                    r.selection.position(r.source)
                }
                _ => Position::default(),
            };
            let polled = (b_change.get_mut)(s).text_changed(|| before);
            b_change.route(s, polled, tx);
        }),
        menu: Some(Rc::new(move |s: &mut S, at, tx: &mut Tx| {
            let mut polled = Polled::default();
            polled.push(FilterIntent::FieldMenu { at });
            b_menu.route(s, polled, tx);
        })),
    }
}

fn plain(m: Mods) -> bool {
    !(m.shift || m.ctrl || m.alt || m.cmd || m.logo)
}

/// A key in the field, before the editor sees it. True takes it.
fn key<S: 'static>(bar: &FilterBar<S>, s: &mut S, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if (bar.get)(s).edit.is_composing() {
        return false;
    }
    let repeat = tx.key_repeat();
    match &ev.key {
        Key::Escape if !(ev.mods.ctrl || ev.mods.alt || ev.mods.cmd || ev.mods.logo) => {
            if !repeat {
                let polled = (bar.get_mut)(s).close();
                bar.route(s, polled, tx);
            }
            true
        }
        Key::Enter if plain(ev.mods) => {
            if repeat {
                return true;
            }
            let target = bar.results.and_then(|f| (bar.get)(s).target(f(s)));
            if let Some(id) = target {
                let mut polled = Polled::default();
                polled.push(FilterIntent::Open { id });
                bar.route(s, polled, tx);
            }
            true
        }
        Key::Down if plain(ev.mods) => {
            let Some(results) = bar.results else {
                return false;
            };
            let st = (bar.get)(s);
            let r = results(s);
            if r.source.is_empty() || st.text().trim().is_empty() {
                return false;
            }
            // The outlined top result, or the first selected result after a pick.
            let Some(id) = st.landing(r) else {
                return false;
            };
            (bar.get_mut)(s).pick();
            let mut polled = Polled::default();
            polled.push(FilterIntent::FocusResults { id });
            bar.route(s, polled, tx);
            if let (true, Some(to)) = (bar.tab_to_results, &bar.focus_return) {
                tx.focus_when_shown(to.clone(), true);
            } else if bar.tab_to_results {
                tx.send_key(KeyEvent {
                    key: Key::Tab,
                    mods: Mods::default(),
                });
            }
            true
        }
        Key::Text(t) if t == " " && plain(ev.mods) => {
            let selected = bar.results.is_some_and(|f| !f(s).selection.is_empty());
            let previews = selected && bar.previews.as_ref().is_some_and(|f| f(s));
            if previews {
                let mut polled = Polled::default();
                polled.push(FilterIntent::Preview);
                bar.route(s, polled, tx);
            }
            previews
        }
        _ => false,
    }
}

pub fn position_of(r: Results<'_>) -> Position {
    r.selection.position(r.source)
}
