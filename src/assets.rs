// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::CString,
    fs, io,
    io::Cursor,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use gtk::{gdk, gdk::prelude::GdkCairoContextExt, gio, glib};

pub mod icons {
    pub const ARROW_DOWN: &str = "strata-arrow-down";
    pub const ARROW_DOWN_WIDE_NARROW: &str = "strata-arrow-down-wide-narrow";
    pub const ARROW_LEFT: &str = "strata-arrow-left";
    pub const ARROW_RIGHT: &str = "strata-arrow-right";
    pub const ARROW_UP: &str = "strata-arrow-up";
    pub const ARROW_UP_NARROW_WIDE: &str = "strata-arrow-up-narrow-wide";
    pub const CHECK: &str = "strata-check";
    pub const CHECK_ON_PRIMARY: &str = "strata-check-on-primary";
    pub const CHEVRON_DOWN: &str = "strata-chevron-down";
    pub const CHEVRON_RIGHT: &str = "strata-chevron-right";
    pub const CLIPBOARD_PASTE: &str = "strata-clipboard-paste";
    pub const COPY: &str = "strata-copy";
    pub const CORNER_DOWN_LEFT: &str = "strata-corner-down-left";
    pub const DOCUMENTS: &str = "strata-file-text";
    pub const DOWNLOADS: &str = "strata-download";
    pub const EJECT: &str = "strata-eject";
    pub const EYE: &str = "strata-eye";
    pub const EYE_OFF: &str = "strata-eye-off";
    pub const EXTERNAL_LINK: &str = "strata-external-link";
    pub const FILE_ARCHIVE: &str = "strata-file-archive";
    pub const FILE_CODE: &str = "strata-file-code";
    pub const FILE_PLUS: &str = "strata-file-plus";
    pub const FOLDER: &str = "strata-folder";
    pub const FOLDER_PLUS: &str = "strata-folder-plus";
    pub const HARD_DRIVE: &str = "strata-hard-drive";
    pub const INFO: &str = "strata-info";
    pub const FUNNEL: &str = "strata-funnel";
    pub const GRID: &str = "strata-grid";
    pub const HOME: &str = "strata-house";
    pub const LIST: &str = "strata-list";
    pub const LIST_ACTIVE: &str = "strata-list-active";
    pub const LIST_CHECKS: &str = "strata-list-checks";
    pub const LIST_TREE: &str = "strata-list-tree";
    pub const KEY: &str = "strata-key";
    pub const KEYBOARD: &str = "strata-keyboard";
    pub const MONITOR: &str = "strata-monitor";
    pub const NETWORK: &str = "strata-network";
    pub const PALETTE: &str = "strata-palette";
    pub const PANEL_LEFT: &str = "strata-panel-left-symbolic";
    pub const PAUSE: &str = "strata-pause";
    pub const PENCIL: &str = "strata-pencil";
    pub const PIN: &str = "strata-pin";
    pub const PLAY: &str = "strata-play";
    pub const PLUS: &str = "strata-plus";
    pub const PRINTER: &str = "strata-printer";
    pub const PICTURES: &str = "strata-image";
    pub const ROWS: &str = "strata-rows";
    pub const SCISSORS: &str = "strata-scissors";
    pub const SEARCH: &str = "strata-search";
    pub const SETTINGS: &str = "strata-settings";
    pub const SETTINGS_2: &str = "strata-settings-2";
    pub const REFRESH: &str = "strata-refresh";
    pub const SLIDERS: &str = "strata-sliders-horizontal";
    pub const TERMINAL: &str = "strata-terminal";
    pub const TRASH: &str = "strata-trash";
    pub const TRIANGLE_ALERT: &str = "strata-triangle-alert";
    pub const UNPLUG: &str = "strata-unplug";
    pub const VIDEOS: &str = "strata-video";
    pub const VOLUME_2: &str = "strata-volume-2";
    pub const VOLUME_X: &str = "strata-volume-x";
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
const JETBRAINS_MONO: &[u8] = include_bytes!("../data/fonts/JetBrainsMono[wght].ttf");

struct PrimaryIcon {
    image: glib::WeakRef<gtk::Image>,
    name: String,
}

thread_local! {
    static PRIMARY_ICON_COLOR: RefCell<String> = RefCell::new("#8bc9eb".to_owned());
    static PRIMARY_ICONS: RefCell<Vec<PrimaryIcon>> = const { RefCell::new(Vec::new()) };
    static DANGER_ICON_COLOR: RefCell<String> = RefCell::new("#e5484d".to_owned());
    static DANGER_ICONS: RefCell<Vec<PrimaryIcon>> = const { RefCell::new(Vec::new()) };
    static ICON_TEXTURES: RefCell<HashMap<(String, String), gdk::Texture>> = RefCell::new(HashMap::new());
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
    image.set_pixel_size(pixel_size);
    set_primary_icon(&image, name);
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
    if let Some(texture) = folder_decoration_texture(decoration, color) {
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

pub fn primary_icon_color() -> String {
    PRIMARY_ICON_COLOR.with(|color| color.borrow().clone())
}

pub fn danger_icon(name: &str, pixel_size: i32) -> gtk::Image {
    let image = gtk::Image::new();
    image.set_pixel_size(pixel_size);
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
    if let Some(texture) = primary_icon_texture(name, color) {
        image.set_paintable(Some(&texture));
    } else {
        image.set_icon_name(Some(name));
    }
}

fn primary_icon_texture(name: &str, color: &str) -> Option<gdk::Texture> {
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
    texture_from_svg(name, color, svg_at_texture_size(source))
}

fn folder_decoration_texture(decoration: &str, color: &str) -> Option<gdk::Texture> {
    let folder_data = gio::resources_lookup_data(
        "/io/github/lgse/Strata/icons/scalable/actions/strata-folder.svg",
        gio::ResourceLookupFlags::NONE,
    )
    .ok()?;
    let folder = std::str::from_utf8(folder_data.as_ref()).ok()?;
    let mut source = svg_at_texture_size(recolor_icon_source(folder, color)).replacen(
        "fill=\"none\"",
        &format!("fill=\"{color}\" fill-opacity=\"0.92\""),
        1,
    );
    if let Some(emoji) = icons::custom_emoji(decoration) {
        return folder_emoji_texture(&source, emoji, color);
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
    texture_from_svg(&format!("folder-decoration:{decoration}"), color, source)
}

fn folder_emoji_texture(folder_source: &str, emoji: &str, color: &str) -> Option<gdk::Texture> {
    let key = (format!("folder-emoji:{emoji}"), color.to_owned());
    if let Some(texture) = cached_icon_texture(&key) {
        return Some(texture);
    }
    let folder =
        gdk_pixbuf::Pixbuf::from_read(Cursor::new(folder_source.as_bytes().to_vec())).ok()?;
    render_emoji_texture(key, emoji, 52.0, (44.0, 44.0), (48.0, 56.0), Some(&folder))
}

fn emoji_texture(emoji: &str) -> Option<gdk::Texture> {
    let key = (format!("emoji:{emoji}"), "native".to_owned());
    if let Some(texture) = cached_icon_texture(&key) {
        return Some(texture);
    }
    render_emoji_texture(key, emoji, 78.0, (82.0, 82.0), (48.0, 48.0), None)
}

fn render_emoji_texture(
    key: (String, String),
    emoji: &str,
    preferred_size: f64,
    bounds: (f64, f64),
    center: (f64, f64),
    background: Option<&gdk_pixbuf::Pixbuf>,
) -> Option<gdk::Texture> {
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 96, 96).ok()?;
    let context = cairo::Context::new(&surface).ok()?;
    if let Some(background) = background {
        context.set_source_pixbuf(background, 0.0, 0.0);
        context.paint().ok()?;
    }

    let (layout, ink) = fitted_emoji_layout(&context, emoji, preferred_size, bounds.0, bounds.1);
    context.set_source_rgb(1.0, 1.0, 1.0);
    context.move_to(
        center.0 - f64::from(ink.x() + ink.width() / 2),
        center.1 - f64::from(ink.y() + ink.height() / 2),
    );
    pangocairo::functions::show_layout(&context, &layout);

    let mut png = Vec::new();
    surface.write_to_png(&mut png).ok()?;
    let pixbuf = gdk_pixbuf::Pixbuf::from_read(Cursor::new(png)).ok()?;
    Some(cache_icon_texture(key, gdk::Texture::for_pixbuf(&pixbuf)))
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

fn svg_at_texture_size(source: String) -> String {
    source
        .replacen("width=\"24\"", &format!("width=\"{ICON_TEXTURE_PX}\""), 1)
        .replacen("height=\"24\"", &format!("height=\"{ICON_TEXTURE_PX}\""), 1)
}

fn texture_from_svg(cache_name: &str, color: &str, source: String) -> Option<gdk::Texture> {
    let key = (cache_name.to_owned(), color.to_owned());
    if let Some(texture) = cached_icon_texture(&key) {
        return Some(texture);
    }
    let pixbuf = gdk_pixbuf::Pixbuf::from_read(Cursor::new(source.into_bytes())).ok()?;
    Some(cache_icon_texture(key, gdk::Texture::for_pixbuf(&pixbuf)))
}

fn cached_icon_texture(key: &(String, String)) -> Option<gdk::Texture> {
    ICON_TEXTURES.with(|textures| textures.borrow().get(key).cloned())
}

fn cache_icon_texture(key: (String, String), texture: gdk::Texture) -> gdk::Texture {
    ICON_TEXTURES.with(|textures| {
        let mut textures = textures.borrow_mut();
        if textures.len() >= ICON_TEXTURE_CACHE_LIMIT {
            textures.clear();
        }
        textures.insert(key, texture.clone());
    });
    texture
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
