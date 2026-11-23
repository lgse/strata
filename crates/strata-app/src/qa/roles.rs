// SPDX-License-Identifier: MIT

/// Rune's name for a role, as `Runtime::accessibility_json` spells it, and the AT-SPI name.
pub(super) const ROLES: [(&str, &str); 52] = [
    ("alert", "alert"),
    ("alert-dialog", "alert"),
    ("button", "button"),
    ("cell", "table cell"),
    ("check-box", "check box"),
    ("column-header", "column header"),
    ("combo-box", "combo box"),
    ("default-button", "button"),
    ("dialog", "dialog"),
    ("document", "text"),
    ("grid", "table"),
    ("grid-cell", "table cell"),
    ("group", GROUP),
    ("heading", "heading"),
    ("image", "image"),
    ("label", "label"),
    ("link", "link"),
    ("list", "list"),
    ("list-box", "list"),
    ("list-box-option", "list item"),
    ("list-item", "list item"),
    ("menu", "menu"),
    ("menu-bar", "menu bar"),
    ("menu-item", "menu item"),
    ("menu-item-check-box", "check menu item"),
    ("menu-item-radio", "radio menu item"),
    ("meter", "level bar"),
    ("multiline-text-input", "text"),
    ("password-input", "password text"),
    ("progress-indicator", "progress bar"),
    ("radio-button", "radio button"),
    ("region", GROUP),
    ("row", "table row"),
    ("row-header", "row header"),
    ("scroll-view", "scroll pane"),
    ("search-input", "text"),
    ("slider", "slider"),
    ("spin-button", "spin button"),
    ("splitter", "separator"),
    ("status", "status bar"),
    ("switch", "toggle button"),
    ("tab", "page tab"),
    ("tab-list", "page tab list"),
    ("tab-panel", "panel"),
    ("table", "table"),
    ("text-input", "text"),
    ("toolbar", "tool bar"),
    ("tooltip", "label"),
    ("tree", "tree"),
    ("tree-grid", "tree table"),
    ("tree-item", "tree item"),
    ("window", "frame"),
];

const GROUP: &str = "grouping";
const TOGGLE_BUTTON: &str = "toggle button";
const UNNAMED_GROUP: &str = "panel";
const UNKNOWN: &str = "filler";

pub(super) fn at_spi(role: &str, named: bool) -> &'static str {
    let mapped = ROLES
        .iter()
        .find_map(|(rune, at_spi)| (*rune == role).then_some(*at_spi));
    match mapped {
        Some(GROUP) if !named => UNNAMED_GROUP,
        Some(mapped) => mapped,
        None => UNKNOWN,
    }
}

pub(super) const SEARCH_BOX: &str = "entry";

pub(super) fn is_search_box(role: &str, key: Option<&str>) -> bool {
    let open_with_search = |key: &str| key.starts_with("open-with-") && key.ends_with("-search");
    role == "text-input" && key.is_some_and(open_with_search)
}

pub(super) fn at_spi_toggled(role: &str, named: bool, toggled: bool) -> &'static str {
    if toggled && matches!(role, "button" | "default-button") {
        return TOGGLE_BUTTON;
    }
    at_spi(role, named)
}

#[cfg(test)]
mod tests;
