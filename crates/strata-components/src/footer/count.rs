//! The footer's item and selection count, by Strata's rules (`src/ui/shortcut_footer/status.rs`
//! at lgse/strata `upstream/main`), with every word, plural and size as data.

use std::fmt;
use std::rc::Rc;

/// Up to this many selected entries the count lists the folders, the files and the selected
/// files' size; past it, only how many are selected (Strata's `DETAILED_SELECTION`). The host
/// reads per-entry details only up to it.
pub const DETAILED_SELECTION: usize = 64;

/// What the footer counts: the active listing, its selection and a filter over it.
///
/// Used by: hosts, through [`summary`].
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
    /// What they are, read only up to [`DETAILED_SELECTION`] selected entries. `None` past it
    /// (or while unknown): the count then names the selection without a size.
    pub details: Option<SelectionDetails>,
}

/// The selected entries' kinds and the selected files' known sizes. A symlink to a file counts
/// with its target's size, as the size column shows it (the host's).
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
    /// Whether the results are on screen. While a typed query's search hasn't reported back
    /// they aren't, and the count keeps describing the directory.
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
    /// The size after a selection: " (64 MB)", " (3 B known; size incomplete)",
    /// " (size unavailable)".
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

/// A byte count in decimal units, as GLib's `g_format_size` writes it in English: "3 B",
/// "1.5 kB", "64.0 MB". Hosts pass their own (Strata's `i18n::file_size`).
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

/// The footer's count for `listing`, by Strata's rules:
///
/// - A filter whose results are displayed counts its results ("1 item", "0 items"), with their
///   file and folder breakdown as the description, whatever is selected among them.
/// - Otherwise, with nothing selected, "N items", described by the breakdown.
/// - With every entry selected: the listing's folders and files, "1 folder, 2 files selected",
///   with no size.
/// - With more than [`DETAILED_SELECTION`] selected (or no details): "N items selected".
/// - Otherwise the selection's folders and files, with the selected files' size: all known
///   " (64 MB)", some " (3 B known; size incomplete)", none " (size unavailable)". Folder
///   contents are never counted.
/// - A selection's description: "S of N items selected." and the size note.
///
/// Used by: hosts (Strata's footer).
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
    let text = if listing.total > 0 && selected.count == listing.total {
        whole_listing(listing, words)
    } else {
        match selected.details {
            Some(d) if selected.count <= DETAILED_SELECTION => details(&d, words),
            _ => (words.selected)(&(words.items)(selected.count)),
        }
    };
    let selection = (words.selected_of)(selected.count, listing.total);
    Count {
        text,
        description: (words.selection_description)(&selection),
    }
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
