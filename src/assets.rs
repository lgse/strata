// SPDX-License-Identifier: MIT

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    ffi::CString,
    fs, io,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use gtk::{gdk, gio, glib, prelude::*};

mod vector;

pub mod icons {
    pub const ARROW_DOWN: &str = "strata-arrow-down";
    pub const ARROW_DOWN_WIDE_NARROW: &str = "strata-arrow-down-wide-narrow";
    pub const ARROW_LEFT: &str = "strata-arrow-left";
    pub const ARROW_RIGHT: &str = "strata-arrow-right";
    pub const ARROW_UP: &str = "strata-arrow-up";
    pub const ARROW_UP_NARROW_WIDE: &str = "strata-arrow-up-narrow-wide";
    pub const APP_WINDOW: &str = "strata-app-window";
    pub const CHECK: &str = "strata-check";
    pub const CIRCLE_CHECK: &str = "strata-circle-check";
    pub const CIRCLE_X: &str = "strata-circle-x";
    pub const CHECK_ON_PRIMARY: &str = "strata-check-on-primary";
    pub const CHEVRON_RIGHT: &str = "strata-chevron-right";
    pub const CLOCK: &str = "strata-clock";
    pub const CLIPBOARD_PASTE: &str = "strata-clipboard-paste";
    pub const COPY: &str = "strata-copy";
    pub const COPY_PLUS: &str = "strata-copy-plus";
    pub const COG: &str = "strata-cog";
    pub const CORNER_DOWN_LEFT: &str = "strata-corner-down-left";
    pub const DISC: &str = "strata-disc";
    pub const DATABASE: &str = "strata-database";
    pub const DOCUMENTS: &str = "strata-file-text";
    pub const DOWNLOADS: &str = "strata-download";
    pub const EJECT: &str = "strata-eject";
    pub const EYE: &str = "strata-eye";
    pub const EYE_OFF: &str = "strata-eye-off";
    pub const EXTERNAL_LINK: &str = "strata-external-link";
    pub const FILE_ARCHIVE: &str = "strata-file-archive";
    pub const FILE_AUDIO: &str = "strata-audio-lines";
    pub const FILE_BRACES: &str = "strata-file-braces";
    pub const FILE_CODE: &str = "strata-file-code";
    pub const LANG_ASTRO: &str = "strata-lang-astro";
    pub const LANG_C: &str = "strata-lang-c";
    pub const LANG_CLOJURE: &str = "strata-lang-clojure";
    pub const LANG_CPP: &str = "strata-lang-cpp";
    pub const LANG_CRYSTAL: &str = "strata-lang-crystal";
    pub const LANG_CSHARP: &str = "strata-lang-csharp";
    pub const LANG_CSS: &str = "strata-lang-css";
    pub const LANG_DART: &str = "strata-lang-dart";
    pub const LANG_ELIXIR: &str = "strata-lang-elixir";
    pub const LANG_ELM: &str = "strata-lang-elm";
    pub const LANG_ERLANG: &str = "strata-lang-erlang";
    pub const LANG_FSHARP: &str = "strata-lang-fsharp";
    pub const LANG_GO: &str = "strata-lang-go";
    pub const LANG_GRAPHQL: &str = "strata-lang-graphql";
    pub const LANG_GROOVY: &str = "strata-lang-groovy";
    pub const LANG_HASKELL: &str = "strata-lang-haskell";
    pub const LANG_HTML: &str = "strata-lang-html";
    pub const LANG_JAVA: &str = "strata-lang-java";
    pub const LANG_JS: &str = "strata-lang-js";
    pub const LANG_JULIA: &str = "strata-lang-julia";
    pub const LANG_JUPYTER: &str = "strata-lang-jupyter";
    pub const LANG_KOTLIN: &str = "strata-lang-kotlin";
    pub const LANG_LUA: &str = "strata-lang-lua";
    pub const LANG_NIXOS: &str = "strata-lang-nixos";
    pub const LANG_NIM: &str = "strata-lang-nim";
    pub const LANG_OCAML: &str = "strata-lang-ocaml";
    pub const LANG_PERL: &str = "strata-lang-perl";
    pub const LANG_PHP: &str = "strata-lang-php";
    pub const LANG_PYTHON: &str = "strata-lang-python";
    pub const LANG_QT: &str = "strata-lang-qt";
    pub const LANG_R: &str = "strata-lang-r";
    pub const LANG_RACKET: &str = "strata-lang-racket";
    pub const LANG_RUBY: &str = "strata-lang-ruby";
    pub const LANG_RUST: &str = "strata-lang-rust";
    pub const LANG_SCALA: &str = "strata-lang-scala";
    pub const LANG_SOLIDITY: &str = "strata-lang-solidity";
    pub const LANG_SVELTE: &str = "strata-lang-svelte";
    pub const LANG_SWIFT: &str = "strata-lang-swift";
    pub const LANG_TERRAFORM: &str = "strata-lang-terraform";
    pub const LANG_TS: &str = "strata-lang-ts";
    pub const LANG_VUE: &str = "strata-lang-vue";
    pub const LANG_ZIG: &str = "strata-lang-zig";
    pub const FILE_SPREADSHEET: &str = "strata-file-spreadsheet";
    pub const FILE_TERMINAL: &str = "strata-file-terminal";
    pub const FILE_PLUS: &str = "strata-file-plus";
    pub const FILE_TYPE: &str = "strata-file-type";
    pub const FOLDER: &str = "strata-folder";
    pub const FOLDER_ARCHIVE: &str = "strata-folder-archive";
    pub const FOLDER_INPUT: &str = "strata-folder-input";
    pub const FOLDER_OPEN: &str = "strata-folder-open";
    pub const FOLDER_OUTPUT: &str = "strata-folder-output";
    pub const FOLDER_PLUS: &str = "strata-folder-plus";
    pub const HARD_DRIVE: &str = "strata-hard-drive";
    pub const INFO: &str = "strata-info";
    pub const GLOBE: &str = "strata-globe";
    pub const CODE_XML: &str = "strata-code-xml";
    pub const BUG: &str = "strata-bug";
    pub const BOX: &str = "strata-box";
    pub const SCALE: &str = "strata-scale";
    pub const CORNER_DOWN_RIGHT: &str = "strata-corner-down-right";
    pub const FUNNEL: &str = "strata-funnel";
    pub const COLUMNS: &str = "strata-columns";
    pub const ICONS: &str = "strata-icons";
    pub const HOME: &str = "strata-house";
    pub const LIBRARY: &str = "strata-library";
    pub const LIST: &str = "strata-list";
    pub const LIST_CHECKS: &str = "strata-list-checks";
    pub const LOCK: &str = "strata-lock";
    pub const LOCK_OPEN: &str = "strata-lock-open";
    pub const KEY: &str = "strata-key";
    pub const KEY_ROUND: &str = "strata-key-round";
    pub const MONITOR: &str = "strata-monitor";
    pub const NETWORK: &str = "strata-network";
    pub const PALETTE: &str = "strata-palette";
    pub const PANEL_LEFT: &str = "strata-panel-left-symbolic";
    pub const PANEL_RIGHT_CLOSE: &str = "strata-panel-right-close";
    pub const PAUSE: &str = "strata-pause";
    pub const PACKAGE_OPEN: &str = "strata-package-open";
    pub const PACKAGE_PLUS: &str = "strata-package-plus";
    pub const PENCIL: &str = "strata-pencil";
    pub const PIN: &str = "strata-pin";
    pub const PLAY: &str = "strata-play";
    pub const MAXIMIZE: &str = "strata-maximize";
    pub const MINIMIZE: &str = "strata-minimize";
    pub const SKIP_BACK: &str = "strata-skip-back";
    pub const SKIP_FORWARD: &str = "strata-skip-forward";
    pub const MINUS: &str = "strata-minus";
    pub const MUSIC: &str = "strata-music-2";
    pub const PLUS: &str = "strata-plus";
    pub const PRESENTATION: &str = "strata-presentation";
    pub const PRINTER: &str = "strata-printer";
    pub const PICTURES: &str = "strata-image";
    pub const ROWS: &str = "strata-rows";
    pub const ROUTE: &str = "strata-route";
    pub const SCISSORS: &str = "strata-scissors";
    pub const SEARCH: &str = "strata-search";
    pub const SEND_HORIZONTAL: &str = "strata-send-horizontal";
    pub const SETTINGS: &str = "strata-settings";
    pub const SETTINGS_2: &str = "strata-settings-2";
    pub const REFRESH: &str = "strata-refresh";
    pub const SHREDDER: &str = "strata-shredder";
    pub const SLIDERS: &str = "strata-sliders-horizontal";
    pub const TERMINAL: &str = "strata-terminal";
    pub const TRASH: &str = "strata-trash";
    pub const TRIANGLE_ALERT: &str = "strata-triangle-alert";
    pub const UNDO_2: &str = "strata-undo-2";
    pub const UNPLUG: &str = "strata-unplug";
    pub const VIDEOS: &str = "strata-video";
    pub const VOLUME_2: &str = "strata-volume-2";
    pub const VOLUME_X: &str = "strata-volume-x";
    pub const WRAP_TEXT: &str = "strata-wrap-text";
    pub const X: &str = "strata-x";

    pub const CUSTOMIZATION_CHOICES: [(&str, &str); 16] = [
        (DOCUMENTS, "Documents"),
        (DOWNLOADS, "Downloads"),
        (FILE_CODE, "Code"),
        (FILE_ARCHIVE, "Archive"),
        (PICTURES, "Pictures"),
        (VIDEOS, "Videos"),
        (TERMINAL, "Terminal"),
        (HOME, "Home"),
        (HARD_DRIVE, "Storage"),
        (NETWORK, "Network"),
        (MONITOR, "Computer"),
        (KEY, "Private"),
        (PIN, "Pinned"),
        (PLAY, "Media"),
        (SETTINGS, "Settings"),
        (LIST_CHECKS, "Tasks"),
    ];

    pub fn custom_emoji(name: &str) -> Option<&str> {
        let emoji = name.strip_prefix("emoji:")?;
        (!emoji.is_empty() && emoji.len() <= 64 && !emoji.chars().any(char::is_control))
            .then_some(emoji)
    }

    pub fn is_customization_choice(name: &str) -> bool {
        CUSTOMIZATION_CHOICES
            .iter()
            .any(|(icon_name, _)| *icon_name == name)
            || custom_emoji(name).is_some()
    }
}

const FONT_VERSION: &str = "2.304";
const ICON_TEXTURE_PX: i32 = 96;
const ICON_TEXTURE_CACHE_LIMIT: usize = 256;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum IconContext {
    #[default]
    Interface,
    Grid,
}
const JETBRAINS_MONO: &[u8] = include_bytes!("../data/fonts/JetBrainsMono[wght].ttf");

pub const CHROME_ICON_PX: i32 = 16;

struct PrimaryIcon {
    image: glib::WeakRef<gtk::Image>,
    name: String,
}

thread_local! {
    static INTERFACE_ICON_SCALE: Cell<f64> = const { Cell::new(1.0) };
    static INTERFACE_ICONS: RefCell<Vec<(glib::WeakRef<gtk::Image>, i32)>> = const { RefCell::new(Vec::new()) };
    static PRIMARY_ICON_COLOR: RefCell<String> = RefCell::new("#8bc9eb".to_owned());
    static PRIMARY_ICONS: RefCell<Vec<PrimaryIcon>> = const { RefCell::new(Vec::new()) };
    static TEXT_ICON_COLOR: RefCell<String> = RefCell::new("#e6edf3".to_owned());
    static TEXT_ICONS: RefCell<Vec<PrimaryIcon>> = const { RefCell::new(Vec::new()) };
    static DANGER_ICON_COLOR: RefCell<String> = RefCell::new("#e5484d".to_owned());
    static DANGER_ICONS: RefCell<Vec<PrimaryIcon>> = const { RefCell::new(Vec::new()) };
    static ICON_TEXTURES: RefCell<IconTextureCache> = RefCell::new(IconTextureCache::default());
}

pub fn prepare() -> Result<(), Box<dyn std::error::Error>> {
    gio::resources_register_include!("strata.gresource")?;

    let font_directory = glib::user_cache_dir()
        .join("strata")
        .join("fonts")
        .join(FONT_VERSION);
    fs::create_dir_all(&font_directory)?;

    let regular = font_directory.join("JetBrainsMono.ttf");
    write_if_changed(&regular, JETBRAINS_MONO)?;
    register_application_fonts([regular])?;

    Ok(())
}

pub fn register_icon_theme() {
    if let Some(display) = gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_resource_path("/io/github/lgse/Strata/icons");
    }
    // Desktop shells resolve the window icon by matching the application ID to a
    // desktop entry, but GTK also needs the name to expose the bundled icon on its
    // own surfaces and on compositors that accept a toplevel icon.
    gtk::Window::set_default_icon_name(crate::APPLICATION_ID);
}

pub fn primary_icon(name: &str, pixel_size: i32) -> gtk::Image {
    let image = gtk::Image::new();
    register_interface_icon(&image, pixel_size);
    set_primary_icon(&image, name);
    image
}

pub fn set_interface_icon_scale(scale: f64) {
    INTERFACE_ICON_SCALE.set(scale);
    INTERFACE_ICONS.with(|icons| {
        icons.borrow_mut().retain(|(weak, base)| {
            let Some(image) = weak.upgrade() else {
                return false;
            };
            image.set_pixel_size((f64::from(*base) * scale).round().max(1.0) as i32);
            true
        });
    });
}

fn register_interface_icon(image: &gtk::Image, base: i32) {
    image.set_pixel_size(
        (f64::from(base) * INTERFACE_ICON_SCALE.get())
            .round()
            .max(1.0) as i32,
    );
    INTERFACE_ICONS.with(|icons| {
        let mut icons = icons.borrow_mut();
        icons.retain(|(image, _)| image.upgrade().is_some());
        icons.push((image.downgrade(), base));
    });
    image.connect_scale_factor_notify(|image| {
        for (registry, color) in [
            (&PRIMARY_ICONS, primary_icon_color()),
            (
                &TEXT_ICONS,
                TEXT_ICON_COLOR.with(|color| color.borrow().clone()),
            ),
            (
                &DANGER_ICONS,
                DANGER_ICON_COLOR.with(|color| color.borrow().clone()),
            ),
        ] {
            let name = registry.with(|icons| {
                icons
                    .borrow()
                    .iter()
                    .find(|icon| icon.image.upgrade().as_ref() == Some(image))
                    .map(|icon| icon.name.clone())
            });
            if let Some(name) = name {
                apply_primary_icon(image, &name, &color);
                break;
            }
        }
    });
}

pub fn text_icon(name: &str, pixel_size: i32) -> gtk::Image {
    let image = gtk::Image::new();
    register_interface_icon(&image, pixel_size);
    let color = TEXT_ICON_COLOR.with(|color| color.borrow().clone());
    apply_primary_icon(&image, name, &color);
    TEXT_ICONS.with(|icons| register_icon(icons, &image, name));
    image
}

pub fn set_text_icon_color(color: &str) {
    TEXT_ICON_COLOR.with(|current| current.replace(color.to_owned()));
    TEXT_ICONS.with(|icons| recolor_registered_icons(icons, color));
}

pub fn chrome_icon(name: &str) -> gtk::Image {
    let image = gtk::Image::new();
    image.add_css_class("chrome-icon");
    register_interface_icon(&image, CHROME_ICON_PX);
    set_primary_icon(&image, name);
    // Fill stretches paintables when desktop themes allocate extra button space.
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    image
}

pub fn set_primary_icon(image: &gtk::Image, name: &str) {
    let color = PRIMARY_ICON_COLOR.with(|color| color.borrow().clone());
    apply_primary_icon(image, name, &color);
    PRIMARY_ICONS.with(|icons| {
        let mut icons = icons.borrow_mut();
        icons.retain(|icon| icon.image.upgrade().is_some());
        if let Some(icon) = icons
            .iter_mut()
            .find(|icon| icon.image.upgrade().as_ref() == Some(image))
        {
            icon.name = name.to_owned();
            return;
        }
        let image_ref = glib::WeakRef::new();
        image_ref.set(Some(image));
        icons.push(PrimaryIcon {
            image: image_ref,
            name: name.to_owned(),
        });
    });
}

pub fn set_primary_icon_color(color: &str) {
    PRIMARY_ICON_COLOR.with(|current| current.replace(color.to_owned()));
    PRIMARY_ICONS.with(|icons| recolor_registered_icons(icons, color));
}

pub fn remove_primary_icon(image: &gtk::Image) {
    PRIMARY_ICONS.with(|icons| {
        icons
            .borrow_mut()
            .retain(|icon| icon.image.upgrade().as_ref() != Some(image));
    });
}

pub fn set_custom_colored_icon(image: &gtk::Image, name: &str, color: &str) {
    remove_primary_icon(image);
    apply_primary_icon(image, name, color);
}

pub fn set_folder_decoration_icon(image: &gtk::Image, decoration: &str, color: &str) {
    remove_primary_icon(image);
    if let Some(texture) = sized_folder_decoration_paintable(
        decoration,
        color,
        image.pixel_size(),
        image.scale_factor(),
        IconContext::Interface,
    ) {
        image.set_paintable(Some(&texture));
    } else {
        apply_primary_icon(image, icons::FOLDER, color);
    }
}

pub fn set_emoji_icon(image: &gtk::Image, emoji: &str) {
    remove_primary_icon(image);
    if let Some(texture) = emoji_texture(emoji) {
        image.set_paintable(Some(&texture));
    }
}

pub fn emoji_icon_paintable(emoji: &str) -> Option<gdk::Texture> {
    emoji_texture(emoji)
}

pub fn primary_icon_color() -> String {
    PRIMARY_ICON_COLOR.with(|color| color.borrow().clone())
}

pub fn danger_icon(name: &str, pixel_size: i32) -> gtk::Image {
    let image = gtk::Image::new();
    register_interface_icon(&image, pixel_size);
    let color = DANGER_ICON_COLOR.with(|color| color.borrow().clone());
    apply_primary_icon(&image, name, &color);
    DANGER_ICONS.with(|icons| register_icon(icons, &image, name));
    image
}

pub fn set_danger_icon_color(color: &str) {
    DANGER_ICON_COLOR.with(|current| current.replace(color.to_owned()));
    DANGER_ICONS.with(|icons| recolor_registered_icons(icons, color));
}

fn register_icon(icons: &RefCell<Vec<PrimaryIcon>>, image: &gtk::Image, name: &str) {
    let mut icons = icons.borrow_mut();
    icons.retain(|icon| icon.image.upgrade().is_some());
    if let Some(icon) = icons
        .iter_mut()
        .find(|icon| icon.image.upgrade().as_ref() == Some(image))
    {
        icon.name = name.to_owned();
        return;
    }
    let image_ref = glib::WeakRef::new();
    image_ref.set(Some(image));
    icons.push(PrimaryIcon {
        image: image_ref,
        name: name.to_owned(),
    });
}

fn recolor_registered_icons(icons: &RefCell<Vec<PrimaryIcon>>, color: &str) {
    icons.borrow_mut().retain(|icon| {
        let Some(image) = icon.image.upgrade() else {
            return false;
        };
        apply_primary_icon(&image, &icon.name, color);
        true
    });
}

fn apply_primary_icon(image: &gtk::Image, name: &str, color: &str) {
    let texture_px = if image.has_css_class("chrome-icon") {
        texture_px_for_pixel_size(image.pixel_size())
            .max(image.pixel_size().saturating_mul(image.scale_factor()))
            .clamp(24, 768)
    } else {
        ICON_TEXTURE_PX
            .max(image.pixel_size().saturating_mul(image.scale_factor()))
            .clamp(24, 768)
    };
    if let Some(texture) = primary_icon_texture_at(
        name,
        color,
        texture_px,
        image.pixel_size(),
        IconContext::Interface,
    ) {
        image.set_paintable(Some(&texture));
    } else {
        image.set_icon_name(Some(name));
    }
}

fn texture_px_for_pixel_size(pixel_size: i32) -> i32 {
    // Avoid excessive downsampling of toolbar strokes while supporting 2× displays.
    if pixel_size > 0 {
        pixel_size.saturating_mul(2).clamp(24, 768)
    } else {
        ICON_TEXTURE_PX
    }
}

pub(crate) fn sized_icon_paintable(
    name: &str,
    color: &str,
    logical_px: i32,
    scale_factor: i32,
    context: IconContext,
) -> Option<gdk::Texture> {
    let texture_px = ICON_TEXTURE_PX
        .max(logical_px.saturating_mul(scale_factor))
        .clamp(24, 768);
    primary_icon_texture_at(name, color, texture_px, logical_px, context)
}

pub(crate) fn drag_icon_texture(name: &str, color: &str, texture_px: i32) -> Option<gdk::Texture> {
    let texture_px = texture_px.clamp(24, 768);
    primary_icon_texture_at(name, color, texture_px, texture_px, IconContext::Interface)
}

fn primary_icon_texture_at(
    name: &str,
    color: &str,
    texture_px: i32,
    logical_px: i32,
    context: IconContext,
) -> Option<gdk::Texture> {
    let path = format!("/io/github/lgse/Strata/icons/scalable/actions/{name}.svg");
    let data = gio::resources_lookup_data(&path, gio::ResourceLookupFlags::NONE).ok()?;
    let source = std::str::from_utf8(data.as_ref()).ok()?;
    let mut source = recolor_icon_source(source, color);
    if name == icons::FOLDER {
        source = source.replacen(
            "fill=\"none\"",
            &format!("fill=\"{color}\" fill-opacity=\"0.15\""),
            1,
        );
    }
    texture_from_svg(
        &stroke_cache_name(name, logical_px, context),
        color,
        texture_px,
        svg_at_texture_size(
            compensate_icon_strokes(source, logical_px, context),
            texture_px,
        ),
    )
}

fn stroke_cache_name(name: &str, logical_px: i32, context: IconContext) -> String {
    let context = match context {
        IconContext::Interface if logical_px <= 64 => return name.to_owned(),
        IconContext::Interface => "interface",
        IconContext::Grid => "grid",
    };
    format!("{name}:{context}:stroke-size:{logical_px}")
}

fn compensate_icon_strokes(source: String, logical_px: i32, context: IconContext) -> String {
    let weight = match context {
        IconContext::Interface if logical_px <= 64 => return source,
        IconContext::Interface => 1.0,
        IconContext::Grid => 0.5,
    };
    // Logical size controls perceived weight; raster resolution only controls sharpness.
    let factor = weight * (64.0 / f64::from(logical_px.max(64))).powf(0.35);
    source
        .replace(
            "stroke-width=\"2\"",
            &format!("stroke-width=\"{}\"", 2.0 * factor),
        )
        .replace(
            "stroke-width=\"2.7\"",
            &format!("stroke-width=\"{}\"", 2.7 * factor),
        )
}

pub(crate) fn sized_folder_decoration_paintable(
    decoration: &str,
    color: &str,
    logical_px: i32,
    scale_factor: i32,
    context: IconContext,
) -> Option<gdk::Texture> {
    let texture_px = ICON_TEXTURE_PX
        .max(logical_px.saturating_mul(scale_factor))
        .clamp(24, 768);
    let folder_data = gio::resources_lookup_data(
        "/io/github/lgse/Strata/icons/scalable/actions/strata-folder.svg",
        gio::ResourceLookupFlags::NONE,
    )
    .ok()?;
    let folder = std::str::from_utf8(folder_data.as_ref()).ok()?;
    let mut source = svg_at_texture_size(recolor_icon_source(folder, color), texture_px).replacen(
        "fill=\"none\"",
        &format!("fill=\"{color}\" fill-opacity=\"0.92\""),
        1,
    );
    if let Some(emoji) = icons::custom_emoji(decoration) {
        return folder_emoji_texture(
            &compensate_icon_strokes(source, logical_px, context),
            emoji,
            color,
            logical_px,
            texture_px,
            context,
        );
    }

    let foreground = contrasting_foreground(color);
    let path = format!("/io/github/lgse/Strata/icons/scalable/actions/{decoration}.svg");
    let data = gio::resources_lookup_data(&path, gio::ResourceLookupFlags::NONE).ok()?;
    let badge = std::str::from_utf8(data.as_ref()).ok()?;
    let body = svg_body(badge)?;
    let overlay = format!(
        r#"<g transform="translate(5.5 6.8) scale(.54)" fill="none" stroke="{foreground}" stroke-width="2.7" stroke-linecap="round" stroke-linejoin="round">{body}</g>"#,
    );
    source = source.replacen("</svg>", &format!("{overlay}</svg>"), 1);
    texture_from_svg(
        &stroke_cache_name(
            &format!("folder-decoration:{decoration}"),
            logical_px,
            context,
        ),
        color,
        texture_px,
        compensate_icon_strokes(source, logical_px, context),
    )
}

fn folder_emoji_texture(
    folder_source: &str,
    emoji: &str,
    color: &str,
    logical_px: i32,
    texture_px: i32,
    context: IconContext,
) -> Option<gdk::Texture> {
    let key = (
        stroke_cache_name(&format!("folder-emoji:{emoji}"), logical_px, context),
        color.to_owned(),
        texture_px,
    );
    if let Some(texture) = cached_icon_texture(&key) {
        return Some(texture);
    }
    let folder = vector::surface(folder_source, texture_px)?;
    render_emoji_texture(key, emoji, 52.0, (44.0, 44.0), (48.0, 56.0), Some(&folder))
}

fn emoji_texture(emoji: &str) -> Option<gdk::Texture> {
    let key = (
        format!("emoji:{emoji}"),
        "native".to_owned(),
        ICON_TEXTURE_PX,
    );
    if let Some(texture) = cached_icon_texture(&key) {
        return Some(texture);
    }
    render_emoji_texture(key, emoji, 78.0, (82.0, 82.0), (48.0, 48.0), None)
}

fn render_emoji_texture(
    key: (String, String, i32),
    emoji: &str,
    preferred_size: f64,
    bounds: (f64, f64),
    center: (f64, f64),
    background: Option<&cairo::ImageSurface>,
) -> Option<gdk::Texture> {
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, key.2, key.2).ok()?;
    let context = cairo::Context::new(&surface).ok()?;
    if let Some(background) = background {
        context.set_source_surface(background, 0.0, 0.0).ok()?;
        context.paint().ok()?;
    }
    let scale = f64::from(key.2) / f64::from(ICON_TEXTURE_PX);
    context.scale(scale, scale);

    let (layout, ink) = fitted_emoji_layout(&context, emoji, preferred_size, bounds.0, bounds.1);
    context.set_source_rgb(1.0, 1.0, 1.0);
    context.move_to(
        center.0 - f64::from(ink.x() + ink.width() / 2),
        center.1 - f64::from(ink.y() + ink.height() / 2),
    );
    pangocairo::functions::show_layout(&context, &layout);

    Some(cache_icon_texture(key, texture_from_surface(&surface)?))
}

fn fitted_emoji_layout(
    context: &cairo::Context,
    emoji: &str,
    preferred_size: f64,
    max_width: f64,
    max_height: f64,
) -> (gtk::pango::Layout, gtk::pango::Rectangle) {
    let layout = pangocairo::functions::create_layout(context);
    let mut font = gtk::pango::FontDescription::from_string("emoji");
    font.set_absolute_size(preferred_size * f64::from(gtk::pango::SCALE));
    layout.set_font_description(Some(&font));
    layout.set_text(emoji);

    let (mut ink, _) = layout.pixel_extents();
    let width = f64::from(ink.width().max(1));
    let height = f64::from(ink.height().max(1));
    let scale = (max_width / width).min(max_height / height).min(1.0);
    if scale < 1.0 {
        font.set_absolute_size(preferred_size * scale * f64::from(gtk::pango::SCALE));
        layout.set_font_description(Some(&font));
        ink = layout.pixel_extents().0;
    }
    (layout, ink)
}

fn contrasting_foreground(color: &str) -> &'static str {
    let Some(rgb) = color.strip_prefix('#').filter(|hex| hex.len() >= 6) else {
        return "#f8fafc";
    };
    let Ok(red) = u8::from_str_radix(&rgb[0..2], 16) else {
        return "#f8fafc";
    };
    let Ok(green) = u8::from_str_radix(&rgb[2..4], 16) else {
        return "#f8fafc";
    };
    let Ok(blue) = u8::from_str_radix(&rgb[4..6], 16) else {
        return "#f8fafc";
    };
    let luminance = u32::from(red) * 299 + u32::from(green) * 587 + u32::from(blue) * 114;
    if luminance > 150_000 {
        "#172033"
    } else {
        "#f8fafc"
    }
}

fn svg_body(source: &str) -> Option<&str> {
    let start = source.find('>')? + 1;
    let end = source.rfind("</svg>")?;
    source.get(start..end)
}

fn svg_at_texture_size(source: String, texture_px: i32) -> String {
    source
        .replacen("width=\"24\"", &format!("width=\"{texture_px}\""), 1)
        .replacen("height=\"24\"", &format!("height=\"{texture_px}\""), 1)
}

fn texture_from_svg(
    cache_name: &str,
    color: &str,
    texture_px: i32,
    source: String,
) -> Option<gdk::Texture> {
    let key = (cache_name.to_owned(), color.to_owned(), texture_px);
    if let Some(texture) = cached_icon_texture(&key) {
        return Some(texture);
    }
    let surface = vector::surface(&source, texture_px)?;
    Some(cache_icon_texture(key, texture_from_surface(&surface)?))
}

fn texture_from_surface(surface: &cairo::ImageSurface) -> Option<gdk::Texture> {
    let mut bytes = None;
    surface
        .with_data(|data| bytes = Some(glib::Bytes::from_owned(data.to_vec())))
        .ok()?;
    let format = if cfg!(target_endian = "little") {
        gdk::MemoryFormat::B8g8r8a8Premultiplied
    } else {
        gdk::MemoryFormat::A8r8g8b8Premultiplied
    };
    Some(
        gdk::MemoryTexture::new(
            surface.width(),
            surface.height(),
            format,
            &bytes?,
            surface.stride() as usize,
        )
        .upcast(),
    )
}

type IconTextureKey = (String, String, i32);

struct CachedIconTexture {
    texture: gdk::Texture,
    generation: u64,
}

#[derive(Default)]
struct IconTextureCache {
    entries: HashMap<IconTextureKey, CachedIconTexture>,
    recent: VecDeque<(IconTextureKey, u64)>,
    generation: u64,
}

impl IconTextureCache {
    fn get(&mut self, key: &IconTextureKey) -> Option<gdk::Texture> {
        let cached = self.entries.get_mut(key)?;
        self.generation = self.generation.saturating_add(1);
        cached.generation = self.generation;
        self.recent.push_back((key.clone(), self.generation));
        let texture = cached.texture.clone();
        self.compact_recent();
        Some(texture)
    }

    fn insert(&mut self, key: IconTextureKey, texture: gdk::Texture) -> gdk::Texture {
        self.generation = self.generation.saturating_add(1);
        let generation = self.generation;
        self.entries.insert(
            key.clone(),
            CachedIconTexture {
                texture: texture.clone(),
                generation,
            },
        );
        self.recent.push_back((key, generation));
        while self.entries.len() > ICON_TEXTURE_CACHE_LIMIT {
            let Some((oldest_key, oldest_generation)) = self.recent.pop_front() else {
                break;
            };
            if self
                .entries
                .get(&oldest_key)
                .is_some_and(|cached| cached.generation == oldest_generation)
            {
                self.entries.remove(&oldest_key);
            }
        }
        self.compact_recent();
        texture
    }

    #[cfg(test)]
    fn clear(&mut self) {
        self.entries.clear();
        self.recent.clear();
        self.generation = 0;
    }

    fn compact_recent(&mut self) {
        if self.recent.len() > ICON_TEXTURE_CACHE_LIMIT * 4 {
            self.recent.retain(|(key, generation)| {
                self.entries
                    .get(key)
                    .is_some_and(|cached| cached.generation == *generation)
            });
        }
    }
}

fn cached_icon_texture(key: &IconTextureKey) -> Option<gdk::Texture> {
    ICON_TEXTURES.with(|textures| textures.borrow_mut().get(key))
}

fn cache_icon_texture(key: IconTextureKey, texture: gdk::Texture) -> gdk::Texture {
    ICON_TEXTURES.with(|textures| textures.borrow_mut().insert(key, texture))
}

fn recolor_icon_source(source: &str, color: &str) -> String {
    source
        .replace("#8bc9eb", color)
        .replace("#22d3ee", color)
        .replace("#2e3436", color)
}

fn write_if_changed(path: &Path, contents: &[u8]) -> io::Result<()> {
    let is_current = fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_file() && metadata.len() == contents.len() as u64)
        .unwrap_or(false);

    if !is_current {
        crate::storage::atomic_write(path, contents)?;
    }

    Ok(())
}

#[expect(
    unsafe_code,
    reason = "Fontconfig exposes application-font registration only through its C FFI"
)]
fn register_application_fonts(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: This no-argument Fontconfig call returns a borrowed process-global
    // configuration. We check it for null before passing it to any other FFI call.
    let config = unsafe { fontconfig_sys::FcConfigGetCurrent() };
    if config.is_null() {
        return Err("Fontconfig did not provide a current configuration".into());
    }

    for path in paths {
        let path = CString::new(path.as_os_str().as_bytes())?;

        // SAFETY: `config` was checked above. `path` is a valid, NUL-terminated C string
        // that remains alive for the call, and Fontconfig copies rather than retains it.
        let registered =
            unsafe { fontconfig_sys::FcConfigAppFontAddFile(config, path.as_ptr().cast()) };
        if registered == 0 {
            return Err("Fontconfig could not register a bundled font".into());
        }
    }

    // SAFETY: `config` is the same checked process-global configuration. Registration
    // runs during single-threaded startup before GTK/Pango creates the application's map.
    let rebuilt = unsafe { fontconfig_sys::FcConfigBuildFonts(config) };
    if rebuilt == 0 {
        return Err("Fontconfig could not rebuild the application font set".into());
    }

    Ok(())
}

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::{
    custom_colored_icon_paintable, folder_decoration_paintable, primary_icon_paintable,
};
