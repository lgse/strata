// SPDX-License-Identifier: MIT

//! The footer's item and selection count, as GTK's `update_item_count`
//! (`src/ui/shortcut_footer/status.rs`) reads it from the browser: the active column's
//! entries, its selection (including single-entry name, size and modified date), active
//! filter/search results, and selected entries' missing metadata, in C19's [`summary`] words.
//!
//! - **Read in `poll`, after the shell has handed the browser the view's selection.** A
//!   handler changes the view's selection before the browser hears it, so the count is read
//!   where the two agree, and kept for the build.
//! - **Metadata is asked for.** With up to [`DETAILED_SELECTION`] entries selected, each
//!   selected file without a known size and each selected entry without a known modified time
//!   is filled, as GTK's `request_selected_sizes` / `lacks_metadata` asks, so the count does
//!   not wait for its row to scroll into view. GTK asks again on every browser event, and so
//!   does this: when the selection or the count changes, and after any event the browser
//!   raised. The browser drops a request whose fill is already out, so a fill a reload
//!   dropped, or one the fill's time budget cut off, is asked for again at the next event, as
//!   on GTK.
//! - **An entry a fill answers without metadata is asked once a listing.** An update that
//!   changes another of its fields and leaves size or modified time unknown raises an event
//!   that would ask for it again, without end; such an entry, in the active column, is not
//!   asked again until the column's listing is replaced (its request changes, as a reload
//!   replaces it). An entry that cannot be read at all raises no event, so asking for it again
//!   ends there.
//! - **No active column, no count,** as GTK hides its label.

use std::{collections::HashSet, rc::Rc};

use strata_components::footer::{
    Count, CountWords, DETAILED_SELECTION, FilterCounts, Listing, Selected, SelectionDetails,
    SingleSelection, summary,
};
use strata_core::{
    i18n::{count, file_size, integer, list, tr},
    model::{FileEntry, Location, MetadataValue},
};
use strata_fs::{
    app::{Browser, BrowserEvent},
    services::RequestId,
};

use crate::{App, filter, shell, slots::Listener};

/// What the footer counted at the last poll, and what it knows of the metadata it asks for.
#[derive(Default)]
pub(super) struct Counted {
    listing: Option<Listing>,
    /// A browser event came since the last poll: GTK asks for missing metadata again on each.
    heard: bool,
    /// The listing the files below belong to: the active column and its request.
    under: Option<(usize, Option<RequestId>)>,
    /// The selection's positions at the last poll: a new selection asks for its metadata.
    selected: Vec<usize>,
    /// Entries of the active column a fill answered without metadata, which are not asked for
    /// again in this listing.
    unreadable: HashSet<Location>,
}

/// The browser's events, for the count: each asks for missing metadata again, and a fill that
/// answers an entry without its metadata marks it unreadable.
pub(super) fn listener() -> Listener<App> {
    Box::new(|event, _, heard| {
        let answered = match event {
            BrowserEvent::MetadataFilled { depth, updates } => Some((
                *depth,
                updates
                    .iter()
                    .filter(|(_, entry)| lacks_metadata(entry))
                    .map(|(_, entry)| entry.location.clone())
                    .collect::<Vec<_>>(),
            )),
            _ => None,
        };
        heard.then(move |app: &mut App, _| {
            let active = app.browser.active_depth();
            let counted = &mut app.footer.counted;
            counted.heard = true;
            if let Some((_, unreadable)) = answered.filter(|(depth, _)| Some(*depth) == active) {
                counted.unreadable.extend(unreadable);
            }
            false
        });
    })
}

/// Reads the count from the browser and asks for the selected entries' missing metadata. True
/// when the count changed, so the footer is built again.
pub(super) fn poll(app: &mut App) -> bool {
    let filter_counts = filter::footer_counts(app);
    let browser = Rc::clone(&app.browser);
    let Some(depth) = browser.active_depth() else {
        let counted = &mut app.footer.counted;
        let shown = counted.listing.is_some();
        *counted = Counted::default();
        return shown;
    };
    let under = Some((depth, browser.column_request_id(depth)));
    let (selected, replaced, reselected, heard) = {
        let counted = &mut app.footer.counted;
        let replaced = counted.under != under;
        if replaced {
            counted.under = under;
            counted.unreadable.clear();
        }
        // Past the detailed limit only the number is read, as GTK reads no entry then.
        let selected = if browser.selected_count() <= DETAILED_SELECTION {
            browser.selected_positions(depth)
        } else {
            Vec::new()
        };
        let reselected = counted.selected != selected;
        let heard = std::mem::take(&mut counted.heard);
        counted.selected = selected.clone();
        (selected, replaced, reselected, heard)
    };
    if !filter_counts.is_some_and(|f| f.displayed) && (heard || replaced || reselected) {
        ask_metadata(&browser, depth, &selected, &app.footer.counted.unreadable);
    }
    let listing = Some(read(app, depth, &selected, filter_counts));
    let counted = &mut app.footer.counted;
    if counted.listing == listing {
        return false;
    }
    counted.listing = listing;
    true
}

/// The count the footer shows, in the language now.
pub(super) fn shown(app: &App) -> Option<Count> {
    let listing = app.footer.counted.listing.as_ref()?;
    Some(summary(listing, &words()))
}

fn read(app: &App, depth: usize, selected: &[usize], filter: Option<FilterCounts>) -> Listing {
    let browser = &app.browser;
    let counts = browser.column_entry_counts(depth).unwrap_or_default();
    let count = browser.selected_count();
    let show_modified = shows_modified_date(app);
    let selected = (count > 0).then(|| {
        let detailed = count <= DETAILED_SELECTION;
        Selected {
            count,
            details: detailed.then(|| details(browser, depth, selected)),
            single: (detailed && count == 1)
                .then(|| single_entry(browser, depth, selected, show_modified))
                .flatten(),
        }
    });
    Listing {
        total: counts.total,
        files: counts.files,
        folders: counts.folders,
        selected,
        filter,
    }
}

/// Columns and Icons show the selected entry's modified date in the footer; List omits it
/// because its table already displays a Modified column (`single_entry_label` in GTK).
fn shows_modified_date(app: &App) -> bool {
    shell::shown_view(app).is_some_and(|view| (view.shows)("icons") || (view.shows)("columns"))
}

/// GTK's `single_entry_label` (`src/ui/shortcut_footer/status.rs`): display name, known file
/// size, and formatted modified date (in Columns and Icons views).
fn single_entry(
    browser: &Browser,
    depth: usize,
    selected: &[usize],
    show_modified: bool,
) -> Option<SingleSelection> {
    let [position] = selected else {
        return None;
    };
    let entry = browser.entry_at(depth, *position)?;
    let is_directory = entry.is_directory();
    let size = match entry.size {
        MetadataValue::Known(bytes) => Some(bytes),
        _ => None,
    };
    let modified = show_modified.then(|| strata_core::util::modified_date(&entry));
    Some(SingleSelection {
        name: entry.display_name,
        is_directory,
        size,
        modified,
    })
}

/// The selection's kinds and the selected files' known sizes.
fn details(browser: &Browser, depth: usize, selected: &[usize]) -> SelectionDetails {
    let mut details = SelectionDetails::default();
    for entry in selected
        .iter()
        .filter_map(|&at| browser.entry_at(depth, at))
    {
        if entry.is_directory() {
            details.folders += 1;
        } else {
            details.files += 1;
            if let MetadataValue::Known(size) = entry.size {
                details.known_files += 1;
                details.known_bytes = details.known_bytes.saturating_add(size);
            }
        }
    }
    details
}

/// GTK's `request_selected_sizes`: a fill for each selected entry lacking size or modified
/// timestamp, up to [`DETAILED_SELECTION`] selected, skipping entries a fill found unreadable.
/// The browser drops a request whose fill is already out.
fn ask_metadata(
    browser: &Rc<Browser>,
    depth: usize,
    selected: &[usize],
    unreadable: &HashSet<Location>,
) {
    for &position in selected {
        let Some(entry) = browser.entry_at(depth, position) else {
            continue;
        };
        if lacks_metadata(&entry) && !unreadable.contains(&entry.location) {
            browser.request_metadata_fill(depth, position, entry.location, false);
        }
    }
}

/// GTK's `lacks_metadata` (`src/ui/shortcut_footer/status.rs`).
fn lacks_metadata(entry: &FileEntry) -> bool {
    (!entry.is_directory() && entry.size == MetadataValue::Unknown)
        || entry.modified_unix_seconds == MetadataValue::Unknown
}

/// The count's words, from the catalogs, as GTK's `status.rs` writes them.
fn words() -> CountWords {
    CountWords {
        items: Rc::new(|n| count("items", n)),
        files: Rc::new(|n| count("files", n)),
        folders: Rc::new(|n| count("folders", n)),
        list: Rc::new(|parts| list(parts.iter().cloned())),
        selected: Rc::new(|items| rust_i18n::t!("%{items} selected", items = items).into_owned()),
        breakdown: Rc::new(|files, folders| {
            rust_i18n::t!("%{files}, %{folders}", files = files, folders = folders).into_owned()
        }),
        // The plural form follows the total; the selected count fills in afterwards.
        selected_of: Rc::new(|selected, total| {
            rust_i18n::replace_patterns(
                &count("selected_of_items", total),
                &["selected"],
                &[integer(selected as u64)],
            )
        }),
        selection_description: Rc::new(|selection| {
            rust_i18n::t!(
                "%{selection} Size includes selected files only; folder contents are not counted.",
                selection = selection
            )
            .into_owned()
        }),
        size: Rc::new(file_size),
        size_known: Rc::new(|size| format!(" ({size})")),
        size_incomplete: Rc::new(|size| {
            rust_i18n::t!(" (%{size} known; size incomplete)", size = size).into_owned()
        }),
        size_unavailable: tr(" (size unavailable)"),
    }
}
