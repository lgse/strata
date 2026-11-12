// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    fs, io,
    path::{Path, PathBuf},
    rc::Rc,
};

use gtk::{gdk, glib, prelude::*};
use serde::{Deserialize, Serialize};

use super::browser_modes::DensityScale;
use super::preferences::{
    CornerRoundness, FileIconColors, OmarchyVariant, PreferenceManager, TextSize,
    desktop_text_scale_factor, snapped_root_font_px,
};
use crate::services::LoadHandle;

mod color;
mod document_style;
mod font;
mod generated_css;
mod gtk_settings;
mod library;
mod listeners;
mod omarchy;
mod omarchy_state;
mod portal_text_scale;
mod roundness;
mod source_style;
mod system_appearance;
mod watch;
mod weak_registry;

use color::{blend, canonical_color};
pub(crate) use color::{color_to_hex, parse_rgb_channels};
pub(super) use document_style::{register_document_buffer, register_document_view};
use document_style::{style_document_buffers, style_document_views};
use generated_css::{frame_density_css, tokens_css};
use gtk_settings::{
    apply_base_variant, apply_interface_font, base_theme_import, pin_base_theme,
    pin_manual_font_rendering,
};
use library::{
    azure_tokens, builtins, custom_theme_id, is_custom_theme_event, linked_theme_targets,
    load_custom_themes, merge_builtin_and_custom_themes, themes_directory, validate_tokens,
};
use listeners::{ThemeListeners, on_change};
use omarchy_state::{OmarchyRefresh, OmarchyState, is_omarchy_theme_event};
pub(super) use source_style::{new_source_buffer, source_languages};
use source_style::{resolved_source_palette, stage_source_style_scheme};
use system_appearance::{PortalStartup, SystemAppearance};
use watch::{read_user_stylesheet, user_stylesheet_path};

// Read by the test modules through their `super::*` globs.
#[cfg(test)]
use self::{
    gtk_settings::PINNED_GTK_THEME,
    library::slugify,
    omarchy_state::{source_palette_from_quattro, title_case_slug, tokens_from_quattro},
    source_style::source_style_scheme_xml,
};
#[cfg(test)]
pub(in crate::ui) use omarchy_state::omarchy_state_dir;

/// Above GTK's user priority, every property is unset and the built-in theme
/// restated, so `~/.config/gtk-4.0/gtk.css` cannot decide any of them.
const RESET_STYLE_PRIORITY: u32 = gtk::STYLE_PROVIDER_PRIORITY_USER + 1;
const BASE_THEME_STYLE_PRIORITY: u32 = RESET_STYLE_PRIORITY + 1;
/// Below Strata's stylesheet, so its intentional monospace rules still win.
const FONT_STYLE_PRIORITY: u32 = BASE_THEME_STYLE_PRIORITY + 1;
const STYLE_PRIORITY: u32 = FONT_STYLE_PRIORITY + 1;
const RESET_CSS: &str = "* { all: unset; }";
/// Above the static stylesheet so generated density spacing wins over its presets.
const DENSITY_STYLE_PRIORITY: u32 = STYLE_PRIORITY + 1;
pub(in crate::ui) const TOKEN_STYLE_PRIORITY: u32 = DENSITY_STYLE_PRIORITY + 1;
/// Above the token and roundness styles so the uncomposited fallback wins over them.
const UNCOMPOSITED_STYLE_PRIORITY: u32 = TOKEN_STYLE_PRIORITY + 1;
/// Above every Strata stylesheet, so `~/.config/strata/style.css` has the final word.
const USER_STYLE_PRIORITY: u32 = UNCOMPOSITED_STYLE_PRIORITY + 1;

const STYLE_CSS: &str = include_str!("../style.css");

/// Without a compositor (X11 with no compositing manager) the transparent
/// shadow and corner pixels of popup surfaces render solid black.
const UNCOMPOSITED_CSS: &str = "\
tooltip,
popover,
popover contents {
    box-shadow: none;
    border-radius: 0;
}
";

thread_local! {
    static SHARED_MANAGER: RefCell<Option<Rc<ThemeManager>>> = const { RefCell::new(None) };
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ThemeTokens {
    pub name: String,
    pub background: String,
    pub surface: String,
    pub text: String,
    pub accent: String,
    #[serde(default = "default_danger")]
    pub danger: String,
    pub muted: String,
    pub highlight: String,
    pub border: String,
    pub dim_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syntax_keyword: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syntax_string: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syntax_constant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syntax_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syntax_preprocessor: Option<String>,
}

#[derive(Clone)]
struct SourcePalette {
    statement: String,
    string: String,
    constant: String,
    type_color: String,
    preprocessor: String,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub id: String,
    pub tokens: ThemeTokens,
    pub custom: bool,
}

/// Appearance preferences that change the shared CSS or icon rendering the manager applies.
#[derive(Clone, Debug, PartialEq)]
struct AppearancePreferences {
    mode: String,
    theme: String,
    omarchy_variant: OmarchyVariant,
    text_size: TextSize,
    corner_roundness: CornerRoundness,
    interface_font: Option<String>,
    element_glow: bool,
    file_icon_colors: FileIconColors,
}

impl AppearancePreferences {
    fn read(preferences: &PreferenceManager) -> Self {
        Self {
            mode: preferences.theme_mode(),
            theme: preferences.selected_theme_id(),
            omarchy_variant: preferences.omarchy_variant(),
            text_size: preferences.text_size(),
            corner_roundness: preferences.corner_roundness(),
            interface_font: preferences.interface_font(),
            element_glow: preferences.element_glow(),
            file_icon_colors: preferences.file_icon_colors(),
        }
    }
}

pub struct ThemeManager {
    provider: gtk::CssProvider,
    reset_provider: gtk::CssProvider,
    base_theme_provider: gtk::CssProvider,
    font_provider: gtk::CssProvider,
    base_provider: gtk::CssProvider,
    density_provider: gtk::CssProvider,
    density: Cell<
        Option<(
            super::browser_modes::BrowserDensity,
            super::browser_modes::FrameDensity,
        )>,
    >,
    uncomposited_provider: gtk::CssProvider,
    user_provider: gtk::CssProvider,
    base_roundness: Cell<Option<CornerRoundness>>,
    themes: RefCell<Vec<Theme>>,
    preferences: Rc<PreferenceManager>,
    system_appearance: SystemAppearance,
    omarchy_available: Cell<bool>,
    omarchy_state: RefCell<Option<OmarchyState>>,
    omarchy_monitors: RefCell<Vec<(PathBuf, LoadHandle)>>,
    pending_omarchy_refresh: RefCell<Option<glib::SourceId>>,
    themes_monitor: RefCell<Option<LoadHandle>>,
    linked_theme_monitors: RefCell<Vec<(PathBuf, LoadHandle)>>,
    linked_theme_targets: RefCell<Vec<PathBuf>>,
    pending_themes_refresh: RefCell<Option<glib::SourceId>>,
    user_style_monitor: RefCell<Option<LoadHandle>>,
    pending_user_style_refresh: RefCell<Option<glib::SourceId>>,
    preview: RefCell<Option<ThemeTokens>>,
    preview_generation: Cell<u64>,
    /// Omarchy state can be unreadable mid-switch while the window keeps these colors.
    applied_omarchy_tokens: RefCell<Option<ThemeTokens>>,
    appearance: RefCell<AppearancePreferences>,
    base_variant_dark: Cell<Option<bool>>,
    theme_listeners: ThemeListeners,
    active_model_palette: Cell<crate::services::ModelPalette>,
}

impl ThemeManager {
    pub fn shared() -> Rc<Self> {
        Self::shared_with_portal_startup(PortalStartup::WaitForInitialValue)
    }

    pub(crate) fn shared_without_portal_wait() -> Rc<Self> {
        Self::shared_with_portal_startup(PortalStartup::AsyncOnly)
    }

    fn shared_with_portal_startup(startup: PortalStartup) -> Rc<Self> {
        let existing = SHARED_MANAGER.with(|shared| shared.borrow().clone());
        if let Some(manager) = existing {
            return manager;
        }
        let manager = Self::load(startup);
        SHARED_MANAGER.with(|shared| shared.replace(Some(manager.clone())));
        manager
    }

    fn load(startup: PortalStartup) -> Rc<Self> {
        let preferences = PreferenceManager::shared();
        let themes = merge_builtin_and_custom_themes(builtins(), load_custom_themes());
        // Startup blocks on the portal only when its value picks the first palette.
        let startup = if preferences.theme_mode() == "system" {
            startup
        } else {
            PortalStartup::AsyncOnly
        };
        let system_appearance = SystemAppearance::read(startup);
        let omarchy = OmarchyRefresh::read();
        if let Some(state) = &omarchy.state {
            preferences.set_active_omarchy_theme(&state.name);
        }
        preferences.normalize_loaded_theme_selection(
            |id| themes.iter().any(|theme| theme.id == id),
            system_appearance.is_available(),
            has_omarchy_theme(omarchy.state.as_ref()),
        );
        let manager = Rc::new(Self::new(
            preferences,
            themes,
            system_appearance,
            omarchy.state,
        ));
        manager.start(&omarchy.directories, omarchy.theme_directory.as_deref());
        manager
    }

    fn new(
        preferences: Rc<PreferenceManager>,
        themes: Vec<Theme>,
        system_appearance: SystemAppearance,
        omarchy_state: Option<OmarchyState>,
    ) -> Self {
        let omarchy_available = has_omarchy_theme(omarchy_state.as_ref());
        let appearance = AppearancePreferences::read(&preferences);
        Self {
            provider: gtk::CssProvider::new(),
            reset_provider: gtk::CssProvider::new(),
            base_theme_provider: gtk::CssProvider::new(),
            font_provider: gtk::CssProvider::new(),
            base_provider: gtk::CssProvider::new(),
            density_provider: gtk::CssProvider::new(),
            density: Cell::new(None),
            uncomposited_provider: gtk::CssProvider::new(),
            user_provider: gtk::CssProvider::new(),
            base_roundness: Cell::new(None),
            themes: RefCell::new(themes),
            preferences,
            system_appearance,
            omarchy_available: Cell::new(omarchy_available),
            omarchy_state: RefCell::new(omarchy_state),
            omarchy_monitors: RefCell::new(Vec::new()),
            pending_omarchy_refresh: RefCell::new(None),
            themes_monitor: RefCell::new(None),
            linked_theme_monitors: RefCell::new(Vec::new()),
            linked_theme_targets: RefCell::new(Vec::new()),
            pending_themes_refresh: RefCell::new(None),
            user_style_monitor: RefCell::new(None),
            pending_user_style_refresh: RefCell::new(None),
            preview: RefCell::new(None),
            preview_generation: Cell::new(0),
            applied_omarchy_tokens: RefCell::new(None),
            appearance: RefCell::new(appearance),
            base_variant_dark: Cell::new(None),
            theme_listeners: ThemeListeners::default(),
            active_model_palette: Cell::new(crate::services::ModelPalette {
                accent: 0,
                surface: 0,
            }),
        }
    }

    /// Applies the theme and starts following the preferences and files it reads.
    fn start(self: &Rc<Self>, omarchy_directories: &[PathBuf], omarchy_theme: Option<&Path>) {
        let weak = Rc::downgrade(self);
        self.preferences.observe(Rc::new(move || {
            if let Some(manager) = weak.upgrade() {
                manager.on_preferences_changed();
            }
        }));
        super::motion::sync_toolkit();
        pin_manual_font_rendering();
        pin_base_theme();
        self.install_provider();
        self.apply_base_stylesheet();
        self.apply_density();
        self.apply_selected();
        self.monitor_text_scaling();
        self.monitor_system_appearance();
        portal_text_scale::follow_if_portals_disabled();
        self.monitor_omarchy(omarchy_directories, omarchy_theme);
        self.monitor_custom_themes(themes_directory().is_dir());
        self.monitor_linked_themes(linked_theme_targets(&themes_directory()));
        self.user_provider.load_from_string(&read_user_stylesheet());
        self.monitor_user_stylesheet();
    }
    pub fn themes(&self) -> Vec<Theme> {
        self.themes.borrow().clone()
    }

    pub fn is_omarchy_available(&self) -> bool {
        self.omarchy_available.get()
    }

    pub fn follows_omarchy(&self) -> bool {
        self.preferences.theme_mode() == "omarchy"
    }

    pub fn selected_id(&self) -> String {
        self.preferences.selected_theme_id()
    }

    pub(crate) fn active_model_palette(&self) -> crate::services::ModelPalette {
        self.active_model_palette.get()
    }

    /// Applies the current value immediately, then only changes to that value.
    /// Capture weak references to owned widgets/state; the anchor owns the binding's lifetime.
    pub(crate) fn bind_theme_preference<T: PartialEq + Clone + 'static>(
        &self,
        anchor: &impl IsA<gtk::Widget>,
        read: impl Fn(&Self) -> T + 'static,
        apply: impl Fn(&gtk::Widget, T) + 'static,
    ) {
        self.theme_listeners
            .bind(self, anchor, on_change(read, apply));
    }

    pub fn select_theme(&self, id: &str) {
        if !self.themes.borrow().iter().any(|theme| theme.id == id) {
            return;
        }
        self.preview.take();
        let mode = if self.follows_system_appearance() {
            "system"
        } else {
            "theme"
        };
        let changed = self.preferences.theme_mode() != mode || self.selected_id() != id;
        self.preferences.set_theme_selection(mode, Some(id));
        if !changed {
            // Re-selecting the current theme restores it after a live preview.
            self.apply_selected();
        }
    }

    pub fn set_follow_omarchy(&self, enabled: bool) {
        if enabled && !self.is_omarchy_available() {
            return;
        }
        self.preview.take();
        let changed = self.follows_omarchy() != enabled;
        let mode = if enabled { "omarchy" } else { "theme" };
        self.preferences.set_theme_selection(mode, None);
        if !changed {
            self.apply_selected();
        }
    }

    /// Applies `tokens` until [`Self::cancel_preview`] or a theme selection; appearance
    /// changes in the meantime re-apply the preview rather than the saved theme.
    /// Returns the preview's generation for [`Self::cancel_preview_from`].
    pub fn preview(&self, tokens: &ThemeTokens) -> Option<u64> {
        validate_tokens(tokens).ok()?;
        self.preview.replace(Some(tokens.clone()));
        self.preview_generation
            .set(self.preview_generation.get().wrapping_add(1));
        self.apply_tokens(tokens, None);
        Some(self.preview_generation.get())
    }

    pub fn cancel_preview(&self) {
        if self.preview.take().is_some() {
            self.apply_selected();
        }
    }

    /// Cancels the preview only if no later one has replaced it.
    pub fn cancel_preview_from(&self, generation: u64) {
        if self.preview_generation.get() == generation {
            self.cancel_preview();
        }
    }

    #[cfg(test)]
    pub(crate) fn is_previewing(&self) -> bool {
        self.preview.borrow().is_some()
    }

    pub fn save_custom_theme(
        self: &Rc<Self>,
        tokens: ThemeTokens,
        done: impl FnOnce(io::Result<String>) + 'static,
    ) {
        let value = validate_tokens(&tokens)
            .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))
            .and_then(|()| toml::to_string_pretty(&tokens).map_err(io::Error::other));
        let value = match value {
            Ok(value) => value,
            Err(error) => return done(Err(error)),
        };
        let taken: Vec<String> = self
            .themes
            .borrow()
            .iter()
            .map(|theme| theme.id.clone())
            .collect();
        let name = tokens.name.clone();
        let saved = crate::storage::writer::run(move || {
            let directory = themes_directory();
            fs::create_dir_all(&directory)?;
            // Never replace a file already there, even one that did not load: it may be
            // a broken or newly linked dotfile the user still wants.
            let id = custom_theme_id(&name, |candidate| {
                taken.iter().any(|id| id == candidate)
                    || fs::symlink_metadata(directory.join(format!("{candidate}.toml"))).is_ok()
            });
            crate::storage::atomic_write_config(
                &directory.join(format!("{id}.toml")),
                value.as_bytes(),
            )?;
            Ok(id)
        });
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let saved = saved
                .await
                .unwrap_or_else(|_| Err(io::Error::from_raw_os_error(libc::ECANCELED)));
            let Some(manager) = weak.upgrade() else {
                return;
            };
            match saved {
                Ok(id) => {
                    // The directory may not have existed when the watch was last armed.
                    manager.monitor_custom_themes(true);
                    manager.themes.borrow_mut().push(Theme {
                        id: id.clone(),
                        tokens,
                        custom: true,
                    });
                    manager.select_theme(&id);
                    done(Ok(id));
                }
                Err(error) => done(Err(error)),
            }
        });
    }

    pub fn appearance_tokens(&self) -> ThemeTokens {
        if self.follows_omarchy()
            && let Some(tokens) = self
                .omarchy_tokens(self.preferences.omarchy_variant())
                .or_else(|| self.applied_omarchy_tokens.borrow().clone())
        {
            return tokens;
        }
        self.current_tokens().unwrap_or_else(azure_tokens)
    }

    pub fn starter_tokens(&self) -> ThemeTokens {
        self.current_tokens().unwrap_or_else(azure_tokens)
    }

    fn install_provider(&self) {
        self.reset_provider.load_from_string(RESET_CSS);
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &self.reset_provider,
                RESET_STYLE_PRIORITY,
            );
            gtk::style_context_add_provider_for_display(
                &display,
                &self.base_theme_provider,
                BASE_THEME_STYLE_PRIORITY,
            );
            gtk::style_context_add_provider_for_display(
                &display,
                &self.font_provider,
                FONT_STYLE_PRIORITY,
            );
            gtk::style_context_add_provider_for_display(
                &display,
                &self.base_provider,
                STYLE_PRIORITY,
            );
            gtk::style_context_add_provider_for_display(
                &display,
                &self.density_provider,
                DENSITY_STYLE_PRIORITY,
            );
            gtk::style_context_add_provider_for_display(
                &display,
                &self.provider,
                TOKEN_STYLE_PRIORITY,
            );
            gtk::style_context_add_provider_for_display(
                &display,
                &self.uncomposited_provider,
                UNCOMPOSITED_STYLE_PRIORITY,
            );
            gtk::style_context_add_provider_for_display(
                &display,
                &self.user_provider,
                USER_STYLE_PRIORITY,
            );
            Self::apply_composited_display(&display, &self.uncomposited_provider);
            let weak_provider = self.uncomposited_provider.downgrade();
            display.connect_notify_local(Some("composited"), move |display, _| {
                if let Some(provider) = weak_provider.upgrade() {
                    Self::apply_composited_display(display, &provider);
                }
            });
        }
        self.user_provider
            .connect_parsing_error(|_, section, error| {
                tracing::warn!(
                    %error,
                    path = %user_stylesheet_path().display(),
                    location = %section,
                    "invalid rule in user stylesheet"
                );
            });
    }

    fn apply_composited_display(display: &gdk::Display, provider: &gtk::CssProvider) {
        if display.is_composited() {
            provider.load_from_string("");
        } else {
            provider.load_from_string(UNCOMPOSITED_CSS);
        }
    }

    /// Reparses the stylesheet only when the roundness changed; the other
    /// appearance preferences are applied through the token provider.
    fn apply_base_stylesheet(&self) {
        let roundness = self.preferences.corner_roundness();
        if self.base_roundness.replace(Some(roundness)) == Some(roundness) {
            return;
        }
        self.base_provider
            .load_from_string(&roundness::scale_radii(STYLE_CSS, roundness.scale()));
    }

    #[cfg(test)]
    pub(crate) fn base_stylesheet(&self) -> String {
        self.base_provider.to_str().into()
    }

    /// Kept apart from the token provider so dragging a density slider restyles
    /// only spacing rather than reapplying the whole theme.
    fn apply_density(&self) {
        let density = (
            self.preferences.browser_density(),
            self.preferences.frame_density(),
        );
        if self.density.replace(Some(density)) == Some(density) {
            return;
        }
        let (browser, frame) = density;
        let mut css = match browser {
            super::browser_modes::BrowserDensity::Custom(px) => {
                super::browser_modes::browser_density_custom_css(px)
            }
            _ => String::new(),
        };
        css.push_str(&frame_density_css(frame.px()));
        self.density_provider.load_from_string(&css);
    }

    #[cfg(test)]
    pub(crate) fn density_stylesheet(&self) -> String {
        self.density_provider.to_str().into()
    }

    fn apply_selected(&self) {
        let preview = self.preview.borrow().clone();
        if let Some(tokens) = preview {
            self.apply_tokens(&tokens, None);
            return;
        }
        if self.follows_omarchy() {
            if let Some(tokens) = self.omarchy_tokens(self.preferences.omarchy_variant()) {
                self.applied_omarchy_tokens.replace(Some(tokens.clone()));
                let palette = self
                    .omarchy_state
                    .borrow()
                    .as_ref()
                    .and_then(OmarchyState::source_palette);
                self.apply_tokens(&tokens, palette.as_ref());
            }
            return;
        }
        self.applied_omarchy_tokens.take();
        if let Some(tokens) = self.current_tokens() {
            self.apply_tokens(&tokens, None);
        }
    }

    fn omarchy_tokens(&self, variant: OmarchyVariant) -> Option<ThemeTokens> {
        self.omarchy_state
            .borrow()
            .as_ref()
            .and_then(|state| state.tokens(variant))
    }

    pub(in crate::ui) fn current_tokens(&self) -> Option<ThemeTokens> {
        let id = self.effective_theme_id();
        self.themes
            .borrow()
            .iter()
            .find(|theme| theme.id == id)
            .map(|theme| theme.tokens.clone())
    }

    fn apply_tokens(&self, tokens: &ThemeTokens, source_palette: Option<&SourcePalette>) {
        let color = |value: &str| {
            let [red, green, blue] = parse_rgb_channels(value).unwrap_or_default();
            (u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)
        };
        self.active_model_palette
            .set(crate::services::ModelPalette {
                accent: color(&tokens.accent),
                surface: color(&tokens.surface),
            });
        super::document_media::apply_theme(tokens);
        super::browser::find::apply_theme(&tokens.accent, &tokens.background);
        super::preview::audio::apply_theme(tokens);
        let root_font_px = snapped_root_font_px(
            self.preferences.text_size().root_font_px(),
            desktop_text_scale_factor(),
        );
        let glow = if self.preferences.element_glow() {
            "@strata_accent"
        } else {
            "transparent"
        };
        self.provider.load_from_string(&format!(
            "{}\n@define-color strata_glow {glow};\n",
            tokens_css(tokens, root_font_px)
        ));
        let family = self.preferences.interface_font();
        self.font_provider
            .load_from_string(&font::stylesheet(family.as_deref()));
        apply_interface_font(family.as_deref(), root_font_px);
        crate::assets::set_interface_icon_scale(root_font_px / 13.0);
        let dark_background =
            parse_rgb_channels(&tokens.background).is_none_or(|[red, green, blue]| {
                u32::from(red) * 299 + u32::from(green) * 587 + u32::from(blue) * 114 < 128_000
            });
        if self.base_variant_dark.replace(Some(dark_background)) != Some(dark_background) {
            apply_base_variant(dark_background);
            self.base_theme_provider
                .load_from_string(&base_theme_import(dark_background));
        }
        crate::assets::set_catppuccin_flavor(crate::assets::CatppuccinFlavor::for_dark_background(
            dark_background,
        ));
        crate::assets::set_catppuccin_theme_colors(
            self.preferences.file_icon_colors() == FileIconColors::Theme,
        );
        // SVG strokes accept fewer color forms than GTK does.
        crate::assets::set_primary_icon_color(&color_to_hex(&tokens.accent));
        crate::assets::set_text_icon_color(&color_to_hex(&tokens.text));
        crate::assets::set_danger_icon_color(&color_to_hex(&tokens.danger));
        super::thumbnail::refresh_all_customized_icons();
        stage_source_style_scheme(tokens, source_palette);
        style_document_buffers(tokens);
        style_document_views(tokens);
        self.theme_listeners.notify(self);
    }

    fn on_preferences_changed(&self) {
        let appearance = AppearancePreferences::read(&self.preferences);
        if *self.appearance.borrow() != appearance {
            self.appearance.replace(appearance);
            self.apply_base_stylesheet();
            self.apply_selected();
        }
        self.apply_density();
        self.theme_listeners.notify(self);
    }
}

fn has_omarchy_theme(state: Option<&OmarchyState>) -> bool {
    state.is_some_and(|state| state.tokens(OmarchyVariant::Lighter).is_some())
}

fn default_danger() -> String {
    "#e5484d".to_owned()
}

/// The token's color fields, borrowed as `$tokens` is. Exhaustive destructuring
/// makes a new field fail to compile until it is classified.
macro_rules! color_fields {
    ($tokens:expr) => {{
        let ThemeTokens {
            name: _,
            background,
            surface,
            text,
            accent,
            danger,
            muted,
            highlight,
            border,
            dim_text,
            syntax_keyword,
            syntax_string,
            syntax_constant,
            syntax_type,
            syntax_preprocessor,
        } = $tokens;
        [
            background, surface, text, accent, danger, muted, highlight, border, dim_text,
        ]
        .into_iter()
        .chain(
            [
                syntax_keyword,
                syntax_string,
                syntax_constant,
                syntax_type,
                syntax_preprocessor,
            ]
            .into_iter()
            .flatten(),
        )
    }};
}

impl ThemeTokens {
    fn color_fields(&self) -> impl Iterator<Item = &String> {
        color_fields!(self)
    }

    fn color_fields_mut(&mut self) -> impl Iterator<Item = &mut String> {
        color_fields!(self)
    }
    pub(super) fn initialize_syntax_colors(&mut self) {
        let palette = resolved_source_palette(self, None);
        self.syntax_keyword = Some(palette.statement);
        self.syntax_string = Some(palette.string);
        self.syntax_constant = Some(palette.constant);
        self.syntax_type = Some(palette.type_color);
        self.syntax_preprocessor = Some(palette.preprocessor);
    }
}

#[cfg(test)]
mod tests;
