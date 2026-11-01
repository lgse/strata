// SPDX-License-Identifier: MIT

//! Theme, motion, window controls, typography and density: how the interface looks.

use super::*;

impl PreferenceManager {
    pub(in crate::ui) fn theme_mode(&self) -> String {
        self.preferences.borrow().mode.clone()
    }

    pub(in crate::ui) fn selected_theme_id(&self) -> String {
        self.preferences.borrow().theme.clone()
    }

    /// Writes the theme selection as one change so mode and theme publish together.
    pub(in crate::ui) fn set_theme_selection(&self, mode: &str, theme: Option<&str>) {
        {
            let mut preferences = self.preferences.borrow_mut();
            preferences.mode = mode.to_owned();
            if let Some(theme) = theme {
                preferences.theme = theme.to_owned();
            }
        }
        self.save_preferences();
    }

    pub fn window_show_close(&self) -> bool {
        self.preferences.borrow().window_show_close
    }

    pub fn set_window_show_close(&self, enabled: bool) {
        self.preferences.borrow_mut().window_show_close = enabled;
        self.save_preferences();
    }

    pub fn window_show_minimize(&self) -> bool {
        self.preferences.borrow().window_show_minimize
    }

    pub fn set_window_show_minimize(&self, enabled: bool) {
        self.preferences.borrow_mut().window_show_minimize = enabled;
        self.save_preferences();
    }

    pub fn window_show_maximize(&self) -> bool {
        self.preferences.borrow().window_show_maximize
    }

    pub fn set_window_show_maximize(&self, enabled: bool) {
        self.preferences.borrow_mut().window_show_maximize = enabled;
        self.save_preferences();
    }

    /// The variant remembered for the active Omarchy theme.
    pub fn omarchy_variant(&self) -> OmarchyVariant {
        let active = self.active_omarchy_theme.borrow();
        active
            .as_ref()
            .and_then(|theme| {
                self.preferences
                    .borrow()
                    .omarchy_variants
                    .get(theme)
                    .copied()
            })
            .unwrap_or_default()
    }

    pub fn set_omarchy_variant(&self, variant: OmarchyVariant) {
        let Some(theme) = self.active_omarchy_theme.borrow().clone() else {
            return;
        };
        if self.omarchy_variant() == variant {
            return;
        }
        {
            let variants = &mut self.preferences.borrow_mut().omarchy_variants;
            if variant == OmarchyVariant::default() {
                variants.remove(&theme);
            } else {
                variants.insert(theme, variant);
            }
        }
        self.save_preferences();
    }

    /// Records which Omarchy theme is current without publishing; the theme
    /// manager reapplies and notifies once its palette state is updated.
    pub(in crate::ui) fn set_active_omarchy_theme(&self, theme: &str) {
        let theme = theme.trim().to_owned();
        let mut preferences = self.preferences.borrow_mut();
        // Older versions always saved "original", so only a contrast choice was deliberate.
        if let Some(legacy) = preferences.legacy_omarchy_variant.take()
            && legacy == OmarchyVariant::Contrast
        {
            preferences
                .omarchy_variants
                .entry(theme.clone())
                .or_insert(legacy);
        }
        self.active_omarchy_theme.replace(Some(theme));
    }

    pub fn element_glow(&self) -> bool {
        self.preferences.borrow().element_glow
    }

    pub fn set_element_glow(&self, enabled: bool) {
        if self.element_glow() == enabled {
            return;
        }
        self.preferences.borrow_mut().element_glow = enabled;
        self.save_preferences();
    }

    pub fn file_icon_colors(&self) -> FileIconColors {
        self.preferences.borrow().file_icon_colors
    }

    pub fn set_file_icon_colors(&self, colors: FileIconColors) {
        if self.file_icon_colors() == colors {
            return;
        }
        self.preferences.borrow_mut().file_icon_colors = colors;
        self.save_preferences();
    }

    pub fn reduce_motion(&self) -> bool {
        self.preferences.borrow().reduce_motion
    }

    pub fn set_reduce_motion(&self, reduced: bool) {
        self.preferences.borrow_mut().reduce_motion = reduced;
        super::super::motion::set_reduce_motion(reduced);
        self.save_preferences();
    }

    pub fn browser_mode(&self) -> super::super::browser_modes::BrowserMode {
        browser_mode_from_stored(&self.preferences.borrow().browser_mode)
    }

    pub fn set_browser_mode(&self, mode: super::super::browser_modes::BrowserMode) {
        self.preferences.borrow_mut().browser_mode = stored_browser_mode(mode).to_owned();
        self.save_preferences();
    }

    pub fn browser_density(&self) -> super::super::browser_modes::BrowserDensity {
        let preferences = self.preferences.borrow();
        stored_density(
            &preferences.browser_density,
            preferences.browser_density_custom_px,
        )
    }

    pub fn set_browser_density(&self, density: super::super::browser_modes::BrowserDensity) {
        {
            let mut preferences = self.preferences.borrow_mut();
            let preferences = &mut *preferences;
            store_density(
                density,
                &mut preferences.browser_density,
                &mut preferences.browser_density_custom_px,
            );
        }
        self.save_preferences();
    }

    pub fn frame_density(&self) -> super::super::browser_modes::FrameDensity {
        let preferences = self.preferences.borrow();
        stored_density(
            &preferences.frame_density,
            preferences.frame_density_custom_px,
        )
    }

    pub fn set_frame_density(&self, density: super::super::browser_modes::FrameDensity) {
        {
            let mut preferences = self.preferences.borrow_mut();
            let preferences = &mut *preferences;
            store_density(
                density,
                &mut preferences.frame_density,
                &mut preferences.frame_density_custom_px,
            );
        }
        self.save_preferences();
    }

    pub fn language(&self) -> crate::i18n::Language {
        self.preferences.borrow().language
    }

    pub fn set_language(&self, language: crate::i18n::Language) {
        self.preferences.borrow_mut().language = language;
        self.save_preferences();
    }

    pub fn language_restart_required(&self) -> bool {
        self.language().locale() != self.startup_locale
    }

    pub fn interface_renderer(&self) -> InterfaceRenderer {
        self.preferences.borrow().interface_renderer
    }

    pub fn set_interface_renderer(&self, renderer: InterfaceRenderer) {
        self.preferences.borrow_mut().interface_renderer = renderer;
        self.save_preferences();
    }

    pub fn interface_renderer_restart_required(&self) -> bool {
        self.interface_renderer() != self.startup_interface_renderer
    }

    pub fn text_size(&self) -> TextSize {
        self.preferences.borrow().text_size
    }

    pub fn set_text_size(&self, size: TextSize) {
        if self.text_size() == size {
            self.save_preferences();
            return;
        }
        self.preferences.borrow_mut().text_size = size;
        self.save_preferences();
    }

    pub fn corner_roundness(&self) -> CornerRoundness {
        self.preferences.borrow().corner_roundness
    }

    pub fn set_corner_roundness(&self, roundness: CornerRoundness) {
        if self.corner_roundness() == roundness {
            return;
        }
        self.preferences.borrow_mut().corner_roundness = roundness;
        self.save_preferences();
    }

    /// The chosen interface font family; `None` keeps the bundled default.
    pub fn interface_font(&self) -> Option<String> {
        self.preferences.borrow().interface_font.clone()
    }

    pub fn set_interface_font(&self, family: Option<String>) {
        let family = family
            .map(|family| family.trim().to_owned())
            .filter(|family| !family.is_empty());
        if self.interface_font() == family {
            return;
        }
        self.preferences.borrow_mut().interface_font = family;
        self.save_preferences();
    }

    pub fn interface_scale(&self) -> f64 {
        snapped_root_font_px(self.text_size().root_font_px(), desktop_text_scale_factor()) / 13.0
    }

    pub fn bind_interface_scale(
        self: &Rc<Self>,
        anchor: &impl IsA<gtk::Widget>,
        apply: impl Fn(&gtk::Widget, f64) + 'static,
    ) {
        let apply = Rc::new(apply);
        let changed = apply.clone();
        self.bind_preference(anchor, Self::interface_scale, move |widget, scale| {
            changed(widget, scale)
        });
        if let Some(settings) = gtk::Settings::default() {
            let widget = anchor.downgrade();
            let manager = Rc::downgrade(self);
            let handler = settings.connect_gtk_xft_dpi_notify(move |_| {
                if let (Some(widget), Some(manager)) = (widget.upgrade(), manager.upgrade()) {
                    apply(widget.upcast_ref(), manager.interface_scale());
                }
            });
            let handler = RefCell::new(Some(handler));
            anchor.connect_destroy(move |_| {
                if let Some(handler) = handler.borrow_mut().take() {
                    settings.disconnect(handler);
                }
            });
        }
    }
}
