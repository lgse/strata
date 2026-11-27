// SPDX-License-Identifier: MIT

pub(crate) mod accept;
pub(crate) mod download;
pub(crate) mod floating;
pub(crate) mod geometry;
pub(crate) mod image_conversion;
mod keys;
pub(crate) use keys::{declines_key, refuse_key};
pub(crate) mod options;
pub(crate) mod source;
pub(crate) mod state;
pub(crate) mod tenxer;
mod view;

#[cfg(test)]
mod tests;

use rune::Tx;
use strata_components::collection::element::Activation;
use strata_fs::app::BrowserEvent;

use self::{
    image_conversion::ConversionConsent,
    source::ChooserFileSource,
    state::{ActiveChooser, DestinationHost},
};
use crate::{
    App,
    commands::{Action, Holds},
    filter,
    portal::{ChooserKind, ChooserRequest, Completion, DestinationCheck},
    shell,
    slots::{Later, Listener, Part, Place, Setup},
};

pub(crate) const NAME_KEY: &str = "chooser-name";

pub(crate) enum ChooserPrompt {
    Overwrite(DestinationCheck),
    ConvertImage(ConversionConsent),
}

pub(crate) struct ChooserDialog {
    pub(crate) prompt: Option<ChooserPrompt>,
}

impl ChooserDialog {
    pub(crate) fn open(&mut self, prompt: ChooserPrompt) {
        self.prompt = Some(prompt);
    }

    pub(crate) fn close(&mut self) {
        self.prompt = None;
    }
}

impl Part for ChooserDialog {
    const PLACE: Place<App> = Place::Dialogs(view::build_dialog);

    fn new(_: &Setup<'_>) -> Self {
        Self { prompt: None }
    }

    fn keyboard(app: &App) -> Option<Holds> {
        app.chooser_dialog
            .prompt
            .is_some()
            .then_some(Holds::Modal { field: false })
    }
}

pub(crate) struct Chooser {
    pub(crate) later: Later<App>,
    pub(crate) active: Option<ActiveChooser>,
    /// The floating chooser a browser window opens for Copy to, Move to and Extract to.
    pub(crate) floating: floating::Floating,
}

impl Chooser {
    pub(crate) fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub(crate) fn allows_multiple(&self) -> bool {
        self.active
            .as_ref()
            .is_none_or(ActiveChooser::allows_multiple)
    }

    pub(crate) fn allows_create(&self) -> bool {
        self.active
            .as_ref()
            .is_none_or(ActiveChooser::allows_create)
    }

    /// Shows `message` in the footer's inline error.
    pub(crate) fn show_error(&mut self, message: impl Into<String>) {
        if let Some(active) = self.active.as_mut() {
            active.show_error(message);
        }
    }

    pub(crate) fn end_check(&mut self, generation: u64) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        active.destination_check = false;
        if active.completion.is_none() || active.accept_generation != generation {
            return false;
        }
        active.accept_sensitive = true;
        true
    }

    /// The portal request's token, which names its window, whether or not it has finished.
    pub(crate) fn token(&self) -> Option<String> {
        self.active
            .as_ref()
            .map(|active| active.request.token.clone())
    }

    pub(crate) fn is_confined(&self) -> bool {
        self.active.as_ref().is_some_and(ActiveChooser::is_confined)
    }

    pub(crate) fn chooses_directory(&self) -> bool {
        self.active.as_ref().is_some_and(|active| {
            matches!(
                active.request.kind,
                ChooserKind::Open {
                    directory: true,
                    ..
                }
            )
        })
    }

    pub(crate) fn name_focused(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.has_name && active.name.focused)
    }
}

impl Part for Chooser {
    const PLACE: Place<App> = Place::Footer(view::build_footer);
    const COMMANDS: &'static [Action<App>] = keys::COMMANDS;

    fn new(setup: &Setup<'_>) -> Self {
        Self {
            later: setup.later.clone(),
            active: None,
            floating: floating::Floating::default(),
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, browser, heard| {
            if !browser.is_chooser_mode() {
                return;
            }
            match event {
                BrowserEvent::OpenRequested { location } => {
                    let location = location.clone();
                    heard.then(move |app: &mut App, tx: &mut Tx| {
                        accept::activate_file(app, &location, tx);
                        true
                    });
                }
                BrowserEvent::FocusChanged { .. }
                | BrowserEvent::SelectionSetChanged { .. }
                | BrowserEvent::SelectionSynced { .. } => {
                    heard.then(|app: &mut App, _: &mut Tx| {
                        state::update_selected_filename(app);
                        false
                    });
                }
                _ => {}
            }
        }))
    }

    fn poll(app: &mut App, _clock: f64, tx: &mut Tx) -> bool {
        let tenxer = shell::preferences(app).tenxer_mode;
        let confined = app.chooser.is_confined();
        let Some(active) = app.chooser.active.as_mut() else {
            return false;
        };
        if !std::mem::take(&mut active.focus_initial) {
            return false;
        }
        if tenxer {
            if active.has_name {
                active.name.set_selection(0, 0);
            }
            return true;
        }
        if active.has_name {
            active.name.focused = true;
            active.select_name_stem();
            tx.focus_when_shown(NAME_KEY, false);
        } else if !confined {
            tx.focus_when_shown("header-sidebar-toggle", true);
        }
        true
    }

    fn keyboard(app: &App) -> Option<Holds> {
        let active = app.chooser.active.as_ref()?;
        if active.dropdown_open() {
            return Some(Holds::Menu);
        }
        if active.has_name && active.name.focused {
            return Some(Holds::Field);
        }
        None
    }
}

/// Whether the chooser's footer shows `message` as its error.
#[cfg(feature = "qa")]
pub(crate) fn shows_error(app: &App, message: &str) -> bool {
    app.chooser
        .active
        .as_ref()
        .and_then(|active| active.error.as_deref())
        == Some(message)
}

pub(crate) fn mount_request(
    app: &mut App,
    request: ChooserRequest,
    source: std::rc::Rc<ChooserFileSource>,
    (destination_host, completion): (Option<DestinationHost>, Completion),
) {
    crate::columns::use_chooser_widths(app);
    crate::list::use_chooser_widths(app);
    app.chooser.active = Some(ActiveChooser::new(
        request,
        source,
        destination_host,
        completion,
    ));
}

pub(crate) fn cancel(app: &mut App, tx: &mut Tx) -> bool {
    accept::cancel(app, tx)
}

pub(crate) fn confirms_focused(app: &mut App, tx: &mut Tx) -> bool {
    if !app.browser.is_chooser_mode() {
        return false;
    }
    let Some((depth, position, _)) = app.browser.focused_item() else {
        return false;
    };
    confirm_key_activation(app, (depth, position), Activation::Key, tx)
}

pub(crate) fn clicked_entry_in_chooser(app: &mut App) {
    if shell::preferences(app).tenxer_mode {
        return;
    }
    let save_file = app
        .chooser
        .active
        .as_ref()
        .is_some_and(|active| matches!(active.request.kind, ChooserKind::SaveFile { .. }));
    if !save_file {
        return;
    }
    commit_load_cursor(app);
    if let Some(active) = app.chooser.active.as_mut() {
        active.filename_selection.clear();
    }
    state::update_selected_filename(app);
}

fn commit_load_cursor(app: &mut App) {
    if filter::targets::selected(app).is_some() || !app.browser.selection_is_load_cursor() {
        return;
    }
    let Some((depth, position, _)) = app.browser.focused_item() else {
        return;
    };
    app.browser.commit_selection();
    let selected = app.browser.selected_positions(depth);
    app.browser.set_selection(depth, &selected, Some(position));
}

pub(crate) fn confirm_key_activation(
    app: &mut App,
    (depth, position): (usize, usize),
    how: Activation,
    tx: &mut Tx,
) -> bool {
    if how != Activation::Key || !app.chooser.is_active() {
        return false;
    }
    let Some(entry) = app.browser.entry_at(depth, position) else {
        return false;
    };
    if entry.is_directory() {
        return false;
    }
    accept::confirm_file(app, &entry, tx);
    true
}

pub(crate) fn choose_filter(app: &mut App, chosen_label: &str, _tx: &mut Tx) {
    let selected = {
        let Some(active) = app.chooser.active.as_mut() else {
            return;
        };
        let pos = active
            .filters
            .iter()
            .position(|f| f.portal.label == chosen_label);
        active.filter_open = false;
        let Some(pos) = pos else {
            return;
        };
        active.filter_selected = Some(pos);
        let compiled = active.filters.get(pos).cloned();
        active.source.set_filter(compiled);
        pos
    };
    let _ = selected;
    if let Some(last) = app.browser.active_depth() {
        for depth in 0..=last {
            app.browser.retry_column(depth);
        }
    }
    filter::refresh_source_filter(app);
    state::update_selected_filename(app);
}
