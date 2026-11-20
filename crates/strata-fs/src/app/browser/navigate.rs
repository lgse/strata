// SPDX-License-Identifier: MIT

use std::{path::Path, rc::Rc};

use crate::{
    model::Location,
    services::{LocationValidationError, RequestId},
};

use super::columns::Descent;
use super::location_input::{
    is_relative_path_input, join_relative, location_from_input, unsupported_shorthand_message,
};
use super::{Browser, BrowserEvent, LoadSelection, load_selection, select_on_load};

impl Browser {
    #[cfg(test)]
    pub fn navigate_input(self: &Rc<Self>, input: &str) -> Result<(), LocationValidationError> {
        self.navigate_input_located(input, None).map(|_| ())
    }

    /// Navigation for typed input, returning the location that was opened so a
    /// caller can act once it loads (such as pinning it to a sidebar). A
    /// relative path resolves against the native folder `base`.
    pub fn navigate_input_located(
        self: &Rc<Self>,
        input: &str,
        base: Option<&Path>,
    ) -> Result<Location, LocationValidationError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(LocationValidationError::Empty);
        }

        if let Some(message) = unsupported_shorthand_message(input) {
            return Err(LocationValidationError::UnsupportedShorthand(message));
        }
        if let Some(current) = self
            .active_location()
            .filter(|current| current.display_path() == input)
        {
            self.navigate_validated(current.clone(), true);
            return Ok(current);
        }
        let location = match base.filter(|_| is_relative_path_input(input)) {
            Some(base) => Location::local(join_relative(base, input)),
            None => location_from_input(input)?,
        };
        if location.native_path().is_some() && !location.is_absolute_native() {
            return Err(LocationValidationError::NotAbsolute);
        }
        self.navigate_validated(location.clone(), true);
        Ok(location)
    }

    pub(super) fn navigate_validated(self: &Rc<Self>, location: Location, select_first: bool) {
        self.navigate_validated_inner(location, select_first, Vec::new());
    }

    pub(super) fn navigate_validated_inner(
        self: &Rc<Self>,
        location: Location,
        select_first: bool,
        reveal: Vec<Location>,
    ) {
        let _update = self.navigation_update();
        let generation = self.bump_navigation_generation();
        if self.navigation_generation() != generation {
            return;
        }
        self.navigation_lifecycle.pending.set(Some(generation));
        let weak = Rc::downgrade(self);
        let pending = PendingNavigation {
            generation,
            location: location.clone(),
            select_first,
            reveal,
        };
        let emit = Rc::new(move |result| {
            if let Some(browser) = weak.upgrade() {
                browser.finish_location_validation(&pending, result);
            }
        });
        let load = self.source.validate_location_async(location, emit);
        if self.pending_navigation_generation() == Some(generation) {
            self.validation_load.replace(Some(load));
        }
    }

    /// Navigates directly for native paths and validates URI locations first so mountable
    /// locations can be mounted by the UI before loading them.
    fn finish_location_validation(
        self: &Rc<Self>,
        pending: &PendingNavigation,
        result: Result<(), LocationValidationError>,
    ) {
        let _update = self.navigation_update();
        if !self.finish_navigation_validation(pending.generation) {
            return;
        }
        let location = &pending.location;
        match result {
            Ok(()) if pending.reveal.is_empty() => {
                self.navigate_with_selection(location.clone(), pending.select_first);
            }
            Ok(()) => {
                self.reveal_validated(location.clone(), pending.reveal.clone());
            }
            Err(LocationValidationError::NotDirectory) => self.reveal_in_parent(location),
            Err(error) => self.reject_navigation(pending, error),
        }
    }

    /// A file opens its folder with the file revealed.
    fn reveal_in_parent(self: &Rc<Self>, location: &Location) {
        match location.parent() {
            Some(parent) => {
                self.preview_revealed_location
                    .replace(Some(location.clone()));
                self.navigate_validated_inner(parent, false, vec![location.clone()]);
            }
            None => self.emit(BrowserEvent::LocationNavigationRejected {
                location: location.clone(),
                error: LocationValidationError::NotDirectory,
            }),
        }
    }

    /// A reveal into a folder that still needs mounting waits for it.
    fn reject_navigation(&self, pending: &PendingNavigation, error: LocationValidationError) {
        let awaits_mount = matches!(
            error,
            LocationValidationError::NotMounted(_) | LocationValidationError::Mountable(_)
        );
        if !pending.reveal.is_empty() && awaits_mount {
            self.deferred_reveal
                .replace(Some((pending.location.clone(), pending.reveal.clone())));
        } else {
            self.preview_revealed_location.take();
        }
        self.emit(BrowserEvent::LocationNavigationRejected {
            location: pending.location.clone(),
            error,
        });
    }

    pub fn navigate_location(self: &Rc<Self>, location: Location, select_first: bool) {
        if location.native_path().is_some() {
            self.navigate_with_selection(location, select_first);
        } else {
            self.navigate_validated(location, select_first);
        }
    }

    pub fn navigate(self: &Rc<Self>, location: Location) {
        self.navigate_with_selection(location, true);
    }

    pub fn navigate_with_selection(self: &Rc<Self>, location: Location, select_first: bool) {
        self.navigate_for_selection(location, load_selection(select_first));
    }

    /// Navigates to an ancestor of the current directory and selects the folder the
    /// current directory sits in, or the first entry when it is not an ancestor.
    pub fn navigate_to_ancestor(self: &Rc<Self>, location: Location) {
        let selection = self
            .came_from_child(&location)
            .map_or(LoadSelection::FirstEntry, LoadSelection::Prefer);
        self.navigate_for_selection(location, selection);
    }

    /// The entry of `destination` on the way down to the current directory.
    pub(super) fn came_from_child(&self, destination: &Location) -> Option<Location> {
        let path = self.state.borrow().current_path()?;
        let mut location = path.locations().last()?.clone();
        while let Some(parent) = location.parent() {
            if parent == *destination {
                return Some(location);
            }
            location = parent;
        }
        None
    }

    /// Targets override remembered positions; the first listed target takes the cursor.
    /// Returns true only for an in-place selection that needs no subsequent load.
    pub fn reveal_locations(self: &Rc<Self>, directory: Location, targets: Vec<Location>) -> bool {
        if targets.is_empty() {
            self.navigate_location(directory, true);
            return false;
        }
        if directory.native_path().is_some() || self.open_depth(&directory).is_some() {
            self.reveal_validated(directory, targets)
        } else {
            self.navigate_validated_inner(directory, false, targets);
            false
        }
    }

    pub fn open_depth(&self, directory: &Location) -> Option<usize> {
        let state = self.state.borrow();
        state
            .columns
            .iter()
            .position(|column| &column.location == directory)
    }

    pub(super) fn reveal_validated(
        self: &Rc<Self>,
        directory: Location,
        targets: Vec<Location>,
    ) -> bool {
        let _update = self.navigation_update();
        let Some(depth) = self.open_depth(&directory) else {
            self.navigate_for_selection(directory, LoadSelection::Target(targets));
            return false;
        };
        self.activate_reveal_depth(depth);
        if self.state.borrow().lists_every_target(depth, &targets)
            && self.select_entries_by_location_at(depth, &targets)
        {
            return true;
        }
        self.refresh_column_revealing(depth, targets);
        false
    }

    pub fn reveal_reloaded_locations(
        self: &Rc<Self>,
        directory: &Location,
        targets: Vec<Location>,
    ) {
        let _update = self.navigation_update();
        let Some(depth) = self.open_depth(directory) else {
            return;
        };
        self.activate_reveal_depth(depth);
        self.refresh_column_revealing(depth, targets);
    }

    /// The caller must retry missing targets as monitor updates arrive.
    pub fn reveal_monitored_locations(
        self: &Rc<Self>,
        directory: Location,
        targets: &[Location],
    ) -> bool {
        let _update = self.navigation_update();
        let Some(depth) = self.open_depth(&directory) else {
            return false;
        };
        self.activate_reveal_depth(depth);
        self.select_entries_by_location_at(depth, targets)
    }

    pub fn lists_every_target(&self, depth: usize, targets: &[Location]) -> bool {
        self.state.borrow().lists_every_target(depth, targets)
    }

    pub(super) fn activate_reveal_depth(self: &Rc<Self>, depth: usize) {
        let _update = self.navigation_update();
        self.bump_navigation_generation();
        self.close_column(depth + 1);
        if self.active_depth() != Some(depth) {
            self.set_active_column(depth);
        }
    }

    pub fn descend_revealing(
        self: &Rc<Self>,
        parent_depth: usize,
        directory: Location,
        targets: Vec<Location>,
    ) {
        if self.is_open_child(parent_depth, &directory) {
            self.reveal_locations(directory, targets);
        } else {
            let selection = LoadSelection::Target(targets);
            self.descend_with_selection(Descent::new(parent_depth, directory, selection));
        }
    }

    pub fn cancel_deferred_reveal(&self) {
        self.deferred_reveal.take();
        self.preview_revealed_location.take();
    }

    pub(super) fn apply_resolved_location_reveal(
        self: &Rc<Self>,
        depth: usize,
        request_id: RequestId,
    ) {
        let revealed_hidden = self
            .state
            .borrow_mut()
            .take_resolved_location_reveal(depth, request_id)
            == Some(true);
        if revealed_hidden && !self.preferences.get().show_hidden {
            self.toggle_hidden();
        }
    }

    pub(in crate::app) fn report_unresolved_location_reveal(
        &self,
        depth: usize,
        request_id: RequestId,
    ) {
        let location = self
            .state
            .borrow_mut()
            .take_unresolved_location_reveal(depth, request_id);
        if let Some(location) = location {
            self.emit(BrowserEvent::LocationRevealFailed { location });
        }
    }

    pub(super) fn navigate_for_selection(
        self: &Rc<Self>,
        location: Location,
        selection: LoadSelection,
    ) {
        let _update = self.navigation_update();
        if !self.source.allows_navigation(&location) {
            return;
        }
        let selection = match (self.deferred_reveal.take(), selection) {
            (
                Some((directory, targets)),
                LoadSelection::FirstEntry | LoadSelection::Nothing | LoadSelection::Prefer(_),
            ) if directory == location => LoadSelection::Target(targets),
            (_, selection) => selection,
        };
        let reveals_preview_target = self
            .preview_revealed_location
            .borrow()
            .as_ref()
            .is_some_and(|target| {
                matches!(&selection, LoadSelection::Target(targets) if targets.contains(target))
            });
        if !reveals_preview_target {
            self.preview_revealed_location.take();
        }
        self.bump_navigation_generation();
        if self.active_location().as_ref() == Some(&location) {
            return;
        }
        if self.active_location().is_some() {
            self.emit(BrowserEvent::NavigationStarting { history: false });
        }
        self.close_peek();
        self.loads.borrow_mut().clear();
        self.monitors.borrow_mut().clear();
        self.cancel_deferred_work();
        let request_id = self.new_request_id();
        self.state
            .borrow_mut()
            .navigate(location.clone(), request_id);
        select_on_load(&mut self.state.borrow_mut(), 0, selection);
        self.emit(BrowserEvent::Reset);
        self.emit(BrowserEvent::ColumnAdded {
            depth: 0,
            location: location.clone(),
        });
        self.emit(BrowserEvent::FocusChanged {
            depth: 0,
            position: None,
            triggered_by_removal: false,
        });
        self.start_load(0, location, request_id);
    }
}

/// A navigation waiting for its location to validate; `reveal` lists the
/// entries to select once it opens.
struct PendingNavigation {
    generation: u64,
    location: Location,
    select_first: bool,
    reveal: Vec<Location>,
}
