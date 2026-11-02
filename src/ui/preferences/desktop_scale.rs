// SPDX-License-Identifier: MIT

//! The desktop's text scaling, applied to the interface's root font size.

pub(super) const GTK_DEFAULT_DPI: f64 = 96.0;

pub(super) const GTK_DPI_UNITS: f64 = 1024.0;

pub(in crate::ui) fn desktop_text_scale_factor() -> f64 {
    gtk::Settings::default()
        .map(|settings| text_scale_factor_from_xft_dpi(settings.gtk_xft_dpi()))
        .unwrap_or(1.0)
}

pub(super) fn text_scale_factor_from_xft_dpi(xft_dpi: i32) -> f64 {
    if xft_dpi <= 0 {
        return 1.0;
    }
    f64::from(xft_dpi) / (GTK_DEFAULT_DPI * GTK_DPI_UNITS)
}

pub(in crate::ui) fn snapped_root_font_px(root_font_px: u32, scale_factor: f64) -> f64 {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return f64::from(root_font_px);
    }
    // CSS px bypass Xft DPI. Apply desktop text scaling here, once; GTK applies
    // the surface's monitor scale independently. Keep hinted glyph rows whole.
    (f64::from(root_font_px) * scale_factor).round()
}
