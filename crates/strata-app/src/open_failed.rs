// SPDX-License-Identifier: MIT

use strata_core::{i18n::tr, model::Location};
use strata_fs::{
    app::{Browser, BrowserEvent},
    services::LocationValidationError,
};
use strata_kit::icons::IconName;

use crate::{
    App,
    error_dialog::{self, Error, Offer},
    shell,
    slots::{Frame, Listener, Part, Place, Setup},
};

/// The reports' texts, from the catalogs once.
pub(crate) struct OpenFailed {
    from_listing: String,
    by_path: String,
    administrator: String,
}

#[derive(Clone, Copy)]
enum Opened {
    FromListing,
    ByPath,
}

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

pub(crate) fn typed_refused(app: &mut App, error: &LocationValidationError) {
    let title = app.open_failed.by_path.clone();
    let view = shell::view_focus(app);
    error_dialog::show(app, Error::new(title, error.message()).returning_to(view));
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
            let reported = reported(browser, location, error);
            let error = error.clone();
            heard.then(move |app: &mut App, _| {
                if matches!(opened, Opened::ByPath) {
                    app.browser.cancel_deferred_reveal();
                    if crate::connect_server::fail_pending(app, &error) {
                        return true;
                    }
                }
                if let Some(admin) = reported {
                    report(app, opened, &error, admin);
                }
                true
            });
        }))
    }
}

#[cfg(test)]
mod tests;
