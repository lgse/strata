// SPDX-License-Identifier: MIT

use std::time::Duration;

use gtk::{gio, glib, prelude::*};

use crate::{adapters::location_for_file, model::Location};

use super::{BrowserView, WeakBrowserView, present_target};

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
    let parent = browser.overlay().root().and_downcast::<gtk::Window>();
    let operation = gtk::MountOperation::new(parent.as_ref());

    let connecting = browser.downgrade();
    let cancel_operation = operation.clone();
    glib::timeout_add_local_once(CONNECTING_DELAY, move || {
        show_connecting(connecting, generation, cancel_operation);
    });

    let weak = browser.downgrade();
    let retry_file = file.clone();
    let retry_location = location.clone();
    glib::MainContext::default().spawn_local(async move {
        let outcome = query_kind(&file, Some(&operation)).await;
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
            Err(_) => show_error(browser, retry_file, retry_location),
        }
    });
}

/// A no-follow fallback preserves broken native symlinks as revealable entries.
async fn query_kind(
    file: &gio::File,
    operation: Option<&gtk::MountOperation>,
) -> Result<Kind, glib::Error> {
    let mut mounted = false;
    loop {
        match file
            .query_info_future(
                "standard::type",
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await
        {
            Ok(info) => {
                return Ok(match info.file_type() {
                    gio::FileType::Directory | gio::FileType::Mountable => Kind::Directory,
                    _ => Kind::File,
                });
            }
            Err(error)
                if !mounted
                    && operation.is_some()
                    && error.matches(gio::IOErrorEnum::NotMounted) =>
            {
                mounted = true;
                if let Err(error) = file
                    .mount_enclosing_volume_future(gio::MountMountFlags::NONE, operation)
                    .await
                    && !error.matches(gio::IOErrorEnum::AlreadyMounted)
                {
                    return Err(error);
                }
            }
            Err(error) => {
                return match file
                    .query_info_future(
                        "standard::is-symlink",
                        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                        glib::Priority::DEFAULT,
                    )
                    .await
                {
                    Ok(info) if info.is_symlink() => Ok(Kind::File),
                    _ => Err(error),
                };
            }
        }
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

/// Cancellation disowns late results because a native query may remain blocked in the kernel.
fn show_connecting(weak: WeakBrowserView, generation: u64, operation: gtk::MountOperation) {
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
        operation.reply(gio::MountOperationResult::Aborted);
        cancel_browser.browser().bump_navigation_generation();
        clear_status(&cancel_browser);
        cancel_browser.navigate_location(Location::local(super::home_directory()));
    });

    overlay.add_overlay(&row);
}

fn show_error(browser: BrowserView, file: gio::File, location: Location) {
    let overlay = browser.overlay();
    let (row, content) = status_container();

    let label = gtk::Label::new(Some("Unable to open location"));
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
