//! The category tabs: a tablist of glyphs, one Tab stop, arrows moving and choosing.
//!
//! Its own tablist, as C19's and C20c's are their own: rune-kit's SegmentedControl is a
//! radio group of text labels at the pin (Rune draft U-C20c-1). Sharing the three is a
//! follow-up.

use rune::{El, Tx, row};
use strata_kit::look::Look;

use super::grid::{Metrics, glyph};
use super::state::Page;
use super::view::IconPicker;
use crate::controls::tabbed_form::{RING_ALPHA, UNDERLINE};

impl<S: 'static> IconPicker<S> {
    /// The tablist, or none with fewer than two tabs.
    pub(super) fn tabs(&self, s: &S, look: &Look, m: &Metrics) -> Option<El> {
        let st = (self.get)(s);
        let tabs = st.tabs();
        if tabs.len() < 2 {
            return None;
        }
        let k = &look.kit;
        let chosen = st.page();
        let n = tabs.len();
        let strip = row()
            .key(self.tabs_key())
            .role("tablist")
            .label(self.words.tabs.clone())
            .focus_group()
            .w_full()
            .items_center()
            .border_sides([1.0, 0.0, 0.0, 0.0], k.alpha(k.c.border_base, 0.35))
            .children(tabs.iter().enumerate().map(|(i, &page)| {
                let selected = page == chosen;
                let name = self.page_name(s, page);
                let art = self.page_glyph(s, page);
                let underline = if selected {
                    k.c.text_accent
                } else {
                    k.alpha(k.c.text_accent, 0.0)
                };
                let picker = self.clone();
                row()
                    .key(self.tab_key(s, page))
                    .role("tab")
                    .label(name.clone())
                    .help(name)
                    .position_in_set(i + 1, n)
                    .focusable(true)
                    .focus_selected(selected)
                    .focus_follows()
                    .focus_ring(2.0, -2.0, k.alpha(k.c.text_accent, RING_ALPHA))
                    .hover(k.c.hover)
                    .grow(1.0)
                    .h(m.tile)
                    .center()
                    .radius(k.radius.sm)
                    .border_sides([0.0, 0.0, UNDERLINE, 0.0], underline)
                    .child(art.map_or_else(rune::el, |c| glyph(look, &small(m), &c)))
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| picker.show(s, page, tx))
            }));
        Some(strip)
    }
}

/// A tab's glyph: an icon's size, an emoji at it too.
fn small(m: &Metrics) -> Metrics {
    Metrics {
        emoji: m.icon,
        ..*m
    }
}

impl<S: 'static> IconPicker<S> {
    /// Shows `page`, clearing the query; the grid scrolls to its top.
    pub(super) fn show(&self, s: &mut S, page: Page, tx: &mut Tx) {
        let st = (self.get_mut)(s);
        if st.page() == page {
            return;
        }
        st.select_page_in(page, Some((&self.grid_key(), tx)));
    }

    /// The next tab after the one shown (or before, `back`), wrapping: Ctrl+N and Ctrl+P, as
    /// GTK's chooser steps sections. While searching, the first (or last) tab.
    pub(super) fn step_tab(&self, s: &mut S, back: bool, tx: &mut Tx) -> bool {
        let tabs = (self.get)(s).tabs();
        if tabs.len() < 2 {
            return false;
        }
        let at = tabs.iter().position(|&p| p == (self.get)(s).page());
        let next = match (at, back) {
            (Some(i), false) => (i + 1) % tabs.len(),
            (Some(i), true) => (i + tabs.len() - 1) % tabs.len(),
            (None, false) => 0,
            (None, true) => tabs.len() - 1,
        };
        self.show(s, tabs[next], tx);
        true
    }
}
