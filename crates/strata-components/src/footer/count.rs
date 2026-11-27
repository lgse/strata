// SPDX-License-Identifier: MIT

use std::fmt;
use std::rc::Rc;

pub const DETAILED_SELECTION: usize = 64;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// Every entry the listing shows.
    pub total: usize,
    /// Of them, files and folders.
    pub files: usize,
    pub folders: usize,
    /// The selection, if anything is selected.
    pub selected: Option<Selected>,
    /// A filter or search over the listing, if one is open.
    pub filter: Option<FilterCounts>,
}

/// The selection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selected {
    /// How many entries are selected.
    pub count: usize,
    pub details: Option<SelectionDetails>,
    /// The sole selected entry, when `count == 1` and the host names it instead of counting.
    pub single: Option<SingleSelection>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SingleSelection {
    pub name: String,
    pub is_directory: bool,
    pub size: Option<u64>,
    /// Pre-formatted modified date, or `None` when omitted (e.g. List view).
    pub modified: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionDetails {
    pub files: usize,
    pub folders: usize,
    /// Selected files whose size is known.
    pub known_files: usize,
    /// Their sizes, summed.
    pub known_bytes: u64,
}

/// A filter's or search's results.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FilterCounts {
    pub files: usize,
    pub folders: usize,
    pub displayed: bool,
}

/// The count's text and its accessible description.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Count {
    pub text: String,
    pub description: String,
}

/// A count in words: "1 item", "5 items". The host gives its language's plural rules.
pub type Plural = Rc<dyn Fn(usize) -> String>;

/// Words around words: "%{items} selected".
pub type Template = Rc<dyn Fn(&str) -> String>;

/// Words around two words: "%{files}, %{folders}".
pub type Template2 = Rc<dyn Fn(&str, &str) -> String>;

/// A list of parts in words: "1 folder, 2 files".
pub type ListWords = Rc<dyn Fn(&[String]) -> String>;

/// The count's words. [`CountWords::default`] is Strata's English.
#[derive(Clone)]
pub struct CountWords {
    /// "N items", "N files", "N folders".
    pub items: Plural,
    pub files: Plural,
    pub folders: Plural,
    /// A list of parts: "1 folder, 2 files".
    pub list: ListWords,
    /// "%{items} selected".
    pub selected: Template,
    /// The file and folder breakdown: "%{files}, %{folders}".
    pub breakdown: Template2,
    /// "S of N items selected.", the plural following the total N.
    pub selected_of: Rc<dyn Fn(usize, usize) -> String>,
    /// The selection's description: "%{selection} Size includes selected files only; …".
    pub selection_description: Template,
    /// A size: "64 MB".
    pub size: Rc<dyn Fn(u64) -> String>,
    pub size_known: Template,
    pub size_incomplete: Template,
    pub size_unavailable: String,
}

impl fmt::Debug for CountWords {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CountWords").finish_non_exhaustive()
    }
}

fn english(one: &'static str, many: &'static str) -> Plural {
    Rc::new(move |n| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{} {many}", grouped(n))
        }
    })
}

/// `n` with its digits in groups of three, as Strata's English writes counts ("1,234").
pub fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl Default for CountWords {
    fn default() -> Self {
        Self {
            items: english("item", "items"),
            files: english("file", "files"),
            folders: english("folder", "folders"),
            list: Rc::new(|parts| parts.join(", ")),
            selected: Rc::new(|items| format!("{items} selected")),
            breakdown: Rc::new(|files, folders| format!("{files}, {folders}")),
            selected_of: Rc::new(|selected, total| {
                let selected = grouped(selected);
                if total == 1 {
                    format!("{selected} of 1 item selected.")
                } else {
                    format!("{selected} of {} items selected.", grouped(total))
                }
            }),
            selection_description: Rc::new(|selection| {
                format!(
                    "{selection} Size includes selected files only; folder contents are not counted."
                )
            }),
            size: Rc::new(decimal_size),
            size_known: Rc::new(|size| format!(" ({size})")),
            size_incomplete: Rc::new(|size| format!(" ({size} known; size incomplete)")),
            size_unavailable: " (size unavailable)".to_owned(),
        }
    }
}

pub fn decimal_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["kB", "MB", "GB", "TB", "PB", "EB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

pub fn summary(listing: &Listing, words: &CountWords) -> Count {
    if let Some(filter) = listing.filter.filter(|f| f.displayed) {
        return Count {
            text: (words.items)(filter.files + filter.folders),
            description: breakdown(words, filter.files, filter.folders),
        };
    }
    let Some(selected) = listing.selected.as_ref().filter(|s| s.count > 0) else {
        return Count {
            text: (words.items)(listing.total),
            description: breakdown(words, listing.files, listing.folders),
        };
    };
    let text = selection_text(listing, selected, words);
    let selection = (words.selected_of)(selected.count, listing.total);
    Count {
        text,
        description: (words.selection_description)(&selection),
    }
}

fn selection_text(listing: &Listing, selected: &Selected, words: &CountWords) -> String {
    if listing.total > 0 && selected.count == listing.total {
        return whole_listing(listing, words);
    }
    if selected.count > DETAILED_SELECTION {
        return (words.selected)(&(words.items)(selected.count));
    }
    if selected.count == 1
        && let Some(entry) = selected.single.as_ref()
    {
        return single_entry(entry, words);
    }
    match selected.details {
        Some(d) => details(&d, words),
        None => (words.selected)(&(words.items)(selected.count)),
    }
}

fn single_entry(entry: &SingleSelection, words: &CountWords) -> String {
    let mut text = entry.name.clone();
    if !entry.is_directory {
        match entry.size {
            Some(bytes) => text.push_str(&(words.size_known)(&(words.size)(bytes))),
            None => text.push_str(&words.size_unavailable),
        }
    }
    if let Some(date) = entry.modified.as_deref().filter(|d| !d.is_empty()) {
        text.push_str(&format!(" — {date}"));
    }
    text
}

fn breakdown(words: &CountWords, files: usize, folders: usize) -> String {
    (words.breakdown)(&(words.files)(files), &(words.folders)(folders))
}

fn parts(words: &CountWords, files: usize, folders: usize) -> Vec<String> {
    let mut parts = Vec::new();
    if folders > 0 {
        parts.push((words.folders)(folders));
    }
    if files > 0 {
        parts.push((words.files)(files));
    }
    parts
}

fn whole_listing(listing: &Listing, words: &CountWords) -> String {
    let parts = parts(words, listing.files, listing.folders);
    let items = if parts.is_empty() {
        (words.items)(listing.total)
    } else {
        (words.list)(&parts)
    };
    (words.selected)(&items)
}

fn details(d: &SelectionDetails, words: &CountWords) -> String {
    let mut text = (words.selected)(&(words.list)(&parts(words, d.files, d.folders)));
    if d.files > 0 {
        let size = (words.size)(d.known_bytes);
        if d.known_files >= d.files {
            text.push_str(&(words.size_known)(&size));
        } else if d.known_files > 0 {
            text.push_str(&(words.size_incomplete)(&size));
        } else {
            text.push_str(&words.size_unavailable);
        }
    }
    text
}
