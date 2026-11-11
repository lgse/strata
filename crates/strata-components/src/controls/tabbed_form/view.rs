//! The form's view: the tablist in Strata's underline look over the stacked panels.

use std::rc::Rc;

use rune::{El, Key, KeyEvent, TextStyle, Tx, col, el, row, text};
use strata_kit::look::Look;

use super::state::{FormPage, Mounting, TabbedFormState};

/// Strata's `.settings-action-tabs` (style.css): a tab at least 36 px tall (`min-height`, so
/// it grows with a large text size) with 6 px sides and 8 px after it, its label at 55 % until
/// hovered, focused or chosen, a 2 px underline the label's width, a 2 px ring 2 px inside the
/// tab at half the accent, and a rule under the strip at 35 % of the border colour.
pub const TAB_HEIGHT: f32 = 36.0;
pub const TAB_PAD_X: f32 = 6.0;
pub const TAB_GAP: f32 = 8.0;
pub const STRIP_PAD_X: f32 = 16.0;
pub const LABEL_REST_OPACITY: f32 = 0.55;
pub const UNDERLINE: f32 = 2.0;
pub const RING_ALPHA: f32 = 0.5;
pub const RULE_ALPHA: f32 = 0.35;
/// Strata's tab label weight (600).
pub const LABEL_WEIGHT: f32 = 600.0;
/// The error mark's diameter.
pub const MARK: f32 = 6.0;

/// What the form says to assistive technology.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormLabels {
    /// The tablist's name ("Action editor sections").
    pub tabs: String,
    /// The description of a tab whose page has errors.
    pub has_errors: String,
}

impl Default for FormLabels {
    fn default() -> Self {
        Self {
            tabs: "Sections".into(),
            has_errors: "Has errors".into(),
        }
    }
}

type SelectFn<S> = Rc<dyn Fn(&mut S, usize, &mut Tx)>;

/// A notebook of pages under a tablist (the action editor's General, Script and Behavior).
///
/// - Tabs are one Tab stop. Left and Right choose the previous or next page as they move
///   (wrapping), Home and End the first and last; a press chooses. Focus stays on the tab.
/// - Ctrl+Page Up and Ctrl+Page Down choose the previous or next page from anywhere: the host
///   routes them from its `on_key` with [`TabbedForm::window_key`] (a focused field passes them
///   on to the window, never to an ancestor). As in GtkNotebook, a keyboard on a tab moves to
///   the new tab, one on the page being hidden to the new page's first focusable element (the
///   first in the tree, not a group's `focus_selected` one, as `Tx::focus` picks; Rune keeps no
///   last-focused child), else to its tab, and one
///   elsewhere stays. The host's poll calls [`TabbedFormState::tick`] to carry it.
/// - A page with [`FormPage::has_errors`] shows a dot in the danger colour after its title, and
///   its tab is described as having errors and reports itself invalid.
pub struct TabbedForm<S> {
    key: String,
    get: fn(&S) -> &TabbedFormState,
    get_mut: fn(&mut S) -> &mut TabbedFormState,
    mounting: Mounting,
    labels: FormLabels,
    on_select: Option<SelectFn<S>>,
}

impl<S: 'static> TabbedForm<S> {
    pub fn new(
        key: impl Into<String>,
        get: fn(&S) -> &TabbedFormState,
        get_mut: fn(&mut S) -> &mut TabbedFormState,
    ) -> Self {
        Self {
            key: key.into(),
            get,
            get_mut,
            mounting: Mounting::default(),
            labels: FormLabels::default(),
            on_select: None,
        }
    }

    pub fn mounting(mut self, mounting: Mounting) -> Self {
        self.mounting = mounting;
        self
    }

    pub fn labels(mut self, labels: FormLabels) -> Self {
        self.labels = labels;
        self
    }

    /// Hear a page being chosen, after the state has changed.
    pub fn on_select(mut self, f: impl Fn(&mut S, usize, &mut Tx) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }

    /// The tablist's key.
    pub fn tabs_key(&self) -> String {
        format!("{}-tabs", self.key)
    }

    /// Page `i`'s tab.
    pub fn tab_key(&self, i: usize) -> String {
        format!("{}-tab-{i}", self.key)
    }

    /// Page `i`'s panel.
    pub fn panel_key(&self, i: usize) -> String {
        format!("{}-panel-{i}", self.key)
    }

    /// Ctrl+Page Up and Ctrl+Page Down: the previous or next page, wrapping. For the host's
    /// `on_key` while the form shows. True when it took the key.
    pub fn window_key(&self, s: &mut S, pages: usize, ev: &KeyEvent, tx: &mut Tx) -> bool {
        let m = ev.mods;
        if pages == 0 || !(m.ctrl || m.cmd) || m.shift || m.alt || m.logo {
            return false;
        }
        let step = match ev.key {
            Key::PageUp => pages - 1,
            Key::PageDown => 1,
            _ => return false,
        };
        let at = (self.get)(s).selected().min(pages - 1);
        let next = (at + step) % pages;
        if next == at {
            return true;
        }
        // GtkNotebook's `real_switch_page`: a keyboard on a tab moves to the new tab, one on
        // the page being hidden into the new page (its first focusable element, else its tab),
        // and one elsewhere stays. Only the focused key is known here, so a keyboard off the
        // tabs is carried only if the switch drops it (`TabbedFormState::tick`).
        let ring = tx.focus_visible() || tx.by_keys();
        if let Some(f) = tx.focused().map(str::to_owned) {
            if (0..pages).any(|j| f == self.tab_key(j)) {
                tx.focus(self.tab_key(next), ring);
            } else {
                let targets = vec![self.panel_key(next), self.tab_key(next)];
                (self.get_mut)(s).carry(targets, ring, tx);
            }
        }
        choose(self.get_mut, &self.on_select, s, next, tx);
        true
    }

    /// The tablist over the panels. `page` builds page `i`'s body: for every page when
    /// [`Mounting::Kept`], only the chosen one when [`Mounting::Rebuilt`].
    pub fn build(
        &self,
        s: &S,
        look: &Look,
        pages: &[FormPage],
        page: impl Fn(&S, usize) -> El,
    ) -> El {
        let state = (self.get)(s);
        let last = pages.len().saturating_sub(1);
        let chosen = state.selected().min(last);
        let visit = state.visits;
        let k = &look.kit;
        let n = pages.len();
        let tabs = pages
            .iter()
            .enumerate()
            .map(|(i, p)| self.tab(look, p, i, n, i == chosen));
        let strip = row()
            .key(self.tabs_key())
            .role("tablist")
            .label(self.labels.tabs.clone())
            .focus_group()
            .w_full()
            .px(STRIP_PAD_X)
            .items_center()
            .border_sides([0.0, 0.0, 1.0, 0.0], k.alpha(k.c.border_base, RULE_ALPHA))
            .children(tabs);
        // One grid cell holds every built panel, so a kept form is as tall as its tallest page.
        // Changes land at once: a switch neither fades the panels nor slides the form's height.
        let mut stack = row()
            .key(format!("{}-panels", self.key))
            .w_full()
            .equal_columns(1)
            .instant_changes();
        for (i, p) in pages.iter().enumerate() {
            let shown = i == chosen;
            if !shown && self.mounting == Mounting::Rebuilt {
                continue;
            }
            let mut body = page(s, i);
            if self.mounting == Mounting::Rebuilt {
                // A new key per visit: Rune keeps a viewport's offset for an element that
                // mounts again at the same path when the old one left in an instant commit
                // (Rune draft U-C20c-4), so a stable key would bring the old offset back.
                body = col()
                    .key(format!("{}-visit-{visit}", self.panel_key(i)))
                    .w_full()
                    .min_w(0.0)
                    .child(body);
            }
            let panel = col()
                .key(self.panel_key(i))
                .role("tabpanel")
                .label(p.title.clone())
                .grid_cell(0)
                .w_full()
                .min_w(0.0)
                .child(body);
            stack = stack.child(if shown {
                panel
            } else {
                panel.opacity(0.0).inert().a11y_hidden()
            });
        }
        col()
            .key(self.key.clone())
            .w_full()
            .child(strip)
            .child(stack)
    }

    fn tab(&self, look: &Look, p: &FormPage, i: usize, n: usize, chosen: bool) -> El {
        let k = &look.kit;
        let accent = k.c.text_accent;
        let font = TextStyle {
            weight: LABEL_WEIGHT,
            ..k.ty.ui
        };
        let mut label = text(p.title.clone())
            .font(font)
            .color(k.c.text_base)
            .a11y_hidden();
        if !chosen {
            label = label.group_opacity(LABEL_REST_OPACITY, 1.0);
        }
        // The label carries the underline, so it is as wide as the text (Strata's label
        // border, not GTK's tab indicator).
        let underline = row()
            .items_center()
            .border_sides(
                [0.0, 0.0, UNDERLINE, 0.0],
                if chosen { accent } else { k.alpha(accent, 0.0) },
            )
            .child(label);
        let (get_mut, on_select) = (self.get_mut, self.on_select.clone());
        let mut tab = row()
            .key(self.tab_key(i))
            .role("tab")
            .label(p.title.clone())
            .position_in_set(i + 1, n)
            .focusable(true)
            .focus_selected(chosen)
            .focus_follows()
            .focus_ring(2.0, -2.0, k.alpha(accent, RING_ALPHA))
            .hover_group_focus()
            .min_h(TAB_HEIGHT)
            .px(TAB_PAD_X)
            .mr(TAB_GAP)
            .gap(look.space.xs)
            .items_center()
            .child(underline)
            .on_press_tx(move |s: &mut S, tx: &mut Tx| choose(get_mut, &on_select, s, i, tx));
        if p.has_errors {
            tab = tab
                .description(self.labels.has_errors.clone())
                .invalid(true)
                .child(
                    el().key(format!("{}-mark", self.tab_key(i)))
                        .size(MARK, MARK)
                        .radius(MARK / 2.0)
                        .fill(k.c.danger.fg)
                        .a11y_hidden(),
                );
        }
        tab
    }
}

fn choose<S>(
    get_mut: fn(&mut S) -> &mut TabbedFormState,
    on_select: &Option<SelectFn<S>>,
    s: &mut S,
    i: usize,
    tx: &mut Tx,
) {
    if get_mut(s).selected() == i {
        return;
    }
    get_mut(s).select(i);
    if let Some(f) = on_select {
        f(s, i, tx);
    }
}
