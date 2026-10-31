// SPDX-License-Identifier: MIT

use crate::{
    test_support::gtk_test,
    ui::{
        browser_modes::{BrowserDensity, DensityScale, FrameDensity},
        preferences::PreferenceManager,
        theme::ThemeManager,
    },
};

#[test]
fn density_sliders_restyle_live_and_clear_at_their_presets() {
    gtk_test(
        "ui::theme::tests::density::density_sliders_restyle_live_and_clear_at_their_presets",
        || {
            let themes = ThemeManager::shared();
            let manager = PreferenceManager::shared();
            assert_eq!(manager.browser_density(), BrowserDensity::Compact);
            assert_eq!(manager.frame_density(), FrameDensity::Compact);
            assert_eq!(themes.density_stylesheet(), "");

            manager.set_browser_density(BrowserDensity::from_px(9));
            let css = themes.density_stylesheet();
            assert!(css.contains(".density-custom"));
            assert!(css.contains("margin-top: 9px"));

            manager.set_frame_density(FrameDensity::from_px(5));
            let css = themes.density_stylesheet();
            assert!(css.contains("margin-top: 9px"), "content keeps its value");
            for bar in [
                "headerbar.browser-header",
                ".directory-column .column-header",
                ".preview-header",
                ".sidebar-row",
            ] {
                assert!(css.contains(bar), "frame inset reaches {bar}");
            }
            assert!(css.contains("padding-top: 5px"));

            manager.set_browser_density(BrowserDensity::from_px(8));
            let css = themes.density_stylesheet();
            assert!(
                !css.contains(".density-custom"),
                "Airy uses the static rules"
            );
            assert!(css.contains("padding-top: 5px"), "frame keeps its value");

            manager.set_frame_density(FrameDensity::from_px(0));
            assert_eq!(themes.density_stylesheet(), "");
        },
    );
}
