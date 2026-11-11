//! Unit tests for the footer's count (`app/shortcut-reference`, Footer status).

use super::count::*;

fn listing() -> Listing {
    Listing {
        total: 5,
        files: 3,
        folders: 2,
        selected: None,
        filter: None,
    }
}

fn selected(count: usize, d: Option<SelectionDetails>) -> Option<Selected> {
    Some(Selected { count, details: d })
}

fn details(files: usize, folders: usize, known_files: usize, known_bytes: u64) -> SelectionDetails {
    SelectionDetails {
        files,
        folders,
        known_files,
        known_bytes,
    }
}

fn count(l: &Listing) -> Count {
    summary(l, &CountWords::default())
}

#[test]
fn with_nothing_selected_the_count_reads_n_items_described_by_the_breakdown() {
    let c = count(&listing());
    assert_eq!(c.text, "5 items");
    assert_eq!(c.description, "3 files, 2 folders");
    let empty = Listing::default();
    assert_eq!(count(&empty).text, "0 items");
    let one = Listing {
        total: 1,
        files: 1,
        ..Listing::default()
    };
    assert_eq!(count(&one).text, "1 item");
    assert_eq!(count(&one).description, "1 file, 0 folders");
}

#[test]
fn a_selection_lists_folders_then_files_with_the_selected_files_size() {
    let mut l = listing();
    l.selected = selected(3, Some(details(2, 1, 2, 64_000_000)));
    let c = count(&l);
    assert_eq!(c.text, "1 folder, 2 files selected (64.0 MB)");
    assert_eq!(
        c.description,
        "3 of 5 items selected. Size includes selected files only; folder contents are not counted."
    );
}

#[test]
fn unknown_sizes_read_incomplete_or_unavailable_and_folders_alone_have_no_size() {
    let mut l = listing();
    l.selected = selected(2, Some(details(2, 0, 1, 3)));
    assert_eq!(
        count(&l).text,
        "2 files selected (3 B known; size incomplete)"
    );
    l.selected = selected(1, Some(details(1, 0, 0, 0)));
    assert_eq!(count(&l).text, "1 file selected (size unavailable)");
    l.selected = selected(2, Some(details(0, 2, 0, 0)));
    assert_eq!(count(&l).text, "2 folders selected");
}

#[test]
fn selecting_every_item_shows_the_breakdown_without_a_size() {
    let mut l = listing();
    l.selected = selected(5, Some(details(3, 2, 3, 10)));
    assert_eq!(count(&l).text, "2 folders, 3 files selected");
}

#[test]
fn past_the_detailed_limit_the_count_names_only_how_many_are_selected() {
    let l = Listing {
        total: 200,
        files: 200,
        folders: 0,
        selected: selected(65, Some(details(65, 0, 65, 1))),
        filter: None,
    };
    assert_eq!(count(&l).text, "65 items selected");
    let l = Listing {
        selected: selected(64, Some(details(64, 0, 64, 1))),
        ..l
    };
    assert_eq!(count(&l).text, "64 files selected (1 B)");
    let l = Listing {
        selected: selected(3, None),
        ..l
    };
    assert_eq!(count(&l).text, "3 items selected");
}

#[test]
fn displayed_results_count_themselves_whatever_is_selected_and_a_pending_search_keeps_the_directory()
 {
    let mut l = listing();
    l.filter = Some(FilterCounts {
        files: 1,
        folders: 0,
        displayed: true,
    });
    l.selected = selected(1, Some(details(1, 0, 1, 5)));
    let c = count(&l);
    assert_eq!(c.text, "1 item");
    assert_eq!(c.description, "1 file, 0 folders");
    l.filter = Some(FilterCounts::default());
    l.filter.as_mut().unwrap().displayed = true;
    assert_eq!(count(&l).text, "0 items");
    // Typed, not reported back: still the directory (and its selection).
    l.filter.as_mut().unwrap().displayed = false;
    l.selected = None;
    assert_eq!(count(&l).text, "5 items");
}

#[test]
fn sizes_read_as_glib_writes_them() {
    assert_eq!(decimal_size(3), "3 B");
    assert_eq!(decimal_size(1_500), "1.5 kB");
    assert_eq!(decimal_size(64_000_000), "64.0 MB");
}

#[test]
fn the_words_are_the_hosts() {
    let words = CountWords {
        items: std::rc::Rc::new(|n| format!("{n} entries")),
        ..CountWords::default()
    };
    assert_eq!(summary(&listing(), &words).text, "5 entries");
}

#[test]
fn the_english_words_group_digits() {
    let l = Listing {
        total: 1_234,
        files: 1_234,
        ..Listing::default()
    };
    assert_eq!(count(&l).text, "1,234 items");
    assert_eq!(count(&l).description, "1,234 files, 0 folders");
}
