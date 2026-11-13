// SPDX-License-Identifier: MIT

//! Rune's role names as the AT-SPI names the scenarios use. The protocol's role names stay
//! AT-SPI's, which the GTK harness reports (`src/qa/targets/naming.rs`).

/// Rune's name for a role, as `Runtime::accessibility_json` spells it, and the AT-SPI name.
pub(super) const ROLES: [(&str, &str); 44] = [
    ("alert", "alert"),
    ("alert-dialog", "alert"),
    ("button", "button"),
    ("cell", "table cell"),
    ("check-box", "check box"),
    ("column-header", "column header"),
    ("combo-box", "combo box"),
    ("default-button", "button"),
    ("dialog", "dialog"),
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
    ("table", "table"),
    ("text-input", "text"),
    ("window", "frame"),
];

const GROUP: &str = "grouping";
const TOGGLE_BUTTON: &str = "toggle button";
/// What GTK reports for a container with no accessible name, and for a role it has no name
/// for: a scenario's `grouping` must not find Rune's layout groups.
const UNNAMED_GROUP: &str = "panel";
const UNKNOWN: &str = "filler";

/// The AT-SPI name for Rune's `role`. A group counts as one only when it has a name: Rune
/// keeps every keyed box as a group, where GTK reports a plain container as a panel.
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

/// The AT-SPI name for Rune's `role` on a node that reports a pressed state (`toggled`) or
/// not. A button with one is a toggle button, as AccessKit hands it to AT-SPI (`Role::Button`
/// with `toggled`, in `accesskit_atspi_common`) and as GTK's toggle buttons are.
pub(super) fn at_spi_toggled(role: &str, named: bool, toggled: bool) -> &'static str {
    if toggled && role == "button" {
        return TOGGLE_BUTTON;
    }
    at_spi(role, named)
}

#[cfg(test)]
mod tests;
