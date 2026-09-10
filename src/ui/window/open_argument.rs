// SPDX-License-Identifier: MIT

//! Opens a window for a launch argument whose file-vs-directory kind isn't known yet
//! (`main.rs`'s `connect_open`; `org.freedesktop.FileManager1`'s `ShowItems`/`ShowFolders`
//! already know the answer and go through `present_reveal` instead). The window presents
//! immediately; classification runs off the GTK main thread, redirecting to reveal the
//! target in its parent when it turns out to be a regular file.

use std::time::Duration;

use gtk::{gio, glib, prelude::*};

use crate::{adapters::location_for_file, model::Location};

use super::{BrowserView, WeakBrowserView, present_target};

/// How long a pending classification is left to resolve quietly before a spinner and
/// "Connecting to location…" appear, per the recommended feedback in lgse/strata#726.
const CONNECTING_DELAY: Duration = Duration::from_secs(1);

pub fn present_open(application: &gtk::Application, file: gio::File) {
    let Some(location) = location_for_file(&file) else {
        return;
    };
    let browser = present_target(
        application,
        Some(location.clone()),
        Vec::new(),
        false,
        false,
    );
    classify(browser, file, location);
}

enum Kind {
    Directory,
    File,
}

fn classify(browser: BrowserView, file: gio::File, location: Location) {
    let generation = browser.browser().bump_navigation_generation();

    let connecting = browser.downgrade();
    let cancel_fallback = location.clone();
    glib::timeout_add_local_once(CONNECTING_DELAY, move || {
        show_connecting(connecting, generation, cancel_fallback);
    });

    let weak = browser.downgrade();
    let retry_file = file.clone();
    let retry_location = location.clone();
    glib::MainContext::default().spawn_local(async move {
        let outcome = query_kind(&file).await;
        let Some(browser) = weak.upgrade() else {
            return;
        };
        if browser.browser().navigation_generation() != generation {
            return;
        }
        clear_status(&browser);
        match outcome {
            Ok(Kind::Directory) => browser.navigate_location(location),
            Ok(Kind::File) => reveal_in_parent(&browser, &file, location),
            Err(message) => show_error(browser, retry_file, retry_location, message),
        }
    });
}

/// A broken symlink has no type GIO can resolve for the link's target; a no-follow query
/// for the link itself still counts as revealable, matching the pre-async behavior.
async fn query_kind(file: &gio::File) -> Result<Kind, String> {
    match file
        .query_info_future(
            "standard::type",
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await
    {
        Ok(info) => Ok(match info.file_type() {
            gio::FileType::Directory | gio::FileType::Mountable => Kind::Directory,
            _ => Kind::File,
        }),
        Err(error) => match file
            .query_info_future(
                "standard::is-symlink",
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                glib::Priority::DEFAULT,
            )
            .await
        {
            Ok(info) if info.is_symlink() => Ok(Kind::File),
            _ => Err(error.to_string()),
        },
    }
}

fn reveal_in_parent(browser: &BrowserView, file: &gio::File, location: Location) {
    match file.parent().and_then(|parent| location_for_file(&parent)) {
        Some(parent) => {
            let name = file
                .basename()
                .map(|name| name.to_string_lossy().into_owned());
            browser.select_after_load(name.into_iter().collect(), false);
            browser.navigate_location(parent);
        }
        None => browser.navigate_location(location),
    }
}

fn status_widget(overlay: &gtk::Overlay) -> Option<gtk::Widget> {
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        if widget.has_css_class("open-argument-status") {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}

fn clear_status(browser: &BrowserView) {
    let overlay = browser.overlay();
    if let Some(widget) = status_widget(&overlay) {
        overlay.remove_overlay(&widget);
    }
}

fn status_container() -> (gtk::Box, gtk::Box) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.add_css_class("open-argument-status");
    row.set_halign(gtk::Align::Center);
    row.set_valign(gtk::Align::Start);
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&content);
    (row, content)
}

/// Cancelling doesn't guarantee the in-flight GIO query actually stops (a blocked native
/// stat on a dead NFS mount may not respond to cancellation at all); it only stops the UI
/// from waiting and disowns the result, matching lgse/strata#726's cancel semantics.
fn show_connecting(weak: WeakBrowserView, generation: u64, fallback: Location) {
    let Some(browser) = weak.upgrade() else {
        return;
    };
    if browser.browser().navigation_generation() != generation {
        return;
    }
    clear_status(&browser);
    let overlay = browser.overlay();
    let (row, content) = status_container();

    let spinner = gtk::Spinner::new();
    spinner.start();
    content.append(&spinner);

    let label = gtk::Label::new(Some("Connecting to location…"));
    label.add_css_class("form-message");
    content.append(&label);

    let cancel = gtk::Button::with_label("Cancel");
    content.append(&cancel);
    let cancel_browser = browser.clone();
    cancel.connect_clicked(move |_| {
        cancel_browser.browser().bump_navigation_generation();
        clear_status(&cancel_browser);
        cancel_browser.navigate_location(fallback.clone());
    });

    overlay.add_overlay(&row);
}

fn show_error(browser: BrowserView, file: gio::File, location: Location, message: String) {
    let overlay = browser.overlay();
    let (row, content) = status_container();

    let label = gtk::Label::new(Some(&message));
    label.add_css_class("form-message");
    label.add_css_class("error");
    label.set_wrap(true);
    content.append(&label);

    let retry = gtk::Button::with_label("Retry");
    retry.add_css_class("suggested-action");
    content.append(&retry);
    retry.connect_clicked(move |_| {
        clear_status(&browser);
        classify(browser.clone(), file.clone(), location.clone());
    });

    overlay.add_overlay(&row);
}

#[cfg(test)]
mod tests;
