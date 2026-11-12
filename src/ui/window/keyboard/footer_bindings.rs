// SPDX-License-Identifier: MIT

//! Footer prompt and filter wiring that follows the listing and the 10xer mode.

use std::{cell::Cell, rc::Rc};

use gtk::prelude::*;

use crate::{app::Browser, ui::preview::PreviewDrawer};

use super::{Dispatcher, prompts};

/// Leaving 10xer mode ends preview key ownership but keeps the drawer open.
pub(super) fn release_preview_keys_on_mode_exit(
    window: &gtk::Widget,
    preview: &PreviewDrawer,
    browser: &std::rc::Weak<Browser>,
    input: Option<super::PaneInput>,
) {
    let preview = preview.clone();
    let browser = browser.clone();
    crate::ui::preferences::PreferenceManager::shared().bind_preference(
        window,
        crate::ui::preferences::PreferenceManager::tenxer_mode,
        move |window, enabled| {
            if enabled || input.as_ref().is_some_and(|input| !input.get()) {
                return;
            }
            if preview.owns_focus(window.root().and_then(|root| root.focus()).as_ref())
                && let Some(browser) = browser.upgrade()
            {
                browser.focus_active();
            }
        },
    );
}

/// The **f** and **s** prompts filter and search as they are typed, and the
/// footer reports the focused listing's filter or search in every key map.
pub(super) fn bind_footer_filter(dispatcher: &Dispatcher) {
    let view = dispatcher.view.downgrade();
    let descent = dispatcher.filter_descent.clone();
    let cursor_moved = dispatcher.filter_cursor_moved.clone();
    dispatcher
        .shortcuts
        .connect_prompt_changed(move |kind, text| {
            descent.take();
            cursor_moved.set(false);
            let Some(view) = view.upgrade() else {
                return;
            };
            match kind {
                crate::ui::tenxer_mode::Prompt::Filter => view.set_listing_filter(&text),
                crate::ui::tenxer_mode::Prompt::Search => view.set_listing_search(&text),
                _ => {}
            }
        });
    let view = dispatcher.view.downgrade();
    dispatcher.shortcuts.observe_filter(move || {
        view.upgrade()
            .map_or(Some(None), |view| view.filter_status())
    });
    let shortcuts = dispatcher.shortcuts.clone();
    dispatcher.view.set_footer_filter_focus(Rc::new(move || {
        shortcuts.prompt_has_focus()
            && matches!(
                shortcuts.open_prompt_kind(),
                Some(
                    crate::ui::tenxer_mode::Prompt::Filter | crate::ui::tenxer_mode::Prompt::Search
                )
            )
    }));
    let shortcuts = dispatcher.shortcuts.clone();
    let view = dispatcher.view.downgrade();
    let descent = dispatcher.filter_descent.clone();
    dispatcher
        .view
        .connect_filter_results_changed(Rc::new(move || {
            shortcuts.refresh_filter();
            if let Some(view) = view.upgrade() {
                prompts::finish_filter_descent(&view, &shortcuts, &descent);
            }
        }));
    let shortcuts = dispatcher.shortcuts.clone();
    dispatcher
        .view
        .connect_search_selection_changed(Rc::new(move || shortcuts.schedule_filter_refresh()));
    // Over a fill, motion and Space move the result cursor by focus alone, and
    // GTK may land that focus after the selection change has been reported. In
    // Columns the footer also follows the focused column.
    let shortcuts = dispatcher.shortcuts.clone();
    let view = dispatcher.view.downgrade();
    let descent = dispatcher.filter_descent.clone();
    let pending = dispatcher.bookmark_pending.clone();
    let handler = dispatcher.window.connect_focus_widget_notify(move |_| {
        if pending.get()
            && view
                .upgrade()
                .is_none_or(|view| !view.item_view_has_focus())
        {
            shortcuts.cancel_chord();
        }
        shortcuts.schedule_filter_refresh();
        if let Some(view) = view.upgrade() {
            prompts::cancel_stale_filter_descent(&view, &shortcuts, &descent);
        }
    });
    // A closed tab's view unrealizes while its window lives on.
    let window = dispatcher.window.downgrade();
    let handler = Cell::new(Some(handler));
    dispatcher.view.widget().connect_unrealize(move |_| {
        if let (Some(window), Some(handler)) = (window.upgrade(), handler.take()) {
            window.disconnect(handler);
        }
    });
}

/// Enter reaches the prompt only after an input method commits and passes it
/// back; otherwise the key handler submits the prompt.
pub(super) fn submit_prompt_on_activate(
    dispatcher: &Rc<Dispatcher>,
    browser: std::rc::Weak<Browser>,
) {
    let entered = Rc::downgrade(dispatcher);
    dispatcher.shortcuts.connect_prompt_activated(move || {
        if let (Some(dispatcher), Some(browser)) = (entered.upgrade(), browser.upgrade()) {
            dispatcher.submit_prompt(&browser);
        }
    });
}

/// A `/` reaches a filter field's input method like any typed key; a lone `/`
/// it commits enters the matching folder instead of reaching the query.
pub(super) fn descend_on_committed_slash(dispatcher: &Rc<Dispatcher>) {
    let weak = Rc::downgrade(dispatcher);
    let accepts = weak.clone();
    let descends = weak.clone();
    dispatcher.shortcuts.connect_prompt_path_separator(
        move || {
            accepts
                .upgrade()
                .is_some_and(|dispatcher| dispatcher.prompt_accepts_slash())
        },
        move || {
            if let Some(dispatcher) = descends.upgrade() {
                dispatcher.begin_filter_descent(None);
            }
        },
    );
    let accepts = weak.clone();
    dispatcher
        .view
        .connect_filter_descent(crate::ui::browser::FilterDescentHandler {
            accepts: Box::new(move || {
                accepts
                    .upgrade()
                    .is_some_and(|dispatcher| dispatcher.filter_entry_accepts_slash())
            }),
            descend: Box::new(move || {
                if let Some(dispatcher) = weak.upgrade() {
                    dispatcher.descend_from_filter_entry();
                }
            }),
        });
}

pub(super) fn bind_prompt_hints(dispatcher: &Dispatcher) {
    use crate::ui::tenxer_mode::Prompt;
    let revision = dispatcher.destination_revision.clone();
    dispatcher.shortcuts.connect_prompt_reset(move || {
        revision.set(revision.get().wrapping_add(1));
    });
    let hints: Vec<_> = [
        Prompt::Go,
        Prompt::Create,
        Prompt::Rename,
        Prompt::MoveTo,
        Prompt::CopyTo,
        Prompt::ExtractTo,
    ]
    .into_iter()
    .map(|kind| (kind, dispatcher.shortcuts.prompt_sink(kind)))
    .collect();
    let revision = dispatcher.destination_revision.clone();
    dispatcher.shortcuts.connect_prompt_changed(move |kind, _| {
        revision.set(revision.get().wrapping_add(1));
        if let Some((_, hint)) = hints.iter().find(|(hinted, _)| *hinted == kind) {
            hint.show(None, None);
        }
    });
}

pub(super) fn bind_candidate_prompts(dispatcher: &Dispatcher) {
    let shortcuts = dispatcher.shortcuts.clone();
    let history = dispatcher.history.clone();
    let browser = Rc::downgrade(&dispatcher.view.browser());
    let destinations = dispatcher.destinations.clone();
    let targets = dispatcher.destination_targets.clone();
    dispatcher.shortcuts.connect_prompt_changed(move |kind, _| {
        shortcuts.forget_candidate_step();
        if let Some(browser) = browser.upgrade() {
            prompts::show_history_candidates(&shortcuts, &history, &browser, kind);
            prompts::show_folder_candidates(
                &shortcuts,
                &destinations,
                &browser,
                &targets.borrow(),
                kind,
            );
        }
    });
    let shortcuts = dispatcher.shortcuts.clone();
    let view = dispatcher.view.downgrade();
    let targets = dispatcher.destination_targets.clone();
    let revision = dispatcher.destination_revision.clone();
    dispatcher
        .shortcuts
        .connect_candidate_activated(move |path| {
            let Some(view) = view.upgrade() else {
                return;
            };
            let Some(kind) = shortcuts.open_prompt_kind() else {
                return;
            };
            if kind.holds_targets() {
                prompts::send_to_destination(&view, &shortcuts, &targets, &revision, kind, path);
                return;
            }
            if !kind.picks_history() && kind != crate::ui::tenxer_mode::Prompt::Go {
                return;
            }
            prompts::return_to_listing(&shortcuts, &view, &view.browser());
            view.keyboard_navigation();
            view.browser()
                .navigate_with_selection(crate::model::Location::local(path), true);
        });
}

pub(super) fn clear_find_on_mode_exit(
    window: &gtk::Widget,
    dispatcher: &Dispatcher,
    browser: &std::rc::Weak<Browser>,
) {
    let shortcuts = dispatcher.shortcuts.clone();
    let open_with = dispatcher.open_with.clone();
    let armed_actions = dispatcher.armed_actions.clone();
    let bookmark_pending = dispatcher.bookmark_pending.clone();
    let browser = browser.clone();
    let input = dispatcher.input.clone();
    let primed = Cell::new(false);
    crate::ui::preferences::PreferenceManager::shared().bind_preference(
        window,
        crate::ui::preferences::PreferenceManager::tenxer_mode,
        move |window, enabled| {
            let starting = !primed.replace(true);
            if enabled || starting {
                shortcuts.refresh_filter();
                return;
            }
            open_with.invalidate();
            if bookmark_pending.get() {
                shortcuts.cancel_chord();
            }
            armed_actions.take();
            shortcuts.refresh_filter();
            let focus = window.root().and_then(|root| root.focus());
            let prompt_focused = shortcuts.prompt_has_focus();
            shortcuts.dismiss_prompt();
            // Focus left with the prompt, or nowhere: give it back to the listing.
            let focus_lost = prompt_focused || focus.is_none();
            let inactive_pane = input.as_ref().is_some_and(|input| !input.get());
            if !window.is_mapped() || !focus_lost || inactive_pane {
                return;
            }
            if let Some(browser) = browser.upgrade() {
                browser.focus_active();
            }
        },
    );
}
