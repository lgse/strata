// SPDX-License-Identifier: MIT

//! Row, card, and chrome spacing scales, and the CSS a Custom spacing needs.

/// One position on a density slider: a px value, where `COMPACT_PX` and `AIRY_PX`
/// are the snap points and every other value in `0..=MAX_PX` is Custom.
pub(in crate::ui) trait DensityScale: Copy + PartialEq + 'static {
    const COMPACT_PX: u8;
    const AIRY_PX: u8;
    const MAX_PX: u8;
    fn from_px(px: u8) -> Self;
    fn px(self) -> u8;
}

macro_rules! density_scale {
    ($(#[$meta:meta])* $name:ident { compact: $compact:literal, airy: $airy:literal, max: $max:literal }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
        pub enum $name {
            #[default]
            Compact,
            Airy,
            /// `from_px` maps snap-point values to their preset rather than to Custom.
            Custom(u8),
        }

        impl DensityScale for $name {
            const COMPACT_PX: u8 = $compact;
            const AIRY_PX: u8 = $airy;
            const MAX_PX: u8 = $max;

            fn from_px(px: u8) -> Self {
                match px.min(Self::MAX_PX) {
                    $compact => Self::Compact,
                    $airy => Self::Airy,
                    px => Self::Custom(px),
                }
            }

            fn px(self) -> u8 {
                match self {
                    Self::Compact => Self::COMPACT_PX,
                    Self::Airy => Self::AIRY_PX,
                    Self::Custom(px) => px.min(Self::MAX_PX),
                }
            }
        }
    };
}

density_scale!(
    /// File-row and icon-card vertical spacing in logical px.
    BrowserDensity { compact: 4, airy: 8, max: 16 }
);

density_scale!(
    /// Extra vertical inset shared by the header, tab strip, sidebar rows, and
    /// Miller/preview headers, so their heights stay in lock step.
    FrameDensity { compact: 0, airy: 4, max: 12 }
);

/// Spacing for a Custom slider position, extrapolated linearly through the static
/// Compact and Airy rules in `style.css` so the slider has no jump at either preset.
pub(crate) fn browser_density_custom_css(px: u8) -> String {
    let step = (f64::from(px) - f64::from(BrowserDensity::COMPACT_PX))
        / f64::from(BrowserDensity::AIRY_PX - BrowserDensity::COMPACT_PX);
    let between = |compact: f64, airy: f64| (compact + (airy - compact) * step).max(0.0).round();
    let row_margin = px;
    let row_content_margin = px.min(4);
    let row_min_height = between(26.0, 42.0);
    let grid_padding = between(0.0, 5.0);
    let child_margin = between(1.0, 3.0);
    let card_vertical = between(2.0, 5.0);
    let card_horizontal = between(4.0, 7.0);
    format!(
        ".density-custom .file-row, .mode-list.density-custom .list-row {{ min-height: {row_min_height}px; }}\n\
         .density-custom .file-row > *, .mode-list.density-custom .list-row > * {{ margin-top: {row_margin}px; margin-bottom: {row_margin}px; }}\n\
         .density-custom .file-row > .file-row-content {{ margin-top: {row_content_margin}px; margin-bottom: {row_content_margin}px; }}\n\
         .mode-icons.density-custom .file-icons {{ padding: {grid_padding}px; }}\n\
         .mode-icons.density-custom .file-icons > child {{ margin: {child_margin}px; }}\n\
         .mode-icons.density-custom .icons-card {{ padding: {card_vertical}px {card_horizontal}px; }}\n"
    )
}
