// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, glib, prelude::*};

pub(in crate::ui) use candidates::CandidateKeys;
use catalog::{ReferenceView, WeakReferenceView, rebuild_reference};
use indicators::{ChordIndicator, CurrentHit, apply_experimental_label, update_visual_mode};
use prompt::{PromptBar, WeakPromptBar};
use reference::{ReferenceArea, ReferenceSession};
#[cfg(test)]
use status::selection_details;
use status::{refresh_paste_availability, update_item_count, watch_status_widget};

#[cfg(test)]
use super::keys::KeyPress;
use super::{
    browser::FilterStatus,
    browser_modes::BrowserMode,
    shortcut_reference::{ChooserScope, ReferenceScope},
    tenxer_mode::Chord,
};

type ChordListener = Box<dyn Fn(Option<Chord>)>;
type PromptResetListeners = Rc<RefCell<Vec<Box<dyn Fn()>>>>;
/// `None` while the view is busy rebuilding; the footer then retries on idle.
type FilterSource = Rc<RefCell<Option<Rc<dyn Fn() -> Option<Option<FilterStatus>>>>>>;

const FEEDBACK_FLASH: Duration = Duration::from_millis(2_000);

#[derive(Clone)]
pub(super) struct ShortcutFooter {
    root: gtk::Stack,
    status: gtk::Box,
    paste: gtk::Label,
    count: gtk::Label,
    show_hints: Rc<Cell<bool>>,
    more: gtk::ToggleButton,
    panel: gtk::Box,
    session: ReferenceSession,
    reference: gtk::Box,
    categories: gtk::Box,
    sidebar: gtk::ScrolledWindow,
    search: gtk::Entry,
    selected_category: Rc<RefCell<String>>,
    active_area: Rc<Cell<ReferenceArea>>,
    scroll: gtk::ScrolledWindow,
    status_widgets: Rc<RefCell<Vec<gtk::Widget>>>,
    tag: gtk::Label,
    feedback: gtk::Label,
    feedback_epoch: Rc<Cell<u64>>,
    prompt: PromptBar,
    chords: ChordIndicator,
    visual: gtk::Label,
    filter: gtk::Label,
    current: CurrentHit,
    filter_source: FilterSource,
    filter_retry: Rc<Cell<bool>>,
    disposed: Rc<Cell<bool>>,
    observed: Rc<RefCell<std::rc::Weak<crate::app::Browser>>>,
    reference_scope: Rc<Cell<ReferenceScope>>,
}

/// The chrome 10xer mode marks, cleared when the mode turns off.
struct TenxerChrome {
    tag: glib::WeakRef<gtk::Label>,
    panel: glib::WeakRef<gtk::Box>,
    feedback: glib::WeakRef<gtk::Label>,
    prompt: WeakPromptBar,
    chords: ChordIndicator,
    view: WeakReferenceView,
    primed: Cell<bool>,
}

impl TenxerChrome {
    fn apply(&self, enabled: bool) {
        let Some(tag) = self.tag.upgrade() else {
            return;
        };
        if !self.view.reference_alive() {
            return;
        }
        let starting = !self.primed.replace(true);
        apply_experimental_label(&tag, enabled);
        if let Some(panel) = self.panel.upgrade() {
            if enabled {
                panel.add_css_class("tenxer-active");
            } else {
                panel.remove_css_class("tenxer-active");
            }
        }
        if !starting && !enabled {
            self.clear_session_marks();
        }
        if let Some(view) = self.view.upgrade() {
            rebuild_reference(&view);
        }
    }

    fn clear_session_marks(&self) {
        let Some(feedback) = self.feedback.upgrade() else {
            return;
        };
        feedback.set_text("");
        feedback.set_visible(false);
        if let Some(prompt) = self.prompt.upgrade() {
            prompt.close();
        }
        self.chords.clear();
    }
}

impl ShortcutFooter {
    pub fn widget(&self) -> &gtk::Stack {
        &self.root
    }

    pub fn set_activity(&self, widget: &impl IsA<gtk::Widget>) {
        let widget = widget.as_ref().clone();
        self.status.insert_child_after(&widget, Some(&self.count));
        self.status_widgets.borrow_mut().push(widget.clone());
        watch_status_widget(&widget, &self.status_widgets, &self.root);
    }

    fn reference_view(&self) -> ReferenceView {
        ReferenceView {
            reference: self.reference.clone(),
            categories: self.categories.clone(),
            search: self.search.clone(),
            scroll: self.scroll.clone(),
            selected: self.selected_category.clone(),
            scope: self.reference_scope.clone(),
        }
    }

    pub fn bind_preferences(&self, manager: &super::preferences::PreferenceManager) {
        let chrome = TenxerChrome {
            tag: self.tag.downgrade(),
            panel: self.panel.downgrade(),
            feedback: self.feedback.downgrade(),
            prompt: self.prompt.downgrade(),
            chords: self.chords.clone(),
            view: self.reference_view().downgrade(),
            primed: Cell::new(false),
        };
        manager.bind_preference(
            &self.root,
            super::preferences::PreferenceManager::tenxer_mode,
            move |_, enabled| chrome.apply(enabled),
        );
        let view = self.reference_view().downgrade();
        let primed = Cell::new(false);
        manager.bind_preference(
            &self.root,
            // Labels leave out the keys a user added that type-to-search types.
            |preferences| (preferences.keymap_revision(), preferences.type_to_search()),
            move |_, _| {
                if !primed.replace(true) {
                    return;
                }
                if let Some(view) = view.upgrade() {
                    rebuild_reference(&view);
                }
            },
        );
        let show_hints = self.show_hints.clone();
        let more = self.more.downgrade();
        manager.on_keybinding_hints_changed(&self.root, move |_, enabled| {
            show_hints.set(enabled);
            if let Some(more) = more.upgrade() {
                more.set_visible(enabled);
            }
        });
    }

    pub fn connect_clipboard(&self, clipboard: &gdk::Clipboard) -> glib::SignalHandlerId {
        let label = self.paste.downgrade();
        let generation = Rc::new(Cell::new(0));
        refresh_paste_availability(clipboard, &label, &generation);
        clipboard.connect_changed(move |clipboard| {
            refresh_paste_availability(clipboard, &label, &generation);
        })
    }

    pub fn observe_browser(&self, browser: &Rc<crate::app::Browser>) {
        self.observed.replace(Rc::downgrade(browser));
        self.refresh_filter();
        let weak_browser = Rc::downgrade(browser);
        let footer = self.clone();
        browser.observe(move |event| {
            if matches!(
                event,
                crate::app::BrowserEvent::NavigationStarting { .. }
                    | crate::app::BrowserEvent::SelectionSetChanged { .. }
            ) {
                footer.clear_feedback();
            }
            if weak_browser.upgrade().is_some() {
                footer.refresh_filter();
            }
        });
    }

    /// Refreshes once on idle, after focus settles on the cursor row.
    pub(in crate::ui) fn schedule_filter_refresh(&self) {
        if self.disposed.get() || self.filter_retry.replace(true) {
            return;
        }
        let footer = self.clone();
        glib::idle_add_local_once(move || {
            footer.filter_retry.set(false);
            footer.refresh_filter();
        });
    }

    /// Reports `source`'s filter in place of the directory count while its
    /// results are displayed, in every key map; the query mark and hit path
    /// show in 10xer mode and for a subfolder search in either map. Call
    /// [`Self::refresh_filter`] when it changes.
    pub(in crate::ui) fn observe_filter(
        &self,
        source: impl Fn() -> Option<Option<FilterStatus>> + 'static,
    ) {
        self.filter_source.replace(Some(Rc::new(source)));
        self.refresh_filter();
    }

    /// The source's filter, or `None` while it is busy.
    fn read_filter(&self) -> Option<Option<FilterStatus>> {
        let source = self.filter_source.borrow().clone();
        match source {
            Some(source) => source(),
            None => Some(None),
        }
    }

    pub(in crate::ui) fn refresh_filter(&self) {
        if self.disposed.get() {
            return;
        }
        let Some(status) = self.read_filter() else {
            self.schedule_filter_refresh();
            return;
        };
        // The default map keeps the pane's filter field on screen, but a
        // search started by a direct key has no field there.
        let named = status
            .as_ref()
            .filter(|status| status.search || crate::ui::tenxer_mode::chrome_suppressed());
        self.show_filter_mark(named);
        self.current
            .set(named.and_then(|status| status.current.as_deref()));
        let browser = self.observed.borrow().upgrade();
        if let Some(browser) = browser {
            self.show_listing_status(&browser, status.as_ref());
        }
    }

    pub(in crate::ui) fn dispose(&self) {
        self.disposed.set(true);
        self.filter_source.borrow_mut().take();
        self.observed.replace(std::rc::Weak::new());
    }

    fn show_filter_mark(&self, named: Option<&FilterStatus>) {
        let Some(status) = named else {
            self.filter.set_visible(false);
            return;
        };
        let mark = if status.search {
            rust_i18n::t!("search: %{query}", query = status.query)
        } else {
            rust_i18n::t!("filter: %{query}", query = status.query)
        };
        self.filter.set_text(&mark);
        crate::ui::accessibility::set_description(&self.filter, Some(&status.query));
        self.filter.set_visible(true);
    }

    fn show_listing_status(
        &self,
        browser: &Rc<crate::app::Browser>,
        status: Option<&FilterStatus>,
    ) {
        update_item_count(&self.count, browser, status);
        // A range over results is theirs; the hidden directory's never shows.
        let visual = match status {
            Some(status) if status.visual.is_some() => status.visual,
            _ => browser.visual_kind(),
        };
        update_visual_mode(&self.visual, visual);
    }

    #[cfg(test)]
    pub(in crate::ui) fn tag_visible(&self) -> bool {
        self.tag.is_visible()
    }

    pub fn set_mode(&self, mode: BrowserMode) {
        let scope = self.reference_scope.get();
        self.set_reference_scope(ReferenceScope { mode, ..scope });
    }

    pub(in crate::ui) fn set_chooser(&self, chooser: ChooserScope) {
        let scope = self.reference_scope.get();
        self.set_reference_scope(ReferenceScope {
            chooser: Some(chooser),
            ..scope
        });
    }

    fn set_reference_scope(&self, scope: ReferenceScope) {
        self.reference_scope.set(scope);
        rebuild_reference(&self.reference_view());
    }

    pub(in crate::ui) fn show_feedback(&self, text: &str) {
        let epoch = self.feedback_epoch.get().wrapping_add(1);
        self.feedback_epoch.set(epoch);
        self.feedback.set_text(text);
        let visible = !text.is_empty();
        self.feedback.set_visible(visible);
        if !visible {
            return;
        }
        let epochs = self.feedback_epoch.clone();
        let feedback = self.feedback.downgrade();
        glib::timeout_add_local_once(FEEDBACK_FLASH, move || {
            if epochs.get() != epoch {
                return;
            }
            if let Some(feedback) = feedback.upgrade() {
                feedback.set_text("");
                feedback.set_visible(false);
            }
        });
    }

    pub(in crate::ui) fn clear_feedback(&self) {
        self.feedback_epoch
            .set(self.feedback_epoch.get().wrapping_add(1));
        self.feedback.set_text("");
        self.feedback.set_visible(false);
    }

    #[cfg(test)]
    pub(in crate::ui) fn feedback_text(&self) -> String {
        self.feedback.text().to_string()
    }

    #[cfg(test)]
    pub(in crate::ui) fn dismiss_feedback(&self) {
        self.clear_feedback();
    }

    #[cfg(test)]
    pub(in crate::ui) fn filter_mark(&self) -> Option<String> {
        self.filter
            .is_visible()
            .then(|| self.filter.text().to_string())
    }

    #[cfg(test)]
    pub(in crate::ui) fn count_text(&self) -> (String, String) {
        (
            self.count.text().to_string(),
            self.count.tooltip_text().unwrap_or_default().to_string(),
        )
    }
}

mod build;
mod candidates;
mod catalog;
mod chord_panel;
mod indicators;
mod prompt;
mod reference;
mod reference_keys;
mod status;
#[cfg(test)]
mod tests;
