// SPDX-License-Identifier: MIT

//! Stylesheets generated from the theme tokens and density preferences.

use super::ThemeTokens;

/// One inset for every bar so the header, tab strip, sidebar rows, and Miller and
/// preview headers grow together and stay aligned. Padding and `margin-left` only,
/// so `sizing`'s `min-height` and fixed button widths still hold. The sidebar toggle
/// keeps its static margin, which lines it up with the sidebar icons below it.
pub(super) fn frame_density_css(inset: u8) -> String {
    if inset == 0 {
        return String::new();
    }
    // Miller headers keep their two extra bottom pixels that balance the top indicator.
    let column_header_bottom = inset + 2;
    format!(
        "headerbar.browser-header, .tab-strip, .mode-pane-header, .preview-header, .sidebar-row {{ padding-top: {inset}px; padding-bottom: {inset}px; }}\n\
         .directory-column .column-header {{ padding-top: {inset}px; padding-bottom: {column_header_bottom}px; }}\n\
         headerbar.browser-header button.header-action, headerbar.browser-header menubutton.header-action > button, .tab-strip button.header-action, .directory-column .column-header button.column-header-action, .directory-column .column-header menubutton.column-header-action > button, .mode-pane-header button.column-header-action, .mode-pane-header menubutton.column-header-action > button, .mode-pane-header .list-navigation-button, .preview-header .preview-header-action {{ margin-left: {inset}px; }}\n"
    )
}

pub(super) fn tokens_css(tokens: &ThemeTokens, root_font_px: f64) -> String {
    let scale = root_font_px / 13.0;
    let header = (40.0 * scale).round();
    // Column headers add six pixels of padding and three extra border pixels.
    let column_header = header - 9.0;
    let control = (24.0 * scale).round();
    // Sidebar icons are centred in control-sized boxes; headings start where the icons do.
    let sidebar_heading_inset = (control - (17.0 * scale).round()) / 2.0;
    // The spinner keeps its fixed size, so this centres it on the scaled action icon.
    let sidebar_spinner_inset = ((14.0 * scale).round() - 14.0) / 2.0 + 1.0;
    let browser_header = (control + 8.0).max(36.0);
    // Miller headers keep their three-pixel indicator, bottom border and balancing inset.
    let browser_column_header = browser_header - 6.0;
    let sizing = format!(
        "headerbar, headerbar > windowhandle > box, .mode-pane-header, .preview-header {{ min-height: {header}px; }}\n.column-header {{ min-height: {column_header}px; }}\nheaderbar .sidebar-toggle, button.header-action, headerbar menubutton.header-action > button, .preview-header-action, button.column-header-action, menubutton.column-header-action > button {{ min-width: {control}px; min-height: {control}px; }}\n.file-operation-card button.progress-card-action {{ min-width: 0; min-height: 0; }}\n.tab-strip, headerbar.browser-header, headerbar.browser-header > windowhandle > box, .mode-pane-header, .preview-header {{ min-height: {browser_header}px; }}\n.directory-column .column-header {{ min-height: {browser_column_header}px; }}\n.mode-pane-header .list-navigation-button, .sidebar-row > box > image, .sidebar-update > box > image, .sidebar-device-action {{ min-width: {control}px; min-height: {control}px; }}\n.sidebar-heading-chevron {{ min-height: {control}px; }}\n.sidebar-heading {{ padding-left: {sidebar_heading_inset}px; }}\n.sidebar-device-spinner {{ margin-right: {sidebar_spinner_inset}px; }}\n"
    );
    let colors = format!(
        "@define-color strata_bg {};\n@define-color strata_surface {};\n@define-color strata_text {};\n@define-color strata_accent {};\n@define-color strata_danger {};\n@define-color strata_muted {};\n@define-color strata_highlight {};\n@define-color strata_border {};\n@define-color strata_dim_text {};\nwindow, popover, popover.background {{ font-size: {root_font_px:.6}px; }}\n",
        tokens.background,
        tokens.surface,
        tokens.text,
        tokens.accent,
        tokens.danger,
        tokens.muted,
        tokens.highlight,
        tokens.border,
        tokens.dim_text,
    );
    colors + &sizing
}
