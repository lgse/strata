// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};

use crate::ui::browser::FilterStatus;

/// Selections up to this size get their missing file sizes requested and listed.
const DETAILED_SELECTION: usize = 64;

/// The active listing's entry counts.
struct ListingCounts {
    total: usize,
    files: usize,
    folders: usize,
}

pub(super) fn update_item_count(
    label: &gtk::Label,
    browser: &Rc<crate::app::Browser>,
    filter: Option<&FilterStatus>,
) {
    if let Some(filter) = filter.filter(|filter| filter.displayed) {
        label.set_label(&crate::i18n::count("items", filter.total()));
        crate::ui::accessibility::set_description(
            label,
            Some(&file_folder_breakdown(filter.files, filter.folders)),
        );
        label.set_visible(true);
        return;
    }
    let Some(depth) = browser.active_depth() else {
        label.set_visible(false);
        return;
    };
    let counts = browser.column_entry_counts(depth).unwrap_or_default();
    let counts = ListingCounts {
        total: counts.total,
        files: counts.files,
        folders: counts.folders,
    };
    let selected_len = browser.selected_count();
    if selected_len > 0 && selected_len <= DETAILED_SELECTION {
        request_selected_sizes(browser, depth);
    }
    if selected_len > 0 {
        show_selection_count(label, browser, &counts, selected_len);
    } else {
        label.set_label(&crate::i18n::count("items", counts.total));
        crate::ui::accessibility::set_description(
            label,
            Some(&file_folder_breakdown(counts.files, counts.folders)),
        );
    }
    label.set_visible(true);
}

fn lacks_size(entry: &crate::model::FileEntry) -> bool {
    !entry.is_directory() && entry.size == crate::model::MetadataValue::Unknown
}

fn request_selected_sizes(browser: &Rc<crate::app::Browser>, depth: usize) {
    for position in browser.selected_positions(depth) {
        if let Some(entry) = browser.entry_at(depth, position).filter(lacks_size) {
            browser.request_metadata_fill(depth, position, entry.location, false);
        }
    }
}

fn show_selection_count(
    label: &gtk::Label,
    browser: &crate::app::Browser,
    counts: &ListingCounts,
    selected_len: usize,
) {
    label.set_label(&selection_label(browser, counts, selected_len));
    // The plural form follows the total; the selected count fills in afterwards.
    let selection = rust_i18n::replace_patterns(
        &crate::i18n::count("selected_of_items", counts.total),
        &["selected"],
        &[crate::i18n::integer(selected_len as u64)],
    );
    crate::ui::accessibility::set_description(
        label,
        Some(&rust_i18n::t!(
            "%{selection} Size includes selected files only; folder contents are not counted.",
            selection = selection
        )),
    );
}

fn selection_label(
    browser: &crate::app::Browser,
    counts: &ListingCounts,
    selected_len: usize,
) -> String {
    if counts.total > 0 && selected_len == counts.total {
        return whole_listing_label(counts, selected_len);
    }
    if selected_len > DETAILED_SELECTION {
        return rust_i18n::t!(
            "%{items} selected",
            items = crate::i18n::count("items", selected_len)
        )
        .into_owned();
    }
    selection_details(&browser.selected_entries())
}

fn whole_listing_label(counts: &ListingCounts, selected_len: usize) -> String {
    let mut parts = Vec::new();
    if counts.folders > 0 {
        parts.push(crate::i18n::count("folders", counts.folders));
    }
    if counts.files > 0 {
        parts.push(crate::i18n::count("files", counts.files));
    }
    let items = if parts.is_empty() {
        crate::i18n::count("items", selected_len)
    } else {
        crate::i18n::list(parts)
    };
    rust_i18n::t!("%{items} selected", items = items).into_owned()
}

fn file_folder_breakdown(files: usize, folders: usize) -> String {
    rust_i18n::t!(
        "%{files}, %{folders}",
        files = crate::i18n::count("files", files),
        folders = crate::i18n::count("folders", folders)
    )
    .into_owned()
}

pub(super) fn selection_details(entries: &[crate::model::FileEntry]) -> String {
    let folders = entries.iter().filter(|entry| entry.is_directory()).count();
    let files = entries.len() - folders;
    let mut parts = Vec::new();
    if folders > 0 {
        parts.push(crate::i18n::count("folders", folders));
    }
    if files > 0 {
        parts.push(crate::i18n::count("files", files));
    }
    let mut text =
        rust_i18n::t!("%{items} selected", items = crate::i18n::list(parts)).into_owned();
    if files > 0 {
        text.push_str(&size_suffix(entries, files));
    }
    text
}

fn size_suffix(entries: &[crate::model::FileEntry], files: usize) -> String {
    let mut bytes = 0u64;
    let mut known = 0;
    for entry in entries.iter().filter(|entry| !entry.is_directory()) {
        if let crate::model::MetadataValue::Known(size) = entry.size {
            bytes = bytes.saturating_add(size);
            known += 1;
        }
    }
    let size = crate::ui::browser::format_file_size(bytes);
    if known == files {
        format!(" ({size})")
    } else if known > 0 {
        rust_i18n::t!(" (%{size} known; size incomplete)", size = size).into_owned()
    } else {
        crate::i18n::tr(" (size unavailable)")
    }
}

pub(super) fn watch_status_widget(
    widget: &gtk::Widget,
    status_widgets: &Rc<RefCell<Vec<gtk::Widget>>>,
    root: &gtk::Stack,
) {
    let root = root.downgrade();
    let statuses = status_widgets.clone();
    widget.connect_visible_notify(move |_| {
        // Ignore ancestor visibility so a hidden footer can reveal itself.
        if let Some(root) = root.upgrade() {
            root.set_visible(
                statuses
                    .borrow()
                    .iter()
                    .any(gtk::prelude::WidgetExt::get_visible),
            );
        }
    });
}

pub(super) fn refresh_paste_availability(
    clipboard: &gdk::Clipboard,
    label: &glib::WeakRef<gtk::Label>,
    generation: &Rc<Cell<u64>>,
) {
    let revision = generation.get().wrapping_add(1);
    generation.set(revision);
    let Some(paste) = label.upgrade() else {
        return;
    };
    paste.set_visible(false);
    let formats = clipboard.formats();
    if !formats.contains_type(gdk::FileList::static_type())
        && !formats.contain_mime_type("text/uri-list")
    {
        return;
    }
    let clipboard = clipboard.clone();
    let label = label.clone();
    let generation = generation.clone();
    glib::MainContext::default().spawn_local(async move {
        let available = clipboard
            .read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT)
            .await
            .ok()
            .and_then(|value| value.get::<gdk::FileList>().ok())
            .is_some_and(|files| !files.files().is_empty());
        if revision == generation.get()
            && let Some(label) = label.upgrade()
        {
            label.set_visible(available);
        }
    });
}
