// SPDX-License-Identifier: MIT

//! The GtkSourceView style scheme generated from the theme, installed from
//! memory on the first source preview and restaged on every theme change.

use std::{cell::Cell, cell::RefCell, path::Path};

use gtk::{gdk, gio, glib};
use sourceview5::prelude::*;

use super::{
    SourcePalette, ThemeTokens, blend, color_to_hex,
    weak_registry::{WeakRegistry, for_each_live, register},
};

thread_local! {
    static SOURCE_STYLE_PATH_INSTALLED: Cell<bool> = const { Cell::new(false) };
    static SOURCE_LANGUAGE_PATH_INSTALLED: Cell<bool> = const { Cell::new(false) };
    static SOURCE_BUFFERS: WeakRegistry<sourceview5::Buffer> = const { RefCell::new(Vec::new()) };
    /// Installed on the first source preview buffer, so startup performs no SourceView I/O.
    static PENDING_STYLE_TOKENS: RefCell<Option<(ThemeTokens, Option<SourcePalette>)>> = const { RefCell::new(None) };
    static STYLE_SCHEME_DIRTY: Cell<bool> = const { Cell::new(true) };
    static SOURCE_STYLE_RESOURCE: RefCell<Option<gio::Resource>> = const { RefCell::new(None) };
}

const SOURCE_STYLE_FOLDER: &str = "/io/github/lgse/Strata/generated/source-styles/";

fn source_style_manager() -> sourceview5::StyleSchemeManager {
    let manager = sourceview5::StyleSchemeManager::default();
    SOURCE_STYLE_PATH_INSTALLED.with(|installed| {
        if !installed.replace(true) {
            manager.set_search_path(&[format!("resource://{SOURCE_STYLE_FOLDER}").as_str()]);
        }
    });
    manager
}

pub(in crate::ui) fn source_languages() -> sourceview5::LanguageManager {
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

pub(in crate::ui) fn new_source_buffer() -> sourceview5::Buffer {
    let manager = source_style_manager();
    ensure_source_style_scheme_installed();
    let buffer = sourceview5::Buffer::new(None);
    buffer.set_style_scheme(manager.scheme("strata-current").as_ref());
    SOURCE_BUFFERS.with(|buffers| register(buffers, &buffer));
    buffer
}

pub(super) fn stage_source_style_scheme(
    tokens: &ThemeTokens,
    source_palette: Option<&SourcePalette>,
) {
    PENDING_STYLE_TOKENS
        .with(|pending| pending.replace(Some((tokens.clone(), source_palette.cloned()))));
    STYLE_SCHEME_DIRTY.with(|dirty| dirty.set(true));
    let live = SOURCE_BUFFERS.with(|buffers| {
        for_each_live(buffers, |_| {});
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
        for_each_live(buffers, |buffer| buffer.set_style_scheme(scheme.as_ref()));
    });
}

pub(super) fn resolved_source_palette(
    tokens: &ThemeTokens,
    palette: Option<&SourcePalette>,
) -> SourcePalette {
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

pub(super) fn source_style_scheme_xml(
    tokens: &ThemeTokens,
    palette: Option<&SourcePalette>,
) -> String {
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
