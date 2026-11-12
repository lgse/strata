// SPDX-License-Identifier: MIT

//! Pinning, unpinning and moving the user's pinned folders.

use std::rc::Rc;

use crate::{
    model::Location,
    ui::browser::{PinStatus, show_error_dialog},
};

use super::{SidebarState, places::is_standard_place_location};

impl SidebarState {
    pub(super) fn pin_location(self: &Rc<Self>, location: Location, name: String) {
        self.pin_location_with(location, name, |_| {});
    }

    pub(super) fn pin_location_with(
        self: &Rc<Self>,
        location: Location,
        name: String,
        done: impl FnOnce(bool) + 'static,
    ) {
        let expected = location.clone();
        let weak = Rc::downgrade(self);
        self.update_pinned_places(
            move |places| {
                if pin_status(places, &location) != PinStatus::Available {
                    return false;
                }
                places.push((location.clone(), name.clone()));
                true
            },
            move |_| {
                done(weak.upgrade().is_some_and(|state| {
                    pin_status(&state.pinned_places.borrow(), &expected) == PinStatus::Pinned
                }))
            },
        );
    }

    pub(super) fn unpin_location(self: &Rc<Self>, location: &Location) {
        let location = location.clone();
        self.update_pinned_places(move |places| remove_pinned_place(places, &location), |_| {});
    }

    pub(super) fn pin_folders_into(
        self: &Rc<Self>,
        folders: &[(Location, String)],
        (after, before): PinGap<'_>,
        done: impl FnOnce(bool) + 'static,
    ) {
        let folders = folders.to_vec();
        let (after, before) = (after.cloned(), before.cloned());
        self.update_pinned_places(
            move |places| {
                pin_gap_index(places, (after.as_ref(), before.as_ref()))
                    .is_some_and(|index| pin_locations_at(places, &folders, index))
            },
            done,
        );
    }

    pub(super) fn reorder_pinned_into(
        self: &Rc<Self>,
        source: &Location,
        (after, before): PinGap<'_>,
        done: impl FnOnce(bool) + 'static,
    ) {
        let source = source.clone();
        let (after, before) = (after.cloned(), before.cloned());
        self.update_pinned_places(
            move |places| {
                let Some(source) = places.iter().position(|(location, _)| *location == source)
                else {
                    return false;
                };
                pin_gap_index(places, (after.as_ref(), before.as_ref()))
                    .is_some_and(|index| move_pinned_place(places, source, index))
            },
            done,
        );
    }

    pub(super) fn update_pinned_places(
        self: &Rc<Self>,
        change: impl Fn(&mut Vec<(Location, String)>) -> bool + Send + 'static,
        done: impl FnOnce(bool) + 'static,
    ) {
        // Merge into the shared file, never this window's stale snapshot.
        let edit = move |contents: &[u8]| {
            let mut places = crate::adapters::bookmarks::parse_pinned_places(contents);
            change(&mut places)
                .then(|| crate::adapters::bookmarks::serialize_pinned_places(&places).into_bytes())
        };
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            match crate::adapters::bookmarks::update(edit).await {
                Ok(changed) => done(changed),
                Err(error) => {
                    if let Some(state) = weak.upgrade() {
                        show_error_dialog(
                            &state.view().widget(),
                            &crate::i18n::tr("Unable to update pinned folders"),
                            &crate::services::io_error_message(&error),
                        );
                    }
                    done(false);
                }
            }
        });
    }
}

pub(super) fn pin_status(places: &[(Location, String)], location: &Location) -> PinStatus {
    if is_standard_place_location(location) {
        PinStatus::Unavailable
    } else if places.iter().any(|(pinned, _)| pinned == location) {
        PinStatus::Pinned
    } else {
        PinStatus::Available
    }
}

pub(super) fn remove_pinned_place(
    places: &mut Vec<(Location, String)>,
    location: &Location,
) -> bool {
    let original_len = places.len();
    places.retain(|(pinned, _)| pinned != location);
    places.len() != original_len
}

/// The `(after, before)` pins around an insertion gap when the sidebar was built.
pub(super) type PinGap<'a> = (Option<&'a Location>, Option<&'a Location>);

/// Resolves a gap against the latest bookmarks; `None` when both of a gap's
/// neighbours were unpinned elsewhere, so a drop cannot land somewhere unrelated.
pub(super) fn pin_gap_index(
    places: &[(Location, String)],
    (after, before): PinGap<'_>,
) -> Option<usize> {
    let position = |anchor: &Location| places.iter().position(|(location, _)| location == anchor);
    match (before.and_then(position), after.and_then(position)) {
        (Some(index), _) => Some(index),
        (None, Some(index)) => Some(index + 1),
        (None, None) if before.is_none() => Some(places.len()),
        (None, None) => None,
    }
}

/// `index` is an insertion point in the list before `source` is removed.
pub(super) fn move_pinned_place(
    places: &mut Vec<(Location, String)>,
    source: usize,
    index: usize,
) -> bool {
    let out_of_range = source >= places.len() || index > places.len();
    let stays_put = index == source || index == source + 1;
    if out_of_range || stays_put {
        return false;
    }
    let place = places.remove(source);
    places.insert(index - usize::from(index > source), place);
    true
}

pub(super) fn pin_locations_at(
    places: &mut Vec<(Location, String)>,
    sources: &[(Location, String)],
    target_index: usize,
) -> bool {
    let mut seen = places
        .iter()
        .map(|(location, _)| location.clone())
        .collect::<std::collections::HashSet<_>>();
    let additions = sources
        .iter()
        .filter(|(location, _)| {
            !is_standard_place_location(location) && seen.insert(location.clone())
        })
        .cloned()
        .collect::<Vec<_>>();
    if additions.is_empty() {
        return false;
    }
    let index = target_index.min(places.len());
    places.splice(index..index, additions);
    true
}
