// SPDX-License-Identifier: MIT

//! Why a folder or a location could not be opened, as GTK reports it on the window's error
//! dialog ([`crate::error_dialog`]):
//!
//! - **A folder opened from a listing** that the browser refuses (`NavigationRejected`):
//!   "Unable to open directory" (`handle_navigation_rejected` in `src/ui/browser/location.rs`).
//! - **A location opened by its path**, typed into the location bar or asked for by a part,
//!   that the browser refuses once it has looked (`LocationNavigationRejected`): "Unable to
//!   open location" (`on_location_navigation_rejected` in `src/ui/browser/events.rs`). One the
//!   location bar's text is refused for at once comes here too ([`typed_refused`]).
//!
//! The message is the browser's reason. When only permissions keep the user out and the
//! browser offers the folder's `admin://` form, the dialog offers Open as Administrator, which
//! opens it in its place (`show_open_failure`). Not reported, as on GTK: a refusal the user
//! answered already (`NotAuthorized`, a dismissed password prompt). A location that has to be
//! mounted first is GTK's to mount; this interface does not mount yet, so it is reported as
//! the refusal it is.

use strata_core::{i18n::tr, model::Location};
use strata_fs::{
    app::{Browser, BrowserEvent},
    services::LocationValidationError,
};
use strata_kit::icons::IconName;

use crate::{
    App,
    error_dialog::{self, Error, Offer},
    slots::{Frame, Listener, Part, Place, Setup},
};

/// The reports' texts, from the catalogs once.
pub(crate) struct OpenFailed {
    from_listing: String,
    by_path: String,
    administrator: String,
}

/// Which of GTK's two reports a refusal gets.
#[derive(Clone, Copy)]
enum Opened {
    FromListing,
    ByPath,
}

/// A refusal of `location` the user is told of, with the `admin://` form to offer; `None`
/// for one GTK does not report.
fn reported(
    browser: &Browser,
    location: &Location,
    error: &LocationValidationError,
) -> Option<Option<Location>> {
    match error {
        LocationValidationError::NotAuthorized => None,
        LocationValidationError::Inaccessible => Some(browser.administrator_alternative(location)),
        _ => Some(None),
    }
}

fn report(app: &mut App, opened: Opened, error: &LocationValidationError, admin: Option<Location>) {
    let texts = &app.open_failed;
    let title = match opened {
        Opened::FromListing => &texts.from_listing,
        Opened::ByPath => &texts.by_path,
    };
    let failure = Error::new(title.clone(), error.message());
    let failure = match admin {
        Some(administrator) => failure.offering(Offer::new(
            texts.administrator.clone(),
            IconName::Lock,
            move |app: &mut App| app.browser.navigate_location(administrator.clone(), true),
        )),
        None => failure,
    };
    error_dialog::show(app, failure);
}

/// A typed location refused before it was opened (empty, not absolute, a bad URI): "Unable
/// to open location", with no offer, as GTK's `submit_location` reports it.
pub(crate) fn typed_refused(app: &mut App, error: &LocationValidationError) {
    report(app, Opened::ByPath, error, None);
}

/// Nothing is drawn: the report is the error dialog's.
fn nothing(_: &App, _: &mut Frame<'_, App>) -> Option<rune::El> {
    None
}

impl Part for OpenFailed {
    const PLACE: Place<App> = Place::Overlays(nothing);

    fn new(_: &Setup<'_>) -> Self {
        Self {
            from_listing: tr("Unable to open directory"),
            by_path: tr("Unable to open location"),
            administrator: tr("Open as Administrator"),
        }
    }

    fn listen(_: &Setup<'_>) -> Option<Listener<App>> {
        Some(Box::new(|event, browser, heard| {
            let (opened, location, error) = match event {
                BrowserEvent::NavigationRejected {
                    location, error, ..
                } => (Opened::FromListing, location, error),
                BrowserEvent::LocationNavigationRejected { location, error } => {
                    (Opened::ByPath, location, error)
                }
                _ => return,
            };
            let Some(admin) = reported(browser, location, error) else {
                return;
            };
            let error = error.clone();
            heard.then(move |app: &mut App, _| {
                report(app, opened, &error, admin);
                true
            });
        }))
    }
}

#[cfg(test)]
mod tests;
