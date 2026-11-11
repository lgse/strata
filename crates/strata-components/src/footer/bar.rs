//! [`Footer`]: the window footer's status row.

use std::fmt;
use std::rc::Rc;

use rune::{El, Tx, el, row, text};
use strata_kit::look::Look;

use super::count::Count;
use super::marks::{self, FilterMark};
use super::reference::ReferenceState;
use crate::collection::keyboard::RangeKind;

/// The F1 button's look (`.shortcut-footer-button`, `src/style.css`): transparent at rest; on
/// hover the accent at 15 %; pressed or checked the accent at 22 % with an accent border; a
/// 2 px accent ring inside its edge for keyboard focus; 4 px corners. Strata's hover border
/// (accent at 42 %) isn't drawn: Rune has no hover border colour (upstream #328).
pub const BUTTON_HOVER_ALPHA: f32 = 0.15;
pub const BUTTON_CHECKED_ALPHA: f32 = 0.22;
pub const BUTTON_RING: f32 = 2.0;
pub const BUTTON_RADIUS: f32 = 4.0;

/// The footer's words. [`FooterLabels::default`] is Strata's English.
#[derive(Clone)]
pub struct FooterLabels {
    /// The F1 button: "F1 Shortcuts", described "Show all file-view shortcuts (F1)". Strata
    /// writes the key from its key map.
    pub shortcuts: String,
    pub shortcuts_description: String,
    /// The clipboard badge: "Files on clipboard", described "Press Ctrl+V to paste into a
    /// supported directory."
    pub clipboard: String,
    pub clipboard_description: String,
    /// The 10X pill's text, name and caveat.
    pub tenxer: String,
    pub tenxer_name: String,
    pub tenxer_caveat: String,
    /// `filter: %{query}` and `search: %{query}`.
    pub filter: Rc<dyn Fn(&str) -> String>,
    pub search: Rc<dyn Fn(&str) -> String>,
    /// The range marks and their names: ("VISUAL", "Visual select"), ("UNSET", "Visual unset").
    pub visual: (String, String),
    pub unset: (String, String),
}

impl fmt::Debug for FooterLabels {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FooterLabels")
            .field("shortcuts", &self.shortcuts)
            .finish_non_exhaustive()
    }
}

impl Default for FooterLabels {
    fn default() -> Self {
        let caveat = "(experimental feature, under active development)";
        Self {
            shortcuts: "F1 Shortcuts".to_owned(),
            shortcuts_description: "Show all file-view shortcuts (F1)".to_owned(),
            clipboard: "Files on clipboard".to_owned(),
            clipboard_description: "Press Ctrl+V to paste into a supported directory.".to_owned(),
            tenxer: "10X".to_owned(),
            tenxer_name: format!("10xer mode {caveat}"),
            tenxer_caveat: caveat.to_owned(),
            filter: Rc::new(|q| format!("filter: {q}")),
            search: Rc::new(|q| format!("search: {q}")),
            visual: ("VISUAL".to_owned(), "Visual select".to_owned()),
            unset: ("UNSET".to_owned(), "Visual unset".to_owned()),
        }
    }
}

type Toggle<S> = (fn(&S) -> &ReferenceState, fn(&mut S) -> &mut ReferenceState);

/// The window footer: the F1 Shortcuts button at the left, then the search hit under the
/// cursor; at the right the clipboard badge, the filter or search mark, the range mark, the
/// flash, the count, the jobs element, the chord pill and the 10X pill. A footer prompt, while
/// open, takes the whole row. Everything but the button is data the host sets each view.
///
/// ```ignore
/// let footer = Footer::new("footer")
///     .shortcuts(Some((|s: &App| &s.reference, |s: &mut App| &mut s.reference)))
///     .count(Some(footer::summary(&s.listing(), &s.words)))
///     .clipboard(s.files_on_clipboard)
///     .tenxer(s.tenxer)
///     .prompt(prompt.build(s, &s.look, cx, &mut layer))
///     .build(s, &s.look);
/// ```
///
/// Keys under `key`: `<key>` (the row), `<key>-shortcuts`, `<key>-hit`, `<key>-clipboard`,
/// `<key>-mark`, `<key>-range`, `<key>-count`, `<key>-tenxer`.
///
/// Used by: hosts (Strata's browser windows and the portal chooser).
pub struct Footer<S> {
    key: String,
    shortcuts: Option<Toggle<S>>,
    count: Option<Count>,
    clipboard: bool,
    tenxer: bool,
    mark: Option<FilterMark>,
    hit: Option<String>,
    range: Option<RangeKind>,
    flash: Option<El>,
    chord: Option<El>,
    jobs: Option<El>,
    prompt: Option<El>,
    labels: FooterLabels,
}

impl<S: 'static> Footer<S> {
    /// An empty footer keyed `key`.
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            shortcuts: None,
            count: None,
            clipboard: false,
            tenxer: false,
            mark: None,
            hit: None,
            range: None,
            flash: None,
            chord: None,
            jobs: None,
            prompt: None,
            labels: FooterLabels::default(),
        }
    }

    /// The F1 Shortcuts button over the reference's state, or `None` while the user hides it
    /// (Show F1 Shortcuts button off: F1 still opens the reference). A press toggles the
    /// reference; it shows pressed while the reference is open.
    pub fn shortcuts(mut self, toggle: Option<Toggle<S>>) -> Self {
        self.shortcuts = toggle;
        self
    }

    /// The item and selection count ([`summary`](super::summary)).
    pub fn count(mut self, count: Option<Count>) -> Self {
        self.count = count;
        self
    }

    /// Shows the Files on clipboard badge: the host sets it while the clipboard holds a
    /// non-empty file list (from strata-platform's clipboard notices, X3).
    pub fn clipboard(mut self, on: bool) -> Self {
        self.clipboard = on;
        self
    }

    /// Shows the 10X pill: 10xer mode is on.
    pub fn tenxer(mut self, on: bool) -> Self {
        self.tenxer = on;
        self
    }

    /// The `filter:` or `search:` mark. The count shows the results' total meanwhile.
    pub fn mark(mut self, mark: Option<FilterMark>) -> Self {
        self.mark = mark;
        self
    }

    /// The search hit under the cursor, as a path relative to the searched folder.
    pub fn current_hit(mut self, path: Option<String>) -> Self {
        self.hit = path.filter(|p| !p.is_empty());
        self
    }

    /// The active 10xer range's mark: `VISUAL` or `UNSET`.
    pub fn range(mut self, kind: Option<RangeKind>) -> Self {
        self.range = kind;
        self
    }

    /// C18's flash (`filter_bar::flash`).
    pub fn flash(mut self, flash: Option<El>) -> Self {
        self.flash = flash;
        self
    }

    /// C18's chord pill (`filter_bar::chord::pill`), beside the 10X pill.
    pub fn chord(mut self, pill: Option<El>) -> Self {
        self.chord = pill;
        self
    }

    /// C16's jobs element, after the count.
    pub fn jobs(mut self, jobs: Option<El>) -> Self {
        self.jobs = jobs;
        self
    }

    /// C18's footer prompt (`FooterPrompt::build`): while given, it takes the whole row.
    pub fn prompt(mut self, prompt: Option<El>) -> Self {
        self.prompt = prompt;
        self
    }

    /// The footer's words.
    pub fn labels(mut self, labels: FooterLabels) -> Self {
        self.labels = labels;
        self
    }

    /// The F1 button's key.
    pub fn shortcuts_key(&self) -> String {
        format!("{}-shortcuts", self.key)
    }

    /// The footer, or `None` with nothing to show (no button, no status): Strata hides it.
    /// Its height is the look's bar control plus `space.xs` above and below, whatever it shows.
    pub fn build(self, s: &S, look: &Look) -> Option<El> {
        let k = &look.kit;
        let key = self.key.clone();
        let base = row()
            .key(key.clone())
            .w_full()
            .h(look.frame.control + 2.0 * look.space.xs)
            .py(look.space.xs)
            .pl(look.space.xs)
            .pr(look.space.lg)
            .gap(look.space.lg)
            .items_center()
            .fill(k.c.bg_deep)
            .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_base);
        if let Some(prompt) = self.prompt {
            return Some(base.child(prompt));
        }
        let button = self.shortcuts.map(|(get, get_mut)| {
            button(
                look,
                self.shortcuts_key(),
                &self.labels,
                get(s).is_open(),
                get_mut,
            )
        });
        let labels = &self.labels;
        let mut right: Vec<El> = Vec::new();
        if self.clipboard {
            right.push(marks::clipboard(
                look,
                format!("{key}-clipboard"),
                &labels.clipboard,
                &labels.clipboard_description,
            ));
        }
        if let Some(mark) = &self.mark {
            let words = match mark {
                FilterMark::Filter(q) => (labels.filter)(q),
                FilterMark::Search(q) => (labels.search)(q),
            };
            right.push(marks::filter(
                look,
                format!("{key}-mark"),
                words,
                mark.query(),
            ));
        }
        if let Some(kind) = self.range {
            let (words, name) = match kind {
                RangeKind::Select => &labels.visual,
                RangeKind::Unset => &labels.unset,
            };
            right.push(marks::range(look, format!("{key}-range"), words, name));
        }
        right.extend(self.flash);
        // A status: polite already, so its changes are heard without interrupting.
        if let Some(count) = &self.count {
            right.push(
                text(count.text.clone())
                    .key(format!("{key}-count"))
                    .role("status")
                    .label(count.text.clone())
                    .description(count.description.clone())
                    .font(k.ty.small)
                    .color(k.c.text_base),
            );
        }
        right.extend(self.jobs);
        right.extend(self.chord);
        if self.tenxer {
            right.push(marks::tenxer(
                look,
                format!("{key}-tenxer"),
                &labels.tenxer,
                &labels.tenxer_name,
                &labels.tenxer_caveat,
            ));
        }
        let hit = self
            .hit
            .as_deref()
            .map(|p| marks::hit(look, format!("{key}-hit"), p));
        if button.is_none() && hit.is_none() && right.is_empty() {
            return None;
        }
        Some(
            base.children(button)
                .children(hit)
                .child(el().grow(1.0).min_w(0.0))
                .children(right),
        )
    }
}

/// The F1 Shortcuts toggle button.
fn button<S: 'static>(
    look: &Look,
    key: String,
    labels: &FooterLabels,
    open: bool,
    get_mut: fn(&mut S) -> &mut ReferenceState,
) -> El {
    let k = &look.kit;
    let accent = k.c.text_accent;
    let b = row()
        .key(key)
        .role("button")
        .label(labels.shortcuts.clone())
        .description(labels.shortcuts_description.clone())
        .checked(open)
        .focusable(true)
        .h(look.frame.control)
        .px(look.space.sm)
        .items_center()
        .radius(look.round(BUTTON_RADIUS))
        .border(
            1.0,
            if open {
                accent
            } else {
                rune::Color::TRANSPARENT
            },
        )
        .hover(k.alpha(accent, BUTTON_HOVER_ALPHA))
        .press(k.alpha(accent, BUTTON_CHECKED_ALPHA))
        .focus_ring(BUTTON_RING, -BUTTON_RING, accent)
        .on_press_tx(move |s: &mut S, _tx: &mut Tx| get_mut(s).toggle())
        .child(
            text(labels.shortcuts.clone())
                .font(k.ty.small)
                .color(k.c.text_base)
                .a11y_hidden(),
        );
    if open {
        b.fill(k.alpha(accent, BUTTON_CHECKED_ALPHA))
    } else {
        b
    }
}
