//! What the picker says: its names, its status and its empty state, as data.

/// The picker's words. The defaults are English; a host passes its own translations.
///
/// Used by: hosts.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PickerWords {
    /// The grid's accessible name while it shows a category ("Icon", "Emoji"); a tab's own
    /// name follows it ("Emoji, Animals & Nature").
    pub grid: String,
    /// The search field's name and placeholder.
    pub field: String,
    /// The clear button's name.
    pub clear: String,
    /// The tablist's name.
    pub tabs: String,
    /// The recent choices' tab.
    pub recent: String,
    /// The grid's name while it shows a search's results.
    pub results: String,
    /// What the grid says when a search finds nothing.
    pub no_results: String,
    /// The polite status while searching: how many results there are.
    pub count: fn(usize) -> String,
    /// The description of the cell holding the current choice.
    pub current: String,
    /// The popover's accessible name.
    pub popover: String,
}

impl Default for PickerWords {
    fn default() -> Self {
        Self {
            grid: "Icons".into(),
            field: "Search".into(),
            clear: "Clear search".into(),
            tabs: "Categories".into(),
            recent: "Recently used".into(),
            results: "Search results".into(),
            no_results: "No results".into(),
            count: |n| match n {
                1 => "1 result".into(),
                n => format!("{n} results"),
            },
            current: "Current".into(),
            popover: "Choose an icon".into(),
        }
    }
}
