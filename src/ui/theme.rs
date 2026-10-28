// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    fs, io,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, gio, glib, prelude::*};
use serde::{Deserialize, Serialize};
use sourceview5::prelude::BufferExt as _;

use super::browser_modes::DensityScale;
use super::preferences::{
    CornerRoundness, FileIconColors, OmarchyVariant, PreferenceManager, TextSize,
    desktop_text_scale_factor, notify_live, snapped_root_font_px,
};
use crate::{adapters::Watched, model::Location, services::LoadHandle};

mod font;
mod omarchy;
mod roundness;

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
const TOKEN_STYLE_PRIORITY: u32 = DENSITY_STYLE_PRIORITY + 1;
/// Above the token and roundness styles so the uncomposited fallback wins over them.
const UNCOMPOSITED_STYLE_PRIORITY: u32 = TOKEN_STYLE_PRIORITY + 1;
/// Above every Strata stylesheet, so `~/.config/strata/style.css` has the final word.
const USER_STYLE_PRIORITY: u32 = UNCOMPOSITED_STYLE_PRIORITY + 1;
const USER_STYLE_FILE: &str = "style.css";
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
    static SOURCE_STYLE_PATH_INSTALLED: Cell<bool> = const { Cell::new(false) };
    static SOURCE_LANGUAGE_PATH_INSTALLED: Cell<bool> = const { Cell::new(false) };
    static SOURCE_BUFFERS: RefCell<Vec<glib::WeakRef<sourceview5::Buffer>>> = const { RefCell::new(Vec::new()) };
    static DOCUMENT_BUFFERS: RefCell<Vec<glib::WeakRef<gtk::TextBuffer>>> = const { RefCell::new(Vec::new()) };
    static DOCUMENT_VIEWS: RefCell<Vec<glib::WeakRef<super::document_view::DocumentTextView>>> = const { RefCell::new(Vec::new()) };
    /// Installed on the first source preview buffer, so startup performs no SourceView I/O.
    static PENDING_STYLE_TOKENS: RefCell<Option<(ThemeTokens, Option<SourcePalette>)>> = const { RefCell::new(None) };
    static STYLE_SCHEME_DIRTY: Cell<bool> = const { Cell::new(true) };
    static SOURCE_STYLE_RESOURCE: RefCell<Option<gio::Resource>> = const { RefCell::new(None) };
}

const SOURCE_STYLE_FOLDER: &str = "/io/github/lgse/Strata/generated/source-styles/";

const THEME_CATALOG: &str = include_str!("../../data/themes/catalog.toml");

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

#[derive(Deserialize)]
struct ThemeCatalog {
    themes: Vec<CatalogTheme>,
}

#[derive(Deserialize)]
struct CatalogTheme {
    id: String,
    #[serde(flatten)]
    tokens: ThemeTokens,
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

type ThemeRefreshPreference = dyn Fn(&gtk::Widget, &ThemeManager);

struct ThemePreferenceListener {
    active: Cell<bool>,
    anchor: glib::WeakRef<gtk::Widget>,
    refresh: Box<ThemeRefreshPreference>,
}

#[derive(Default)]
struct ThemeListeners {
    notifying: Cell<bool>,
    listeners: Rc<RefCell<Vec<Rc<ThemePreferenceListener>>>>,
}

impl ThemeListeners {
    fn bind<T: PartialEq + Clone + 'static>(
        &self,
        manager: &ThemeManager,
        anchor: &impl IsA<gtk::Widget>,
        read: impl Fn(&ThemeManager) -> T + 'static,
        apply: impl Fn(&gtk::Widget, T) + 'static,
    ) {
        let previous = RefCell::new(None);
        let listener = Rc::new(ThemePreferenceListener {
            active: Cell::new(true),
            anchor: anchor.as_ref().downgrade(),
            refresh: Box::new(move |widget, manager| {
                let value = read(manager);
                if previous.borrow().as_ref() == Some(&value) {
                    return;
                }
                previous.replace(Some(value.clone()));
                apply(widget, value);
            }),
        });
        self.listeners.borrow_mut().push(listener.clone());
        let weak_listeners = Rc::downgrade(&self.listeners);
        let weak_listener = Rc::downgrade(&listener);
        anchor.connect_destroy(move |_| {
            if let Some(listener) = weak_listener.upgrade() {
                listener.active.set(false);
            }
            if let Some(listeners) = weak_listeners.upgrade() {
                listeners.borrow_mut().retain(|candidate| {
                    !std::rc::Weak::ptr_eq(&Rc::downgrade(candidate), &weak_listener)
                });
            }
        });
        (listener.refresh)(anchor.as_ref(), manager);
    }

    fn notify(&self, manager: &ThemeManager) {
        if self.notifying.replace(true) {
            return;
        }
        notify_live(
            &self.listeners,
            |listener| listener.active.get() && listener.anchor.upgrade().is_some(),
            |listener| {
                if listener.active.get()
                    && let Some(anchor) = listener.anchor.upgrade()
                {
                    (listener.refresh)(&anchor, manager);
                }
            },
        );
        self.notifying.set(false);
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
        let existing = SHARED_MANAGER.with(|shared| shared.borrow().clone());
        if let Some(manager) = existing {
            return manager;
        }
        let manager = Self::load();
        SHARED_MANAGER.with(|shared| shared.replace(Some(manager.clone())));
        manager
    }

    fn load() -> Rc<Self> {
        let preferences = PreferenceManager::shared();
        let themes = merge_builtin_and_custom_themes(builtins(), load_custom_themes());
        let omarchy = OmarchyRefresh::read();
        if let Some(state) = &omarchy.state {
            preferences.set_active_omarchy_theme(&state.name);
        }
        let omarchy_available = omarchy
            .state
            .as_ref()
            .and_then(|state| state.tokens(OmarchyVariant::Lighter))
            .is_some();
        preferences.normalize_loaded_theme_selection(
            |id| themes.iter().any(|theme| theme.id == id),
            omarchy_available,
        );
        let appearance = AppearancePreferences {
            mode: preferences.theme_mode(),
            theme: preferences.selected_theme_id(),
            omarchy_variant: preferences.omarchy_variant(),
            text_size: preferences.text_size(),
            corner_roundness: preferences.corner_roundness(),
            interface_font: preferences.interface_font(),
            element_glow: preferences.element_glow(),
            file_icon_colors: preferences.file_icon_colors(),
        };

        let manager = Rc::new(Self {
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
            preferences: preferences.clone(),
            omarchy_available: Cell::new(omarchy_available),
            omarchy_state: RefCell::new(omarchy.state),
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
        });
        let weak = Rc::downgrade(&manager);
        preferences.observe(Rc::new(move || {
            if let Some(manager) = weak.upgrade() {
                manager.on_preferences_changed();
            }
        }));
        pin_manual_font_rendering();
        pin_base_theme();
        manager.install_provider();
        manager.apply_base_stylesheet();
        manager.apply_density();
        manager.apply_selected();
        manager.monitor_text_scaling();
        manager.monitor_omarchy(&omarchy.directories, omarchy.theme_directory.as_deref());
        manager.monitor_custom_themes(themes_directory().is_dir());
        manager.monitor_linked_themes(linked_theme_targets(&themes_directory()));
        manager
            .user_provider
            .load_from_string(&read_user_stylesheet());
        manager.monitor_user_stylesheet();
        manager
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
        self.theme_listeners.bind(self, anchor, read, apply);
    }

    pub fn select_theme(&self, id: &str) {
        if !self.themes.borrow().iter().any(|theme| theme.id == id) {
            return;
        }
        self.preview.take();
        let changed = self.follows_omarchy() || self.selected_id() != id;
        self.preferences.set_theme_selection("theme", Some(id));
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
        self.starter_tokens()
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

    /// A missing or unreadable file clears the provider, so deleting
    /// `style.css` restores Strata's own appearance without a restart.
    fn refresh_user_stylesheet(self: &Rc<Self>) {
        let read = crate::storage::writer::run(read_user_stylesheet);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            if let (Ok(css), Some(manager)) = (read.await, weak.upgrade()) {
                manager.user_provider.load_from_string(&css);
            }
        });
    }

    fn monitor_user_stylesheet(self: &Rc<Self>) {
        let (changed, live) = (Rc::downgrade(self), Rc::downgrade(self));
        let monitor = crate::adapters::watch(
            &Location::local(user_stylesheet_path()),
            Watched::File,
            move |_, _| {
                if let Some(manager) = changed.upgrade() {
                    manager.schedule_user_style_refresh();
                }
            },
            move || {
                if let Some(manager) = live.upgrade() {
                    manager.schedule_user_style_refresh();
                }
            },
        );
        self.user_style_monitor.replace(Some(monitor));
    }

    fn schedule_user_style_refresh(self: &Rc<Self>) {
        if let Some(pending) = self.pending_user_style_refresh.borrow_mut().take() {
            pending.remove();
        }
        let weak = Rc::downgrade(self);
        let refresh = glib::timeout_add_local_once(Duration::from_millis(75), move || {
            let Some(manager) = weak.upgrade() else {
                return;
            };
            manager.pending_user_style_refresh.borrow_mut().take();
            manager.refresh_user_stylesheet();
        });
        self.pending_user_style_refresh.replace(Some(refresh));
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
        let id = self.preferences.selected_theme_id();
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
        let appearance = AppearancePreferences {
            mode: self.preferences.theme_mode(),
            theme: self.preferences.selected_theme_id(),
            omarchy_variant: self.preferences.omarchy_variant(),
            text_size: self.preferences.text_size(),
            corner_roundness: self.preferences.corner_roundness(),
            interface_font: self.preferences.interface_font(),
            element_glow: self.preferences.element_glow(),
            file_icon_colors: self.preferences.file_icon_colors(),
        };
        if *self.appearance.borrow() != appearance {
            self.appearance.replace(appearance);
            self.apply_base_stylesheet();
            self.apply_selected();
        }
        self.apply_density();
        self.theme_listeners.notify(self);
    }

    fn monitor_text_scaling(self: &Rc<Self>) {
        let Some(settings) = gtk::Settings::default() else {
            return;
        };
        let weak = Rc::downgrade(self);
        settings.connect_gtk_xft_dpi_notify(move |_| {
            let Some(manager) = weak.upgrade() else {
                return;
            };
            manager.apply_selected();
        });
    }

    fn monitor_omarchy(self: &Rc<Self>, directories: &[PathBuf], theme_directory: Option<&Path>) {
        let mut monitors = self.omarchy_monitors.take();
        monitors.retain(|(path, _)| directories.contains(path));
        for path in directories {
            if monitors.iter().any(|(watched, _)| watched == path) {
                continue;
            }
            let (changed, live) = (Rc::downgrade(self), Rc::downgrade(self));
            let colors =
                (Some(path.as_path()) == theme_directory).then(|| path.join("colors.toml"));
            let monitor = crate::adapters::watch(
                &Location::local(path),
                Watched::Folder,
                move |file, other_file| {
                    let relevant = |file: &gio::File| {
                        is_omarchy_theme_event(file)
                            || colors
                                .as_ref()
                                .is_some_and(|colors| file.path().as_ref() == Some(colors))
                    };
                    if !relevant(file) && !other_file.is_some_and(relevant) {
                        return;
                    }
                    if let Some(manager) = changed.upgrade() {
                        manager.schedule_omarchy_refresh();
                    }
                },
                move || {
                    if let Some(manager) = live.upgrade() {
                        manager.schedule_omarchy_refresh();
                    }
                },
            );
            monitors.push((path.clone(), monitor));
        }
        self.omarchy_monitors.replace(monitors);
    }

    fn schedule_omarchy_refresh(self: &Rc<Self>) {
        if let Some(pending) = self.pending_omarchy_refresh.borrow_mut().take() {
            pending.remove();
        }
        let weak = Rc::downgrade(self);
        let refresh = glib::timeout_add_local_once(Duration::from_millis(75), move || {
            let Some(manager) = weak.upgrade() else {
                return;
            };
            manager.pending_omarchy_refresh.borrow_mut().take();
            manager.refresh_omarchy_state();
        });
        self.pending_omarchy_refresh.replace(Some(refresh));
    }

    pub(in crate::ui) fn refresh_omarchy_state(self: &Rc<Self>) {
        let read = crate::storage::writer::run(OmarchyRefresh::read);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            if let (Ok(refresh), Some(manager)) = (read.await, weak.upgrade()) {
                manager.apply_omarchy_refresh(refresh);
            }
        });
    }

    fn apply_omarchy_refresh(self: &Rc<Self>, refresh: OmarchyRefresh) {
        self.monitor_omarchy(&refresh.directories, refresh.theme_directory.as_deref());
        let unchanged = self.omarchy_state.replace(refresh.state.clone()) == refresh.state;
        if let Some(state) = &refresh.state {
            self.preferences.set_active_omarchy_theme(&state.name);
            self.appearance.borrow_mut().omarchy_variant = self.preferences.omarchy_variant();
        }
        let available = self.omarchy_tokens(OmarchyVariant::Lighter).is_some()
            || (self.is_omarchy_available() && refresh.name_exists);
        let availability_changed = self.omarchy_available.replace(available) != available;
        if !available && self.follows_omarchy() {
            self.preview.take();
            self.preferences.set_theme_selection("theme", None);
            return;
        }
        if availability_changed {
            self.preferences.notify_changes();
            return;
        }
        if !unchanged {
            if self.follows_omarchy() {
                self.apply_selected();
            }
            // Republishes the variant remembered for a newly active theme.
            self.preferences.notify_changes();
        }
    }

    fn monitor_custom_themes(self: &Rc<Self>, directory_exists: bool) {
        if !directory_exists || self.themes_monitor.borrow().is_some() {
            return;
        }
        let (changed, live) = (Rc::downgrade(self), Rc::downgrade(self));
        let monitor = crate::adapters::watch(
            &Location::local(themes_directory()),
            Watched::Folder,
            move |file, other_file| {
                if !is_custom_theme_event(file) && !other_file.is_some_and(is_custom_theme_event) {
                    return;
                }
                if let Some(manager) = changed.upgrade() {
                    manager.schedule_themes_refresh();
                }
            },
            move || {
                if let Some(manager) = live.upgrade() {
                    manager.schedule_themes_refresh();
                }
            },
        );
        self.themes_monitor.replace(Some(monitor));
    }

    /// Writes through a dotfiles symlink only reach the target's directory.
    fn monitor_linked_themes(self: &Rc<Self>, targets: Vec<PathBuf>) {
        let mut directories: Vec<PathBuf> = Vec::new();
        for parent in targets.iter().filter_map(|target| target.parent()) {
            if !directories.iter().any(|path| path == parent) {
                directories.push(parent.to_path_buf());
            }
        }
        self.linked_theme_targets.replace(targets);
        let mut monitors = self.linked_theme_monitors.take();
        monitors.retain(|(path, _)| directories.contains(path));
        for path in directories {
            if monitors.iter().any(|(watched, _)| *watched == path) {
                continue;
            }
            let (changed, live) = (Rc::downgrade(self), Rc::downgrade(self));
            let monitor = crate::adapters::watch(
                &Location::local(&path),
                Watched::Folder,
                move |file, other_file| {
                    let Some(manager) = changed.upgrade() else {
                        return;
                    };
                    let linked = |file: &gio::File| {
                        file.path().is_some_and(|path| {
                            manager.linked_theme_targets.borrow().contains(&path)
                        })
                    };
                    if linked(file) || other_file.is_some_and(linked) {
                        manager.schedule_themes_refresh();
                    }
                },
                // Catches a write made before the watch went live.
                move || {
                    if let Some(manager) = live.upgrade() {
                        manager.schedule_themes_refresh();
                    }
                },
            );
            monitors.push((path, monitor));
        }
        self.linked_theme_monitors.replace(monitors);
    }

    fn schedule_themes_refresh(self: &Rc<Self>) {
        if let Some(pending) = self.pending_themes_refresh.borrow_mut().take() {
            pending.remove();
        }
        let weak = Rc::downgrade(self);
        let refresh = glib::timeout_add_local_once(Duration::from_millis(75), move || {
            let Some(manager) = weak.upgrade() else {
                return;
            };
            manager.pending_themes_refresh.borrow_mut().take();
            manager.refresh_custom_themes();
        });
        self.pending_themes_refresh.replace(Some(refresh));
    }

    pub(in crate::ui) fn refresh_custom_themes(self: &Rc<Self>) {
        let loaded = crate::storage::writer::run(|| {
            (
                load_custom_themes(),
                linked_theme_targets(&themes_directory()),
            )
        });
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            if let (Ok((custom, targets)), Some(manager)) = (loaded.await, weak.upgrade()) {
                // Links may have been added, removed, or repointed.
                manager.monitor_linked_themes(targets);
                manager.adopt_custom_themes(custom);
            }
        });
    }

    /// A selected file caught mid-write, invalid, or removed keeps its last
    /// valid colors until a read succeeds again.
    fn adopt_custom_themes(&self, custom: Vec<Theme>) {
        let selected = self.preferences.selected_theme_id();
        let before = self.current_tokens();
        let mut themes = merge_builtin_and_custom_themes(builtins(), custom);
        if !themes.iter().any(|theme| theme.id == selected)
            && let Some(previous) = self
                .themes
                .borrow()
                .iter()
                .find(|theme| theme.id == selected)
                .cloned()
        {
            themes.push(previous);
        }
        self.themes.replace(themes);
        let after = self.current_tokens();
        if self.follows_omarchy() || after.is_none() || after == before {
            return;
        }
        self.apply_selected();
        self.preferences.notify_changes();
    }
}

fn is_custom_theme_event(file: &gio::File) -> bool {
    file.path().is_some_and(|path| {
        path.parent() == Some(themes_directory().as_path())
            && path
                .extension()
                .is_some_and(|extension| extension == "toml")
    })
}

fn linked_theme_targets(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "toml")
                && entry
                    .file_type()
                    .is_ok_and(|file_type| file_type.is_symlink())
        })
        .filter_map(|entry| resolve_theme_link(&entry.path()))
        .collect()
}

/// A missing target still resolves, so its directory is watched until the file appears.
fn resolve_theme_link(link: &Path) -> Option<PathBuf> {
    fs::canonicalize(link).ok().or_else(|| {
        let target = link.parent()?.join(fs::read_link(link).ok()?);
        Some(
            fs::canonicalize(target.parent()?)
                .ok()?
                .join(target.file_name()?),
        )
    })
}

fn is_omarchy_theme_event(file: &gio::File) -> bool {
    file.path().is_some_and(|path| {
        let state = omarchy_state_dir();
        state.starts_with(&path)
            || path == state.join("theme")
            || path == state.join("theme.name")
            || path == state.join("theme/colors.toml")
    })
}

fn builtins() -> Vec<Theme> {
    let mut themes: Vec<_> = toml::from_str::<ThemeCatalog>(THEME_CATALOG)
        .map(|catalog| {
            catalog
                .themes
                .into_iter()
                .map(|theme| Theme {
                    id: theme.id,
                    tokens: theme.tokens,
                    custom: false,
                })
                .collect()
        })
        .unwrap_or_default();
    for theme in &mut themes {
        // Dim text carries the same contrast floors as body text, and the lift
        // stops at the first passing step, so it stays dimmer than text.
        let tokens = &mut theme.tokens;
        let dim = omarchy::readable(&tokens.dim_text, &[&tokens.background], &tokens.text, 4.5);
        tokens.dim_text = omarchy::readable(&dim, &[&tokens.surface], &tokens.text, 3.0);
    }
    themes.sort_by_key(|theme| theme.tokens.name.to_lowercase());
    themes
}

fn merge_builtin_and_custom_themes(mut builtins: Vec<Theme>, custom: Vec<Theme>) -> Vec<Theme> {
    builtins.retain(|builtin| !custom.iter().any(|theme| theme.id == builtin.id));
    builtins.extend(custom);
    builtins
}

fn azure_tokens() -> ThemeTokens {
    builtins()
        .into_iter()
        .find(|theme| theme.id == "azure-glow")
        .map(|theme| theme.tokens)
        .unwrap_or_else(|| ThemeTokens {
            name: "Azure Glow".to_owned(),
            background: "#0c1a2b".to_owned(),
            surface: "#122438".to_owned(),
            text: "#c9deed".to_owned(),
            accent: "#4fd6ff".to_owned(),
            danger: default_danger(),
            muted: "#1e3a52".to_owned(),
            highlight: "#244d68".to_owned(),
            border: "#315b75".to_owned(),
            dim_text: "#6f8da3".to_owned(),
            syntax_keyword: None,
            syntax_string: None,
            syntax_constant: None,
            syntax_type: None,
            syntax_preprocessor: None,
        })
}

fn load_custom_themes() -> Vec<Theme> {
    let Ok(entries) = fs::read_dir(themes_directory()) else {
        return Vec::new();
    };
    let mut themes: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "toml")
        })
        .filter_map(|entry| {
            let id = entry.path().file_stem()?.to_string_lossy().into_owned();
            let source = fs::read_to_string(entry.path()).ok()?;
            let mut tokens: ThemeTokens = toml::from_str(&source).ok()?;
            validate_tokens(&tokens).ok()?;
            // Canonical in memory only; the user's file stays as written.
            for color in tokens.color_fields_mut() {
                *color = canonical_color(color)?;
            }
            Some(Theme {
                id,
                tokens,
                custom: true,
            })
        })
        .collect();
    themes.sort_by(|left, right| left.tokens.name.cmp(&right.tokens.name));
    themes
}

#[derive(Clone, PartialEq)]
struct OmarchyState {
    name: String,
    colors: String,
}

impl OmarchyState {
    fn tokens(&self, variant: OmarchyVariant) -> Option<ThemeTokens> {
        tokens_from_quattro(self.name.trim(), &self.colors, variant)
    }

    fn source_palette(&self) -> Option<SourcePalette> {
        source_palette_from_quattro(&self.colors)
    }
}

struct OmarchyRefresh {
    state: Option<OmarchyState>,
    name_exists: bool,
    theme_directory: Option<PathBuf>,
    /// Ancestor watches survive moving away or replacing the current state tree.
    directories: Vec<PathBuf>,
}

impl OmarchyRefresh {
    fn read() -> Self {
        let state = omarchy_state_dir();
        let home = glib::home_dir();
        let theme = state.join("theme");
        // Watch the target's identity, not the stable symlink spelling.
        let theme_directory = fs::canonicalize(&theme).ok();
        let directories = theme_directory
            .iter()
            .map(PathBuf::as_path)
            .chain(state.ancestors().take_while(|path| path.starts_with(&home)))
            .filter(|path| path.is_dir())
            .map(Path::to_path_buf)
            .collect();
        let read = |name: &str| fs::read_to_string(state.join(name)).ok();
        Self {
            state: read("theme.name")
                .zip(read("theme/colors.toml"))
                .map(|(name, colors)| OmarchyState { name, colors }),
            name_exists: state.join("theme.name").is_file(),
            theme_directory,
            directories,
        }
    }
}

fn tokens_from_quattro(name: &str, source: &str, variant: OmarchyVariant) -> Option<ThemeTokens> {
    let values: toml::Value = toml::from_str(source).ok()?;
    let get = |key: &str| quattro_color(&values, key);
    let source_background = get("background")?;
    let text = get("foreground")?;
    let accent = get("accent")?;
    let selection = get("selection").unwrap_or_else(|| blend(&source_background, &accent, 0.25));
    // Some themes set lighter_background equal to background, which would hide hover.
    let lighter = get("lighter_background")
        .filter(|lighter| *lighter != source_background)
        .unwrap_or_else(|| blend(&source_background, &text, 0.10));
    let mut tokens = ThemeTokens {
        name: title_case_slug(name),
        background: source_background.clone(),
        surface: get("dark_background").unwrap_or_else(|| source_background.clone()),
        muted: lighter,
        highlight: selection,
        border: get("muted").unwrap_or_else(|| blend(&source_background, &text, 0.36)),
        dim_text: blend(&source_background, &text, 0.62),
        text,
        accent,
        danger: get("color1").unwrap_or_else(default_danger),
        syntax_keyword: get("magenta").or_else(|| get("color5")),
        syntax_string: get("green").or_else(|| get("color2")),
        syntax_constant: get("orange").or_else(|| get("color9")),
        syntax_type: get("cyan").or_else(|| get("color3")),
        syntax_preprocessor: get("yellow"),
    };
    let light = omarchy::is_light(
        &source_background,
        values.get("mode").and_then(toml::Value::as_str),
    );
    let palette = source_palette_from_quattro(source);
    omarchy::apply_variant(&mut tokens, light, variant, palette.as_ref());
    Some(tokens)
}

fn source_palette_from_quattro(source: &str) -> Option<SourcePalette> {
    let values: toml::Value = toml::from_str(source).ok()?;
    let get = |key: &str| quattro_color(&values, key);
    Some(SourcePalette {
        statement: get("magenta").or_else(|| get("blue"))?,
        string: get("green")?,
        constant: get("orange").or_else(|| get("yellow"))?,
        type_color: get("cyan").or_else(|| get("blue"))?,
        preprocessor: get("yellow")?,
    })
}

/// Quattro values reach GTK CSS, GtkSourceView XML and SVG strokes, so a value GTK
/// cannot parse is treated as missing; callers decide whether that is fatal.
fn quattro_color(values: &toml::Value, key: &str) -> Option<String> {
    canonical_color(values.get(key)?.as_str()?)
}

fn default_danger() -> String {
    "#e5484d".to_owned()
}

fn validate_tokens(tokens: &ThemeTokens) -> Result<(), &'static str> {
    if tokens.name.trim().is_empty() {
        return Err("Enter a theme name");
    }
    if tokens
        .color_fields()
        .any(|color| gdk::RGBA::parse(color).is_err())
    {
        return Err("Every color must be a valid CSS color");
    }
    Ok(())
}

fn source_style_manager() -> sourceview5::StyleSchemeManager {
    let manager = sourceview5::StyleSchemeManager::default();
    SOURCE_STYLE_PATH_INSTALLED.with(|installed| {
        if !installed.replace(true) {
            manager.set_search_path(&[format!("resource://{SOURCE_STYLE_FOLDER}").as_str()]);
        }
    });
    manager
}

pub(super) fn source_languages() -> sourceview5::LanguageManager {
    let manager = sourceview5::LanguageManager::default();
    SOURCE_LANGUAGE_PATH_INSTALLED.with(|installed| {
        if !installed.replace(true) {
            let user = [glib::user_data_dir(), glib::home_dir()];
            let system: Vec<_> = manager
                .search_path()
                .into_iter()
                .filter(|folder| {
                    !user.iter().any(|root| {
                        root != Path::new("/") && Path::new(folder.as_str()).starts_with(root)
                    })
                })
                .collect();
            manager.set_search_path(&system.iter().map(glib::GString::as_str).collect::<Vec<_>>());
        }
    });
    manager
}

pub(super) fn new_source_buffer() -> sourceview5::Buffer {
    let manager = source_style_manager();
    ensure_source_style_scheme_installed();
    let buffer = sourceview5::Buffer::new(None);
    buffer.set_style_scheme(manager.scheme("strata-current").as_ref());
    SOURCE_BUFFERS.with(|buffers| {
        let mut buffers = buffers.borrow_mut();
        buffers.retain(|buffer| buffer.upgrade().is_some());
        let weak = glib::WeakRef::new();
        weak.set(Some(&buffer));
        buffers.push(weak);
    });
    buffer
}

pub(super) fn register_document_buffer(buffer: &gtk::TextBuffer) {
    let tokens = ThemeManager::shared().appearance_tokens();
    style_document_buffer(buffer, &tokens);
    DOCUMENT_BUFFERS.with(|buffers| {
        let mut buffers = buffers.borrow_mut();
        buffers.retain(|buffer| buffer.upgrade().is_some());
        let weak = glib::WeakRef::new();
        weak.set(Some(buffer));
        buffers.push(weak);
    });
}

pub(super) fn register_document_view(view: &super::document_view::DocumentTextView) {
    let tokens = ThemeManager::shared().appearance_tokens();
    style_document_view(view, &tokens);
    DOCUMENT_VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        views.retain(|view| view.upgrade().is_some());
        let weak = glib::WeakRef::new();
        weak.set(Some(view));
        views.push(weak);
    });
}

fn style_document_buffers(tokens: &ThemeTokens) {
    DOCUMENT_BUFFERS.with(|buffers| {
        buffers.borrow_mut().retain(|buffer| {
            let Some(buffer) = buffer.upgrade() else {
                return false;
            };
            style_document_buffer(&buffer, tokens);
            true
        });
    });
}

fn style_document_views(tokens: &ThemeTokens) {
    DOCUMENT_VIEWS.with(|views| {
        views.borrow_mut().retain(|view| {
            let Some(view) = view.upgrade() else {
                return false;
            };
            style_document_view(&view, tokens);
            true
        });
    });
}

fn style_document_view(view: &super::document_view::DocumentTextView, tokens: &ThemeTokens) {
    if let (Ok(fill), Ok(border), Ok(selection)) = (
        gdk::RGBA::parse(blend(&tokens.surface, &tokens.muted, 0.54)),
        gdk::RGBA::parse(blend(&tokens.surface, &tokens.border, 0.5)),
        gdk::RGBA::parse(&tokens.accent),
    ) {
        view.set_colors(fill, border, selection);
    }
}

fn style_document_buffer(buffer: &gtk::TextBuffer, tokens: &ThemeTokens) {
    let parse = |value: &str| gdk::RGBA::parse(value).ok();
    let table = buffer.tag_table();
    if let Some(color) = parse(&tokens.accent) {
        for name in ["document-accent", "document-link"] {
            if let Some(tag) = table.lookup(name) {
                tag.set_foreground_rgba(Some(&color));
            }
        }
    }
    if let Some(color) = parse(&tokens.dim_text)
        && let Some(tag) = table.lookup("document-dim")
    {
        tag.set_foreground_rgba(Some(&color));
    }
    if let Some(color) = parse(&tokens.text)
        && let Some(tag) = table.lookup("document-link-hover")
    {
        tag.set_foreground_rgba(Some(&color));
    }
    if let Some(color) = parse(&tokens.background)
        && let Some(tag) = table.lookup("document-selection")
    {
        tag.set_foreground_rgba(Some(&color));
    }
    if let Some(mut color) = parse(&tokens.highlight) {
        color.set_alpha(0.36);
        if let Some(tag) = table.lookup("document-quote") {
            tag.set_paragraph_background_rgba(Some(&color));
        }
        color.set_alpha(0.5);
        if let Some(tag) = table.lookup("document-link-hover") {
            tag.set_background_rgba(Some(&color));
        }
    }
}

fn stage_source_style_scheme(tokens: &ThemeTokens, source_palette: Option<&SourcePalette>) {
    PENDING_STYLE_TOKENS
        .with(|pending| pending.replace(Some((tokens.clone(), source_palette.cloned()))));
    STYLE_SCHEME_DIRTY.with(|dirty| dirty.set(true));
    let live = SOURCE_BUFFERS.with(|buffers| {
        buffers
            .borrow_mut()
            .retain(|buffer| buffer.upgrade().is_some());
        !buffers.borrow().is_empty()
    });
    if live {
        ensure_source_style_scheme_installed();
    }
}

/// Installs the staged scheme from memory and rescans the style manager, once per staged token set.
fn ensure_source_style_scheme_installed() {
    if !STYLE_SCHEME_DIRTY.with(Cell::get) {
        return;
    }
    let pending = PENDING_STYLE_TOKENS.with(|pending| pending.borrow().clone());
    let Some((tokens, palette)) = pending else {
        return;
    };
    let value = source_style_scheme_xml(&tokens, palette.as_ref());
    match crate::util::gresource::with_file(
        SOURCE_STYLE_FOLDER,
        "strata-current.xml",
        value.as_bytes(),
    ) {
        Ok(resource) => {
            STYLE_SCHEME_DIRTY.with(|dirty| dirty.set(false));
            install_source_style_scheme(resource);
        }
        Err(error) => tracing::warn!(%error, "unable to build preview syntax style"),
    }
}

fn install_source_style_scheme(resource: gio::Resource) {
    gio::resources_register(&resource);
    if let Some(previous) =
        SOURCE_STYLE_RESOURCE.with(|installed| installed.replace(Some(resource)))
    {
        gio::resources_unregister(&previous);
    }
    let manager = source_style_manager();
    manager.force_rescan();
    let scheme = manager.scheme("strata-current");
    SOURCE_BUFFERS.with(|buffers| {
        buffers.borrow_mut().retain(|buffer| {
            let Some(buffer) = buffer.upgrade() else {
                return false;
            };
            buffer.set_style_scheme(scheme.as_ref());
            true
        });
    });
}

fn resolved_source_palette(tokens: &ThemeTokens, palette: Option<&SourcePalette>) -> SourcePalette {
    let fallback = SourcePalette {
        statement: tokens.accent.clone(),
        string: blend(&tokens.accent, &tokens.text, 0.48),
        constant: blend(&tokens.accent, &tokens.text, 0.18),
        type_color: blend(&tokens.accent, &tokens.text, 0.24),
        preprocessor: blend(&tokens.accent, &tokens.text, 0.32),
    };
    let palette = palette.unwrap_or(&fallback);
    let resolve = |token: &Option<String>, fallback: &str| {
        token
            .as_deref()
            .filter(|value| gdk::RGBA::parse(*value).is_ok())
            .unwrap_or(fallback)
            .to_owned()
    };
    SourcePalette {
        statement: resolve(&tokens.syntax_keyword, &palette.statement),
        string: resolve(&tokens.syntax_string, &palette.string),
        constant: resolve(&tokens.syntax_constant, &palette.constant),
        type_color: resolve(&tokens.syntax_type, &palette.type_color),
        preprocessor: resolve(&tokens.syntax_preprocessor, &palette.preprocessor),
    }
}

impl ThemeTokens {
    // Exhaustive destructuring makes a new field fail to compile until it is classified.
    fn color_fields(&self) -> impl Iterator<Item = &String> {
        let Self {
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
        } = self;
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
    }

    fn color_fields_mut(&mut self) -> impl Iterator<Item = &mut String> {
        let Self {
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
        } = self;
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

fn source_style_scheme_xml(tokens: &ThemeTokens, palette: Option<&SourcePalette>) -> String {
    let palette = resolved_source_palette(tokens, palette);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<style-scheme id="strata-current" _name="Strata Current Theme" version="1.0">
  <color name="background" value="{}"/>
  <color name="surface" value="{}"/>
  <color name="text" value="{}"/>
  <color name="accent" value="{}"/>
  <color name="danger" value="{}"/>
  <color name="selection" value="{}"/>
  <color name="dim" value="{}"/>
  <color name="string" value="{}"/>
  <color name="constant" value="{}"/>
  <color name="type" value="{}"/>
  <color name="statement" value="{}"/>
  <color name="preprocessor" value="{}"/>
  <style name="text" foreground="text" background="surface"/>
  <style name="selection" foreground="background" background="accent"/>
  <style name="cursor" foreground="accent"/>
  <style name="current-line" background="background"/>
  <style name="line-numbers" foreground="dim" background="background"/>
  <style name="def:comment" foreground="dim" italic="true"/>
  <style name="def:shebang" foreground="dim" bold="true"/>
  <style name="def:string" foreground="string"/>
  <style name="def:constant" foreground="constant"/>
  <style name="def:special-char" foreground="constant"/>
  <style name="def:identifier" foreground="text"/>
  <style name="def:statement" foreground="statement" bold="true"/>
  <style name="def:type" foreground="type" bold="true"/>
  <style name="def:preprocessor" foreground="preprocessor"/>
  <style name="def:heading" foreground="accent" bold="true"/>
  <style name="def:link-destination" foreground="string" underline="single"/>
  <style name="def:error" foreground="background" background="danger" bold="true"/>
</style-scheme>
"#,
        color_to_hex(&tokens.background),
        color_to_hex(&tokens.surface),
        color_to_hex(&tokens.text),
        color_to_hex(&tokens.accent),
        color_to_hex(&tokens.danger),
        color_to_hex(&tokens.highlight),
        color_to_hex(&tokens.dim_text),
        color_to_hex(&palette.string),
        color_to_hex(&palette.constant),
        color_to_hex(&palette.type_color),
        color_to_hex(&palette.statement),
        color_to_hex(&palette.preprocessor),
    )
}

/// GTK's automatic font rendering hints glyph outlines but not their metrics,
/// so GPU glyph atlas slots are sized from unhinted extents and lose the top
/// pixel row at some sizes (GNOME/gtk#8395). Manual rendering hints metrics
/// too and follows the desktop's hinting settings. Application values outrank
/// the settings portal, so this holds for the whole process.
fn pin_manual_font_rendering() {
    if let Some(settings) = gtk::Settings::default() {
        set_enum_setting(&settings, "gtk-font-rendering", "manual");
    }
}

/// An empty theme bundled in Strata's resources. GTK's own theme provider sits
/// below the reset, so loading a real theme there only adds selector matching.
/// Desktop selections arrive through the settings portal or `settings.ini`,
/// which application values outrank for the whole process.
const PINNED_GTK_THEME: &str = "StrataEmpty";

/// GTK's built-in theme, restated above the reset.
const BASE_THEME: &str = "Default";

/// Keeps GTK's own theme empty and its contrast regular whatever the desktop
/// configures, so only Strata's stylesheets shape its widgets. Values are set
/// even when they already match: only application-sourced values survive later
/// portal updates. `GTK_THEME` is cleared before GTK starts because it bypasses
/// the setting.
fn pin_base_theme() {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    settings.set_gtk_theme_name(Some(PINNED_GTK_THEME));
    set_enum_setting(&settings, "gtk-interface-contrast", "no-preference");
}

/// The built-in theme's light or dark variant follows the active Strata theme
/// rather than the desktop color scheme.
fn apply_base_variant(dark: bool) {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    let scheme = if dark { "dark" } else { "light" };
    if settings
        .find_property("gtk-interface-color-scheme")
        .is_some()
    {
        set_enum_setting(&settings, "gtk-interface-color-scheme", scheme);
    } else {
        settings.set_gtk_application_prefer_dark_theme(dark);
    }
}

fn base_theme_import(dark: bool) -> String {
    let variant = if dark { "gtk-dark.css" } else { "gtk.css" };
    format!("@import url(\"resource:///org/gtk/libgtk/theme/{BASE_THEME}/{variant}\");")
}

/// Skips properties this GTK lacks.
fn set_enum_setting(settings: &gtk::Settings, property: &str, nick: &str) {
    let Some(spec) = settings.find_property(property) else {
        return;
    };
    if let Some(value) =
        glib::EnumClass::with_type(spec.value_type()).and_then(|class| class.to_value_by_nick(nick))
    {
        settings.set_property(property, value);
    }
}

fn apply_interface_font(family: Option<&str>, root_font_px: f64) {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_font_name(Some(&font::font_name(family, root_font_px)));
    }
}

/// One inset for every bar so the header, tab strip, sidebar rows, and Miller and
/// preview headers grow together and stay aligned. Padding and `margin-left` only,
/// so `sizing`'s `min-height` and fixed button widths still hold. The sidebar toggle
/// keeps its static margin, which lines it up with the sidebar icons below it.
fn frame_density_css(inset: u8) -> String {
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

fn tokens_css(tokens: &ThemeTokens, root_font_px: f64) -> String {
    let scale = root_font_px / 13.0;
    let header = (40.0 * scale).round();
    // Column headers add six pixels of padding and three extra border pixels.
    let column_header = header - 9.0;
    let control = (24.0 * scale).round();
    // Sidebar icons are centred in control-sized boxes; headings start where the icons do.
    let sidebar_heading_inset = (control - (17.0 * scale).round()) / 2.0;
    let browser_header = (control + 8.0).max(36.0);
    // Miller headers keep their three-pixel indicator, bottom border and balancing inset.
    let browser_column_header = browser_header - 6.0;
    let sizing = format!(
        "headerbar, headerbar > windowhandle > box, .mode-pane-header, .preview-header {{ min-height: {header}px; }}\n.column-header {{ min-height: {column_header}px; }}\nheaderbar .sidebar-toggle, button.header-action, headerbar menubutton.header-action > button, .preview-header-action, button.column-header-action, menubutton.column-header-action > button {{ min-width: {control}px; min-height: {control}px; }}\n.file-operation-card button.progress-card-action {{ min-width: 0; min-height: 0; }}\n.tab-strip, headerbar.browser-header, headerbar.browser-header > windowhandle > box, .mode-pane-header, .preview-header {{ min-height: {browser_header}px; }}\n.directory-column .column-header {{ min-height: {browser_column_header}px; }}\n.mode-pane-header .list-navigation-button, .sidebar-row > box > image, .sidebar-device-action {{ min-width: {control}px; min-height: {control}px; }}\n.sidebar-heading {{ padding-left: {sidebar_heading_inset}px; }}\n"
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

/// Parses colours GTK accepts (`#rgb`, `#rrggbb`, `rgb(...)`, names) into 8-bit
/// channels. Strata emits these channels as `#rrggbb` in GtkSourceView schemes.
pub(crate) fn parse_rgb_channels(value: &str) -> Option<[u8; 3]> {
    parse_rgba_channels(value).map(|[red, green, blue, _]| [red, green, blue])
}

fn parse_rgba_channels(value: &str) -> Option<[u8; 4]> {
    let color = gdk::RGBA::parse(value).ok()?;
    let channel = |component: f32| (f64::from(component).clamp(0.0, 1.0) * 255.0).round() as u8;
    Some([
        channel(color.red()),
        channel(color.green()),
        channel(color.blue()),
        channel(color.alpha()),
    ])
}

/// Rewrites a color GTK can parse as `#rrggbb`, or `#rrggbbaa` when translucent.
/// `gdk::RGBA::parse` also accepts 9-, 12- and 16-digit hex, which GTK CSS rejects.
fn canonical_color(value: &str) -> Option<String> {
    let [red, green, blue, alpha] = parse_rgba_channels(value)?;
    let rgb = hex_from_channels([red, green, blue]);
    Some(if alpha == u8::MAX {
        rgb
    } else {
        format!("{rgb}{alpha:02x}")
    })
}

fn hex_from_channels(channels: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", channels[0], channels[1], channels[2])
}

/// Canonicalizes a colour token to Strata's `#rrggbb` scheme representation.
pub(crate) fn color_to_hex(value: &str) -> String {
    parse_rgb_channels(value)
        .map(hex_from_channels)
        .unwrap_or_else(|| value.to_owned())
}

fn blend(left: &str, right: &str, amount: f64) -> String {
    let (Some(left), Some(right)) = (parse_rgb_channels(left), parse_rgb_channels(right)) else {
        return right.to_owned();
    };
    let channel = |index: usize| {
        let a = f64::from(left[index]);
        let b = f64::from(right[index]);
        (a + (b - a) * amount).round() as u32
    };
    format!("#{:02x}{:02x}{:02x}", channel(0), channel(1), channel(2))
}

fn slugify(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .chars()
        .fold(String::new(), |mut slug, character| {
            if character.is_ascii_alphanumeric() {
                slug.push(character);
            } else if !slug.is_empty() && !slug.ends_with('-') {
                slug.push('-');
            }
            slug
        })
        .trim_end_matches('-')
        .to_owned()
}

const FALLBACK_CUSTOM_THEME_ID: &str = "custom-theme";

/// File id for a new custom theme: the ASCII slug of `name`, or a generic id when the
/// name has no ASCII letters or digits, suffixed until `taken` no longer matches.
fn custom_theme_id(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let slug = slugify(name);
    let base = if slug.is_empty() {
        FALLBACK_CUSTOM_THEME_ID
    } else {
        slug.as_str()
    };
    let mut id = base.to_owned();
    let mut suffix = 2;
    while taken(&id) {
        id = format!("{base}-{suffix}");
        suffix += 1;
    }
    id
}

fn title_case_slug(slug: &str) -> String {
    slug.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn themes_directory() -> PathBuf {
    super::preferences::config_directory().join("themes")
}

fn user_stylesheet_path() -> PathBuf {
    super::preferences::config_directory().join(USER_STYLE_FILE)
}

fn read_user_stylesheet() -> String {
    let path = user_stylesheet_path();
    match fs::read_to_string(&path) {
        Ok(css) => css,
        Err(error) => {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(%error, path = %path.display(), "cannot read user stylesheet");
            }
            String::new()
        }
    }
}

pub(in crate::ui) fn omarchy_state_dir() -> PathBuf {
    gtk::glib::home_dir().join(".local/state/omarchy/current")
}

#[cfg(test)]
mod tests;
