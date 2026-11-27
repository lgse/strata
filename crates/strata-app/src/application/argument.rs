// SPDX-License-Identifier: MIT

use std::rc::{Rc, Weak};

use rune::Tx;
use strata_core::model::Location;
use strata_fs::{
    adapters::{gio_file_for_location, reveal_target_for_file},
    app::Browser,
};

use crate::{App, shell};

/// What an argument is, as far as opening it goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// It opens: a folder, or a location that mounts as one.
    Folder,
    /// It shows selected in its folder: a file, or a symbolic link, dangling or not, to one.
    Item,
}

pub(crate) fn open(app: &mut App, location: Location) {
    let browser = Rc::downgrade(&app.browser);
    let generation = app.browser.bump_navigation_generation();
    let later = shell::window_later(app);
    let file = gio_file_for_location(&location);
    glib::MainContext::default().spawn_local(async move {
        let kind = kind(&file).await;
        later.then(move |_: &mut App, _: &mut Tx| {
            shown(&browser, generation, (&file, location), kind);
            false
        });
    });
}

fn shown(
    browser: &Weak<Browser>,
    generation: u64,
    (file, location): (&gio::File, Location),
    kind: Option<Kind>,
) {
    let Some(browser) = browser.upgrade() else {
        return;
    };
    if browser.navigation_generation() != generation {
        return;
    }
    match (kind, reveal_target_for_file(file)) {
        (Some(Kind::Item), Some((parent, item))) => {
            browser.reveal_locations(parent, vec![item]);
        }
        _ => browser.navigate_location(location, true),
    }
}

pub(super) async fn kind(file: &gio::File) -> Option<Kind> {
    use gio::prelude::*;
    let followed = file
        .query_info_future(
            "standard::type",
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await;
    match followed {
        Ok(info) => Some(match info.file_type() {
            gio::FileType::Directory | gio::FileType::Mountable => Kind::Folder,
            _ => Kind::Item,
        }),
        Err(_) => file
            .query_info_future(
                "standard::is-symlink",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
            )
            .await
            .ok()
            .filter(|info| info.is_symlink())
            .map(|_| Kind::Item),
    }
}
