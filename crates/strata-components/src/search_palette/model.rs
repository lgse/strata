//! The results model: which generation is current, what shows, what is highlighted, and the
//! state that follows. A port of Strata's `render_results` and `restore_selection`
//! (`src/ui/search/results.rs`), without the search, which is the host's. It knows no Rune.

use super::data::{Answer, PaletteItem, PaletteQuery};
use super::mode::PaletteMode;

/// What the palette's body shows.
///
/// Used by: hosts (to read the state), tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// An empty query in a mode that lists nothing for it: the mode's idle text.
    Idle(String),
    /// A query with nothing to show yet: "Searching…".
    Loading(String),
    /// Results show.
    Results,
    /// A finished query found nothing: the mode's no-results text.
    NoResults(String),
    /// The host failed this query ([`super::PaletteState::fail`]).
    Error(String),
    /// Nothing can be searched while the palette stays open
    /// ([`super::PaletteState::unavailable`]): the field takes no text.
    Unavailable(String),
}

impl Phase {
    /// The words the status shows, or `None` while results show.
    pub fn text(&self) -> Option<&str> {
        match self {
            Phase::Idle(t)
            | Phase::Loading(t)
            | Phase::NoResults(t)
            | Phase::Error(t)
            | Phase::Unavailable(t) => Some(t),
            Phase::Results => None,
        }
    }
}

/// How the list scrolls when an answer lands: Strata's `restore_scroll`, keyed on whether the
/// user has moved the highlight in this query (its `navigation_started`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Follow {
    /// Nothing to do: no list was showing, or nothing moved.
    Stay,
    /// A new query: back to the top, where its highlighted first result is.
    Top,
    /// The same query, the highlight untouched: keep the list's pixel offset, so results a
    /// batch puts above the rows in view show. Rows keyed by place keep it with no request.
    Keep,
    /// The same query, after the user moved the highlight: keep the highlighted row, at this
    /// index now, in view.
    Reveal(usize),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Model {
    generation: u64,
    /// The current generation's text.
    asked: String,
    /// The current generation has had an answer.
    answered: bool,
    /// The current generation's last answer was its last.
    done: bool,
    shown: Vec<PaletteItem>,
    /// The (trimmed) text the shown results answer.
    shown_text: Option<String>,
    highlighted: Option<usize>,
    /// The user moved the highlight since the query last changed (Strata's
    /// `navigation_started`).
    navigated: bool,
    partial: Option<String>,
    error: Option<String>,
    unavailable: Option<String>,
    /// A non-empty query has been asked since the palette opened: idle text says so.
    typed: bool,
    /// The count assistive technology hears, set at a query's first results and when it is
    /// done, not on every batch.
    announced: Option<usize>,
}

impl Model {
    /// Forgets everything: the palette opened or closed. The generation keeps counting, so
    /// an answer for the last open lands nowhere.
    pub fn reset(&mut self) {
        *self = Self {
            generation: self.generation.wrapping_add(1),
            ..Self::default()
        };
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn shown(&self) -> &[PaletteItem] {
        &self.shown
    }

    pub fn highlighted(&self) -> Option<usize> {
        self.highlighted.filter(|&i| i < self.shown.len())
    }

    pub fn set_highlighted(&mut self, i: Option<usize>) {
        self.highlighted = i.filter(|&i| i < self.shown.len());
    }

    pub fn partial(&self) -> Option<&str> {
        self.partial.as_deref()
    }

    pub fn announced(&self) -> Option<usize> {
        self.announced
    }

    /// Whether the current query is still running: the spinner.
    pub fn busy(&self) -> bool {
        !self.done && self.error.is_none() && self.unavailable.is_none()
    }

    fn idle(text: &str, mode: &PaletteMode) -> bool {
        mode.idle.is_some() && text.trim().is_empty()
    }

    /// Asks for `text`. The shown results stay until its answer lands (Strata), except that an
    /// empty query in a mode that lists nothing for it clears them at once.
    pub fn request(&mut self, text: &str, mode: &PaletteMode) -> PaletteQuery {
        self.generation = self.generation.wrapping_add(1);
        text.clone_into(&mut self.asked);
        self.answered = false;
        self.done = false;
        self.error = None;
        self.navigated = false;
        self.announced = None;
        if text.trim().is_empty() {
            if Self::idle(text, mode) {
                self.clear();
            }
        } else {
            self.typed = true;
        }
        PaletteQuery {
            generation: self.generation,
            text: text.to_owned(),
        }
    }

    fn clear(&mut self) {
        self.shown.clear();
        self.shown_text = None;
        self.highlighted = None;
        self.partial = None;
    }

    /// An answer landed, and how the list should scroll for it. One for another generation,
    /// or while nothing can be searched, is dropped (`None`).
    pub fn arrive(&mut self, answer: Answer, mode: &PaletteMode) -> Option<Follow> {
        if answer.generation != self.generation || self.unavailable.is_some() {
            return None;
        }
        self.answered = true;
        self.done = answer.done;
        self.error = None;
        if Self::idle(&self.asked, mode) {
            // Strata renders no results for an empty search query; the answer only ends the
            // spinner.
            return Some(Follow::Stay);
        }
        let had_list = !self.shown.is_empty();
        let old_highlight = self.highlighted();
        let trimmed = self.asked.trim().to_owned();
        let query_changed = self.shown_text.as_deref() != Some(trimmed.as_str());
        let highlight = if answer.items.is_empty() {
            None
        } else if query_changed {
            Some(0)
        } else {
            // The same query again: the highlight stays on its result by id, else at its
            // place (clamped), else on the first.
            let by_id = self
                .highlighted()
                .and_then(|i| self.shown.get(i))
                .and_then(|old| answer.items.iter().position(|it| it.id == old.id));
            Some(
                by_id
                    .or_else(|| self.highlighted.map(|i| i.min(answer.items.len() - 1)))
                    .unwrap_or(0),
            )
        };
        let changed = self.shown != answer.items || query_changed;
        let n = answer.items.len();
        self.shown = answer.items;
        self.shown_text = Some(trimmed);
        self.highlighted = highlight;
        self.partial = answer.partial;
        if answer.done || (self.announced.is_none() && n > 0) {
            self.announced = Some(n);
        }
        // A list that wasn't showing mounts afresh, at its top.
        let follow = if !had_list || n == 0 {
            Follow::Stay
        } else if query_changed {
            Follow::Top
        } else if !self.navigated {
            if changed { Follow::Keep } else { Follow::Stay }
        } else {
            match highlight {
                Some(i) if Some(i) != old_highlight || changed => Follow::Reveal(i),
                _ => Follow::Stay,
            }
        };
        Some(follow)
    }

    /// The host failed the current query.
    pub fn fail(&mut self, generation: u64, message: String) -> bool {
        if generation != self.generation || self.unavailable.is_some() {
            return false;
        }
        self.clear();
        self.answered = true;
        self.done = true;
        self.error = Some(message);
        true
    }

    /// Nothing can be searched until the palette closes.
    pub fn unavailable(&mut self, message: String) {
        self.clear();
        self.done = true;
        self.unavailable = Some(message);
    }

    /// Up (`-1`) or Down (`+1`): moves the highlight, stopping at the ends (Strata's
    /// `move_selection`). With nothing highlighted it lands on the first. False when there is
    /// nothing to move through.
    pub fn step(&mut self, delta: i32) -> bool {
        let n = self.shown.len();
        if n == 0 {
            return false;
        }
        self.navigated = true;
        let next = match self.highlighted() {
            Some(i) => (i as i64 + i64::from(delta)).clamp(0, n as i64 - 1) as usize,
            None => 0,
        };
        self.highlighted = Some(next);
        true
    }

    /// Highlights the result `id` (a right-click on its row).
    pub fn highlight_id(&mut self, id: &str) -> Option<usize> {
        let i = self.shown.iter().position(|it| it.id == id)?;
        self.highlighted = Some(i);
        Some(i)
    }

    pub fn phase(&self, mode: &PaletteMode) -> Phase {
        if let Some(m) = &self.unavailable {
            return Phase::Unavailable(m.clone());
        }
        if let Some(e) = &self.error {
            return Phase::Error(e.clone());
        }
        if let Some(idle) = mode.idle.as_ref().filter(|_| Self::idle(&self.asked, mode)) {
            let t = if self.typed {
                &idle.cleared
            } else {
                &idle.first
            };
            return Phase::Idle(t.clone());
        }
        if !self.shown.is_empty() {
            return Phase::Results;
        }
        if !self.answered || !self.done {
            return Phase::Loading(mode.searching.clone());
        }
        if self.asked.trim().is_empty() {
            Phase::NoResults(mode.no_results_empty.clone())
        } else {
            Phase::NoResults(mode.no_results.clone())
        }
    }
}
