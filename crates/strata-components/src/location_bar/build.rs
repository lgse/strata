// SPDX-License-Identifier: MIT

//! [`LocationBar`]: the builder, and the context its parts share.

use std::rc::Rc;

use rune::{Chord, Cx, El, Mods, Tx, el};
use strata_kit::look::Look;

use super::data::{LocationLabels, Trail};
use super::intent::LocationIntent;
use super::state::{LocationBarState, Polled, SPRING_DELAY};
use super::{crumbs, entry, menu, popup};
use crate::contract::dnd::{Dnd, DropAction, DropOffer};
use crate::scroll::BarLook;

/// Where the bar's intents go.
pub(crate) type Emit<S> = Rc<dyn Fn(&mut S, LocationIntent, &mut Tx)>;

/// Finds the bar's state in the app's.
pub type Access<S> = fn(&mut S) -> &mut LocationBarState;

/// The host's drop action for an ancestor crumb, by its id.
pub(crate) type ActionFn<S> = Rc<dyn Fn(&S, &str, &DropOffer, Mods) -> Option<DropAction>>;

/// Routes what a state call raised through the bar's own intent handler.
pub(crate) fn route<S>(emit: &Emit<S>, s: &mut S, polled: Polled, tx: &mut Tx) {
    for intent in polled.intents {
        emit(s, intent, tx);
    }
}

/// The key of ancestor crumb `i` (its button and drop zone) under the bar keyed `key`.
pub fn crumb_key(key: &str, i: usize) -> String {
    format!("{key}-crumb-{i}")
}

/// The key of the entry's text box (what the suggestions hang under).
pub fn field_key(key: &str) -> String {
    format!("{key}-field")
}

/// The key of suggestion `i`.
pub fn option_key(key: &str, i: usize) -> String {
    format!("{key}-option-{i}")
}

pub struct LocationBar<'a, S> {
    key: String,
    look: &'a Look,
    trail: &'a Trail,
    state: &'a LocationBarState,
    access: Access<S>,
    emit: Emit<S>,
    dnd: Dnd<S>,
    content: &'static [&'static str],
    action: Option<ActionFn<S>>,
    open_in_new: bool,
    spring: Option<f64>,
    complete: Vec<Chord>,
    labels: Option<&'a LocationLabels>,
}

impl<'a, S: 'static> LocationBar<'a, S> {
    pub fn new(
        key: impl Into<String>,
        look: &'a Look,
        trail: &'a Trail,
        state: &'a LocationBarState,
        access: Access<S>,
        on_intent: impl Fn(&mut S, LocationIntent, &mut Tx) + 'static,
    ) -> Self {
        Self {
            key: key.into(),
            look,
            trail,
            state,
            access,
            emit: Rc::new(on_intent),
            dnd: Dnd::rune(),
            content: &[],
            action: None,
            open_in_new: true,
            spring: Some(SPRING_DELAY),
            complete: default_complete_keys(),
            labels: None,
        }
    }

    /// Attaches the crumbs' drop zones through `dnd` (default [`Dnd::rune`]).
    pub fn dnd(mut self, dnd: Dnd<S>) -> Self {
        self.dnd = dnd;
        self
    }

    pub fn content_types(mut self, mimes: &'static [&'static str]) -> Self {
        self.content = mimes;
        self
    }

    pub fn drop_action(
        mut self,
        f: impl Fn(&S, &str, &DropOffer, Mods) -> Option<DropAction> + 'static,
    ) -> Self {
        self.action = Some(Rc::new(f));
        self
    }

    /// Whether a middle click opens a crumb in a new tab or window (off in a file chooser).
    pub fn open_in_new(mut self, on: bool) -> Self {
        self.open_in_new = on;
        self
    }

    pub fn spring_delay(mut self, delay: Option<f64>) -> Self {
        self.spring = delay;
        self
    }

    pub fn complete_keys(mut self, keys: &[Chord]) -> Self {
        self.complete = keys.to_vec();
        self
    }

    /// What assistive technology hears ([`LocationLabels::default`] otherwise).
    pub fn labels(mut self, labels: &'a LocationLabels) -> Self {
        self.labels = Some(labels);
        self
    }

    pub fn build(self, cx: &mut Cx<S>, layer: &mut Vec<El>) -> El {
        let default_labels = LocationLabels::default();
        let labels = self.labels.unwrap_or(&default_labels);
        let ctx = Ctx {
            key: &self.key,
            look: self.look,
            trail: self.trail,
            state: self.state,
            access: self.access,
            emit: &self.emit,
            dnd: &self.dnd,
            content: self.content,
            action: self.action.as_ref(),
            open_in_new: self.open_in_new,
            spring: self.spring,
            complete: &self.complete,
            labels,
        };
        let body = if self.state.editing() {
            let entry = entry::view(&ctx);
            if self.state.suggestions_open() {
                let width = cx
                    .layout(&field_key(&self.key))
                    .or_else(|| cx.layout(&self.key))
                    .map_or(0.0, |r| r[2]);
                popup::view(&ctx, width, layer);
            }
            entry
        } else {
            crumbs::view(&ctx)
        };
        let mut bar = el()
            .key(self.key.clone())
            .role("navigation")
            .label(labels.bar.clone())
            .grow(1.0)
            .min_w(0.0)
            .h(ctx.row())
            .child(body);
        if !self.state.editing() && self.state.menu().is_some() {
            bar = bar.child(menu::view(&ctx, layer));
        }
        bar
    }
}

/// Tab and Shift+Tab.
fn default_complete_keys() -> Vec<Chord> {
    ["tab", "shift+tab"]
        .into_iter()
        .filter_map(|c| Chord::parse(c).ok())
        .collect()
}

/// What the bar's parts read as they build.
pub(crate) struct Ctx<'a, S> {
    pub key: &'a str,
    pub look: &'a Look,
    pub trail: &'a Trail,
    pub state: &'a LocationBarState,
    pub access: Access<S>,
    pub emit: &'a Emit<S>,
    pub dnd: &'a Dnd<S>,
    pub content: &'static [&'static str],
    pub action: Option<&'a ActionFn<S>>,
    pub open_in_new: bool,
    pub spring: Option<f64>,
    pub complete: &'a [Chord],
    pub labels: &'a LocationLabels,
}

impl<S> Ctx<'_, S> {
    /// The crumbs' (and the entry's) height: a bar control.
    pub fn row(&self) -> f32 {
        self.look.frame.control
    }

    /// The overflow bar's row under the crumbs.
    pub fn band(&self) -> f32 {
        let bar = self.bar_look();
        bar.thickness + 2.0 * bar.margin
    }

    pub fn menu_bottom(&self) -> f32 {
        self.row() + self.band()
    }

    pub fn bar_look(&self) -> BarLook {
        BarLook::app(self.look).horizontal()
    }
}
