// SPDX-License-Identifier: GPL-3.0-or-later

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

use crate::model::{SortDirection, SortKey, ViewPreferences};

thread_local! {
    static SHARED_MANAGER: RefCell<std::rc::Weak<ThemeManager>> = const { RefCell::new(std::rc::Weak::new()) };
    static SOURCE_STYLE_PATH_INSTALLED: Cell<bool> = const { Cell::new(false) };
    static SOURCE_BUFFERS: RefCell<Vec<glib::WeakRef<sourceview5::Buffer>>> = const { RefCell::new(Vec::new()) };
}

const CORE_THEMES: [(&str, &str); 7] = [
    (
        "azure-glow",
        include_str!("../../data/themes/azure-glow.toml"),
    ),
    ("0x96f", include_str!("../../data/themes/0x96f.toml")),
    (
        "tokyo-night",
        include_str!("../../data/themes/tokyo-night.toml"),
    ),
    (
        "catppuccin",
        include_str!("../../data/themes/catppuccin.toml"),
    ),
    (
        "everforest",
        include_str!("../../data/themes/everforest.toml"),
    ),
    (
        "rose-pine",
        include_str!("../../data/themes/rose-pine.toml"),
    ),
    (
        "omarchy-light",
        include_str!("../../data/themes/omarchy-light.toml"),
    ),
];

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

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Preferences {
    mode: String,
    theme: String,
    #[serde(default = "default_enabled")]
    single_click_previews: bool,
    #[serde(default)]
    search_open_files_directly: bool,
    #[serde(default = "default_browser_mode")]
    browser_mode: String,
    #[serde(default = "default_browser_density")]
    browser_density: String,
    #[serde(default = "default_sort_key")]
    sort_key: String,
    #[serde(default = "default_sort_direction")]
    sort_direction: String,
    #[serde(default = "default_enabled")]
    check_for_updates: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            mode: "theme".to_owned(),
            theme: "azure-glow".to_owned(),
            single_click_previews: true,
            search_open_files_directly: false,
            browser_mode: default_browser_mode(),
            browser_density: default_browser_density(),
            sort_key: default_sort_key(),
            sort_direction: default_sort_direction(),
            check_for_updates: true,
        }
    }
}

fn default_enabled() -> bool {
    true
}

fn default_browser_mode() -> String {
    "columns".to_owned()
}

fn default_browser_density() -> String {
    "compact".to_owned()
}

fn default_sort_key() -> String {
    "name".to_owned()
}

fn default_sort_direction() -> String {
    "ascending".to_owned()
}

pub struct ThemeManager {
    provider: gtk::CssProvider,
    themes: RefCell<Vec<Theme>>,
    preferences: RefCell<Preferences>,
    omarchy_available: bool,
    omarchy_monitor: RefCell<Option<gio::FileMonitor>>,
    pending_omarchy_refresh: RefCell<Option<glib::SourceId>>,
    previewing: Cell<bool>,
}

impl ThemeManager {
    pub fn shared() -> Rc<Self> {
        SHARED_MANAGER.with(|shared| {
            if let Some(manager) = shared.borrow().upgrade() {
                return manager;
            }
            let manager = Self::load();
            shared.replace(Rc::downgrade(&manager));
            manager
        })
    }

    fn load() -> Rc<Self> {
        let mut themes = builtins();
        themes.extend(load_custom_themes());
        let omarchy_available = load_omarchy_theme().is_some();
        let mut preferences = read_preferences().unwrap_or_default();
        if !themes.iter().any(|theme| theme.id == preferences.theme) {
            preferences.theme = "azure-glow".to_owned();
        }
        if preferences.mode == "omarchy" && !omarchy_available {
            preferences.mode = "theme".to_owned();
        } else if !settings_path().is_file() && omarchy_available {
            preferences.mode = "omarchy".to_owned();
        }

        let manager = Rc::new(Self {
            provider: gtk::CssProvider::new(),
            themes: RefCell::new(themes),
            preferences: RefCell::new(preferences),
            omarchy_available,
            omarchy_monitor: RefCell::new(None),
            pending_omarchy_refresh: RefCell::new(None),
            previewing: Cell::new(false),
        });
        manager.install_provider();
        manager.apply_selected();
        manager.monitor_omarchy();
        manager
    }

    pub fn themes(&self) -> Vec<Theme> {
        self.themes.borrow().clone()
    }

    pub fn is_omarchy_available(&self) -> bool {
        self.omarchy_available
    }

    pub fn follows_omarchy(&self) -> bool {
        self.preferences.borrow().mode == "omarchy"
    }

    pub fn selected_id(&self) -> String {
        self.preferences.borrow().theme.clone()
    }

    pub fn single_click_previews(&self) -> bool {
        self.preferences.borrow().single_click_previews
    }

    pub fn set_single_click_previews(&self, enabled: bool) {
        self.preferences.borrow_mut().single_click_previews = enabled;
        self.save_preferences();
    }

    pub fn search_open_files_directly(&self) -> bool {
        self.preferences.borrow().search_open_files_directly
    }

    pub fn set_search_open_files_directly(&self, enabled: bool) {
        self.preferences.borrow_mut().search_open_files_directly = enabled;
        self.save_preferences();
    }

    pub fn checks_for_updates(&self) -> bool {
        self.preferences.borrow().check_for_updates
    }

    pub fn set_checks_for_updates(&self, enabled: bool) {
        self.preferences.borrow_mut().check_for_updates = enabled;
        self.save_preferences();
    }

    pub fn browser_mode(&self) -> super::browser_modes::BrowserMode {
        match self.preferences.borrow().browser_mode.as_str() {
            "grid" => super::browser_modes::BrowserMode::Grid,
            "explorer" => super::browser_modes::BrowserMode::Explorer,
            _ => super::browser_modes::BrowserMode::Columns,
        }
    }

    pub fn set_browser_mode(&self, mode: super::browser_modes::BrowserMode) {
        self.preferences.borrow_mut().browser_mode = match mode {
            super::browser_modes::BrowserMode::Columns => "columns",
            super::browser_modes::BrowserMode::Grid => "grid",
            super::browser_modes::BrowserMode::Explorer => "explorer",
        }
        .to_owned();
        self.save_preferences();
    }

    pub fn browser_density(&self) -> super::browser_modes::BrowserDensity {
        match self.preferences.borrow().browser_density.as_str() {
            "airy" => super::browser_modes::BrowserDensity::Airy,
            _ => super::browser_modes::BrowserDensity::Compact,
        }
    }

    pub fn set_browser_density(&self, density: super::browser_modes::BrowserDensity) {
        self.preferences.borrow_mut().browser_density = match density {
            super::browser_modes::BrowserDensity::Compact => "compact",
            super::browser_modes::BrowserDensity::Airy => "airy",
        }
        .to_owned();
        self.save_preferences();
    }

    pub fn sort_preferences(&self) -> ViewPreferences {
        sort_preferences(&self.preferences.borrow())
    }

    pub fn set_sort_preferences(&self, preferences: ViewPreferences) {
        let mut stored = self.preferences.borrow_mut();
        stored.sort_key = match preferences.sort_key {
            SortKey::Name => "name",
            SortKey::Size => "size",
            SortKey::Modified => "modified",
            SortKey::Type => "type",
        }
        .to_owned();
        stored.sort_direction = match preferences.sort_direction {
            SortDirection::Ascending => "ascending",
            SortDirection::Descending => "descending",
        }
        .to_owned();
        drop(stored);
        self.save_preferences();
    }

    pub fn select_theme(&self, id: &str) {
        if !self.themes.borrow().iter().any(|theme| theme.id == id) {
            return;
        }
        {
            let mut preferences = self.preferences.borrow_mut();
            preferences.mode = "theme".to_owned();
            preferences.theme = id.to_owned();
        }
        self.previewing.set(false);
        self.apply_selected();
        self.save_preferences();
    }

    pub fn set_follow_omarchy(&self, enabled: bool) {
        if enabled && !self.omarchy_available {
            return;
        }
        self.preferences.borrow_mut().mode = if enabled {
            "omarchy".to_owned()
        } else {
            "theme".to_owned()
        };
        self.previewing.set(false);
        self.apply_selected();
        self.save_preferences();
    }

    pub fn preview(&self, tokens: &ThemeTokens) {
        if validate_tokens(tokens).is_ok() {
            self.previewing.set(true);
            self.apply_tokens(tokens);
        }
    }

    pub fn cancel_preview(&self) {
        if self.previewing.replace(false) {
            self.apply_selected();
        }
    }

    pub fn save_custom_theme(&self, tokens: ThemeTokens) -> io::Result<String> {
        validate_tokens(&tokens)
            .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
        let base = slugify(&tokens.name);
        if base.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Enter a theme name",
            ));
        }
        let directory = themes_directory();
        fs::create_dir_all(&directory)?;
        let mut id = base.clone();
        let mut suffix = 2;
        while self.themes.borrow().iter().any(|theme| theme.id == id) {
            id = format!("{base}-{suffix}");
            suffix += 1;
        }
        let path = directory.join(format!("{id}.toml"));
        let value = toml::to_string_pretty(&tokens).map_err(io::Error::other)?;
        crate::storage::atomic_write(&path, value.as_bytes())?;

        let mut themes = self.themes.borrow_mut();
        if let Some(theme) = themes
            .iter_mut()
            .find(|theme| theme.id == id && theme.custom)
        {
            theme.tokens = tokens;
        } else {
            themes.push(Theme {
                id: id.clone(),
                tokens,
                custom: true,
            });
        }
        drop(themes);
        self.select_theme(&id);
        Ok(id)
    }

    pub fn starter_tokens(&self) -> ThemeTokens {
        self.current_tokens().unwrap_or_else(azure_tokens)
    }

    fn install_provider(&self) {
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &self.provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
            );
        }
    }

    fn apply_selected(&self) {
        if self.follows_omarchy() {
            if let Some(tokens) = load_omarchy_theme() {
                self.apply_tokens(&tokens);
            }
            return;
        }
        if let Some(tokens) = self.current_tokens() {
            self.apply_tokens(&tokens);
        }
    }

    fn current_tokens(&self) -> Option<ThemeTokens> {
        let id = self.preferences.borrow().theme.clone();
        self.themes
            .borrow()
            .iter()
            .find(|theme| theme.id == id)
            .map(|theme| theme.tokens.clone())
    }

    fn apply_tokens(&self, tokens: &ThemeTokens) {
        self.provider.load_from_string(&tokens_css(tokens));
        crate::assets::set_primary_icon_color(&tokens.accent);
        crate::assets::set_text_icon_color(&tokens.text);
        crate::assets::set_danger_icon_color(&tokens.danger);
        install_source_style_scheme(tokens);
    }

    fn save_preferences(&self) {
        let path = settings_path();
        let result = (|| -> io::Result<()> {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let value =
                toml::to_string_pretty(&*self.preferences.borrow()).map_err(io::Error::other)?;
            crate::storage::atomic_write(&path, value.as_bytes())
        })();
        if let Err(error) = result {
            tracing::warn!(%error, "unable to save theme preference");
        }
    }

    fn monitor_omarchy(self: &Rc<Self>) {
        if !self.omarchy_available {
            return;
        }
        let file = gio::File::for_path(omarchy_state_dir());
        let Ok(monitor) =
            file.monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        else {
            return;
        };
        let weak = Rc::downgrade(self);
        monitor.connect_changed(move |_, file, other_file, _| {
            if !is_omarchy_theme_event(file)
                && !other_file
                    .as_ref()
                    .is_some_and(|file| is_omarchy_theme_event(file))
            {
                return;
            }
            let Some(manager) = weak.upgrade() else {
                return;
            };
            if let Some(pending) = manager.pending_omarchy_refresh.borrow_mut().take() {
                pending.remove();
            }
            let weak = weak.clone();
            let refresh = glib::timeout_add_local_once(Duration::from_millis(75), move || {
                let Some(manager) = weak.upgrade() else {
                    return;
                };
                manager.pending_omarchy_refresh.borrow_mut().take();
                if manager.follows_omarchy() && !manager.previewing.get() {
                    manager.apply_selected();
                }
            });
            manager.pending_omarchy_refresh.replace(Some(refresh));
        });
        self.omarchy_monitor.replace(Some(monitor));
    }
}

fn is_omarchy_theme_event(file: &gio::File) -> bool {
    file.path()
        .as_deref()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "theme" || name == "theme.name")
}

fn builtins() -> Vec<Theme> {
    let mut themes: Vec<_> = CORE_THEMES
        .iter()
        .filter_map(|(id, source)| {
            toml::from_str(source).ok().map(|tokens| Theme {
                id: (*id).to_owned(),
                tokens,
                custom: false,
            })
        })
        .collect();
    if let Ok(catalog) = toml::from_str::<ThemeCatalog>(THEME_CATALOG) {
        themes.extend(catalog.themes.into_iter().map(|theme| Theme {
            id: theme.id,
            tokens: theme.tokens,
            custom: false,
        }));
    }
    themes.sort_by_key(|theme| theme.tokens.name.to_lowercase());
    themes
}

fn azure_tokens() -> ThemeTokens {
    toml::from_str(CORE_THEMES[0].1).unwrap_or_else(|_| ThemeTokens {
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
            let tokens: ThemeTokens = toml::from_str(&source).ok()?;
            validate_tokens(&tokens).ok()?;
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

fn read_preferences() -> Option<Preferences> {
    toml::from_str(&fs::read_to_string(settings_path()).ok()?).ok()
}

fn sort_preferences(preferences: &Preferences) -> ViewPreferences {
    let sort_key = match preferences.sort_key.as_str() {
        "name" => SortKey::Name,
        "size" => SortKey::Size,
        "modified" => SortKey::Modified,
        "type" => SortKey::Type,
        _ => return ViewPreferences::default(),
    };
    let sort_direction = match preferences.sort_direction.as_str() {
        "ascending" => SortDirection::Ascending,
        "descending" => SortDirection::Descending,
        _ => return ViewPreferences::default(),
    };
    ViewPreferences {
        sort_key,
        sort_direction,
        ..ViewPreferences::default()
    }
}

fn load_omarchy_theme() -> Option<ThemeTokens> {
    let state = omarchy_state_dir();
    let name = fs::read_to_string(state.join("theme.name")).ok()?;
    let colors = fs::read_to_string(state.join("theme/colors.toml")).ok()?;
    tokens_from_quattro(name.trim(), &colors)
}

fn tokens_from_quattro(name: &str, source: &str) -> Option<ThemeTokens> {
    let values: toml::Value = toml::from_str(source).ok()?;
    let get = |key: &str| {
        values
            .get(key)
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
    };
    let source_background = get("background")?;
    let text = get("foreground")?;
    let accent = get("accent")?;
    let selection = get("selection").unwrap_or_else(|| accent.clone());
    let shadow = get("color8").unwrap_or_else(|| source_background.clone());
    Some(ThemeTokens {
        name: title_case_slug(name),
        background: blend(&source_background, &shadow, 0.35),
        surface: blend(&source_background, &shadow, 0.65),
        muted: blend(&shadow, &text, 0.10),
        highlight: blend(&shadow, &selection, 0.10),
        border: blend(&shadow, &text, 0.36),
        dim_text: blend(&source_background, &text, 0.62),
        text,
        accent,
        danger: get("color1").unwrap_or_else(default_danger),
    })
}

fn default_danger() -> String {
    "#e5484d".to_owned()
}

fn validate_tokens(tokens: &ThemeTokens) -> Result<(), &'static str> {
    if tokens.name.trim().is_empty() {
        return Err("Enter a theme name");
    }
    for color in [
        &tokens.background,
        &tokens.surface,
        &tokens.text,
        &tokens.accent,
        &tokens.danger,
        &tokens.muted,
        &tokens.highlight,
        &tokens.border,
        &tokens.dim_text,
    ] {
        if gdk::RGBA::parse(color).is_err() {
            return Err("Every color must be a valid CSS color");
        }
    }
    Ok(())
}

fn source_style_scheme() -> Option<sourceview5::StyleScheme> {
    sourceview5::StyleSchemeManager::default().scheme("strata-current")
}

pub(super) fn register_source_buffer(buffer: &sourceview5::Buffer) {
    buffer.set_style_scheme(source_style_scheme().as_ref());
    SOURCE_BUFFERS.with(|buffers| {
        let mut buffers = buffers.borrow_mut();
        buffers.retain(|buffer| buffer.upgrade().is_some());
        let weak = glib::WeakRef::new();
        weak.set(Some(buffer));
        buffers.push(weak);
    });
}

fn install_source_style_scheme(tokens: &ThemeTokens) {
    let directory = glib::user_cache_dir().join("strata").join("source-styles");
    if let Err(error) = fs::create_dir_all(&directory).and_then(|()| {
        let value = source_style_scheme_xml(tokens);
        crate::storage::atomic_write(&directory.join("strata-current.xml"), value.as_bytes())
    }) {
        tracing::warn!(%error, "unable to write preview syntax style");
        return;
    }

    let manager = sourceview5::StyleSchemeManager::default();
    SOURCE_STYLE_PATH_INSTALLED.with(|installed| {
        if !installed.replace(true) {
            manager.append_search_path(&directory.to_string_lossy());
        }
    });
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

fn source_style_scheme_xml(tokens: &ThemeTokens) -> String {
    let string = blend(&tokens.accent, &tokens.text, 0.48);
    let constant = blend(&tokens.accent, &tokens.text, 0.18);
    let type_color = blend(&tokens.accent, &tokens.text, 0.24);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<style-scheme id="strata-current" _name="Strata Current Theme" version="1.0">
  <color name="background" value="{}"/>
  <color name="surface" value="{}"/>
  <color name="text" value="{}"/>
  <color name="accent" value="{}"/>
  <color name="selection" value="{}"/>
  <color name="dim" value="{}"/>
  <color name="string" value="{}"/>
  <color name="constant" value="{}"/>
  <color name="type" value="{}"/>
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
  <style name="def:statement" foreground="accent" bold="true"/>
  <style name="def:type" foreground="type" bold="true"/>
  <style name="def:preprocessor" foreground="type"/>
  <style name="def:heading" foreground="accent" bold="true"/>
  <style name="def:link-destination" foreground="string" underline="single"/>
  <style name="def:error" foreground="background" background="accent" bold="true"/>
</style-scheme>
"#,
        tokens.background,
        tokens.surface,
        tokens.text,
        tokens.accent,
        tokens.highlight,
        tokens.dim_text,
        string,
        constant,
        type_color,
    )
}

fn tokens_css(tokens: &ThemeTokens) -> String {
    format!(
        "@define-color theme_bg {};\n@define-color theme_surface {};\n@define-color theme_text {};\n@define-color theme_accent {};\n@define-color theme_danger {};\n@define-color theme_muted {};\n@define-color theme_highlight {};\n@define-color theme_border {};\n@define-color theme_dim_text {};\n",
        tokens.background,
        tokens.surface,
        tokens.text,
        tokens.accent,
        tokens.danger,
        tokens.muted,
        tokens.highlight,
        tokens.border,
        tokens.dim_text,
    )
}

fn blend(left: &str, right: &str, amount: f64) -> String {
    let parse = |value: &str| u32::from_str_radix(value.trim_start_matches('#'), 16).ok();
    let (Some(left), Some(right)) = (parse(left), parse(right)) else {
        return right.to_owned();
    };
    let channel = |shift| {
        let a = f64::from((left >> shift) & 0xff_u32);
        let b = f64::from((right >> shift) & 0xff_u32);
        (a + (b - a) * amount).round() as u32
    };
    format!("#{:02x}{:02x}{:02x}", channel(16), channel(8), channel(0))
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

fn config_directory() -> PathBuf {
    gtk::glib::user_config_dir().join("strata")
}
fn settings_path() -> PathBuf {
    config_directory().join("settings.toml")
}
fn themes_directory() -> PathBuf {
    config_directory().join("themes")
}
fn omarchy_state_dir() -> PathBuf {
    gtk::glib::home_dir().join(".local/state/omarchy/current")
}

#[cfg(test)]
mod tests;
