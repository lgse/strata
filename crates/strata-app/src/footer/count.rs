// SPDX-License-Identifier: MIT

//! The footer's item and selection count, as GTK's `update_item_count`
//! (`src/ui/shortcut_footer/status.rs`) reads it from the browser: the active column's
//! entries, its selection, and the selected files' sizes, in C19's [`summary`] words.
//!
//! - **Read in `poll`, after the shell has handed the browser the view's selection.** A
//!   handler changes the view's selection before the browser hears it, so the count is read
//!   where the two agree, and kept for the build.
//! - **Sizes are asked for.** With up to [`DETAILED_SELECTION`] entries selected, a selected
//!   file whose size the browser does not know yet is filled, as GTK's
//!   `request_selected_sizes` asks, so the count does not wait for its row to scroll into
//!   view. Each is asked once while it stays selected: a fill that cannot read a size leaves
//!   it unknown, and asking again would loop.
//! - **No active column, no count,** as GTK hides its label.
//!
//! Not yet: a filter's or a search's own count ([`Listing::filter`]), which needs the
//! filter's status ([A10]).
//!
//! [A10]: ../../../../docs/rune/parity/packages/A10.md

use std::{collections::HashSet, rc::Rc};

use strata_components::footer::{
    Count, CountWords, DETAILED_SELECTION, Listing, Selected, SelectionDetails, summary,
};
use strata_core::{
    i18n::{count, file_size, integer, list, tr},
    model::{FileEntry, Location, MetadataValue},
};
use strata_fs::app::Browser;

use crate::App;

/// What the footer counted at the last poll, and the sizes it has asked for.
#[derive(Default)]
pub(super) struct Counted {
    listing: Option<Listing>,
    asked: HashSet<Location>,
}

/// Reads the count from the browser and asks for the selected files' missing sizes. True
/// when the count changed, so the footer is built again.
pub(super) fn poll(app: &mut App) -> bool {
    let browser = Rc::clone(&app.browser);
    let counted = &mut app.footer.counted;
    let listing = read(&browser, &mut counted.asked);
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

fn read(browser: &Rc<Browser>, asked: &mut HashSet<Location>) -> Option<Listing> {
    let Some(depth) = browser.active_depth() else {
        asked.clear();
        return None;
    };
    let counts = browser.column_entry_counts(depth).unwrap_or_default();
    let count = browser.selected_count();
    let selected = (count > 0).then(|| Selected {
        count,
        details: (count <= DETAILED_SELECTION).then(|| details(browser, depth, asked)),
    });
    if count == 0 || count > DETAILED_SELECTION {
        asked.clear();
    }
    Some(Listing {
        total: counts.total,
        files: counts.files,
        folders: counts.folders,
        selected,
        filter: None,
    })
}

/// The selection's kinds and known sizes, asking for each selected file's size the browser
/// lacks and has not been asked for.
fn details(browser: &Rc<Browser>, depth: usize, asked: &mut HashSet<Location>) -> SelectionDetails {
    let mut details = SelectionDetails::default();
    let mut lacking = HashSet::new();
    for position in browser.selected_positions(depth) {
        let Some(entry) = browser.entry_at(depth, position) else {
            continue;
        };
        if entry.is_directory() {
            details.folders += 1;
            continue;
        }
        details.files += 1;
        if let MetadataValue::Known(size) = entry.size {
            details.known_files += 1;
            details.known_bytes = details.known_bytes.saturating_add(size);
        } else if lacks_size(&entry) {
            if !asked.contains(&entry.location) {
                browser.request_metadata_fill(depth, position, entry.location.clone(), false);
            }
            lacking.insert(entry.location);
        }
    }
    asked.retain(|location| lacking.contains(location));
    asked.extend(lacking);
    details
}

fn lacks_size(entry: &FileEntry) -> bool {
    entry.size == MetadataValue::Unknown
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
