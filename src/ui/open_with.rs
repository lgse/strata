// SPDX-License-Identifier: MIT

use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use super::{
    blur::BlurBin,
    browser::{dismiss_modal_layer, modal_layer, show_error_dialog},
    controls::modal_layout,
    recent_apps,
};

pub(super) fn categorized_apps(
    content_type: &str,
    requires_uris: bool,
) -> (Vec<gio::AppInfo>, Vec<gio::AppInfo>) {
    let default = gio::AppInfo::default_for_type(content_type, requires_uris);
    let recommended = filter_apps(
        gio::AppInfo::all_for_type(content_type),
        default,
        requires_uris,
    );
    let other = filter_other_apps(gio::AppInfo::all(), &recommended, requires_uris);
    (recommended, other)
}

// Non-native GVfs files can still provide FUSE paths for %f/%F handlers.
pub(super) fn requires_uri_handlers(files: &[gio::File]) -> bool {
    files
        .iter()
        .any(|file| path_requires_uri_handlers(file.path().as_deref()))
}

fn path_requires_uri_handlers(path: Option<&std::path::Path>) -> bool {
    path.is_none()
}

fn app_is_candidate(app: &gio::AppInfo, requires_uris: bool) -> bool {
    app.should_show() && (!requires_uris || app.supports_uris())
}

fn filter_apps(
    apps: Vec<gio::AppInfo>,
    default: Option<gio::AppInfo>,
    requires_uris: bool,
) -> Vec<gio::AppInfo> {
    let mut apps = apps
        .into_iter()
        .filter(|app| app_is_candidate(app, requires_uris))
        .collect::<Vec<_>>();
    apps.sort_by_cached_key(|app| app.display_name().to_lowercase());
    let mut unique = Vec::with_capacity(apps.len());
    if let Some(default) = default.filter(|app| !requires_uris || app.supports_uris()) {
        unique.push(default);
    }
    for app in apps {
        if !unique.iter().any(|existing| existing.equal(&app)) {
            unique.push(app);
        }
    }
    unique
}

pub(super) fn filter_other_apps(
    apps: Vec<gio::AppInfo>,
    recommended: &[gio::AppInfo],
    requires_uris: bool,
) -> Vec<gio::AppInfo> {
    let mut apps = apps
        .into_iter()
        .filter(|app| {
            app_is_candidate(app, requires_uris) && !recommended.iter().any(|rec| rec.equal(app))
        })
        .collect::<Vec<_>>();
    apps.sort_by_cached_key(|app| app.display_name().to_lowercase());
    let mut unique: Vec<gio::AppInfo> = Vec::with_capacity(apps.len());
    for app in apps {
        if !unique.iter().any(|existing| existing.equal(&app)) {
            unique.push(app);
        }
    }
    unique
}

fn app_ids(apps: &[gio::AppInfo]) -> HashSet<String> {
    apps.iter()
        .filter_map(|app| app.id().map(|id| id.to_string()))
        .collect()
}

// For mixed types, only promote handlers shared by every type. Single-type
// history may also promote applications explicitly chosen from Other.
fn split_recent(
    content_types: &[String],
    recommended: &mut Vec<gio::AppInfo>,
    other: &mut Vec<gio::AppInfo>,
) -> Vec<gio::AppInfo> {
    let ordered = recent_apps::load().recent_ids(content_types);
    if ordered.is_empty() {
        return Vec::new();
    }
    let mut eligible = app_ids(recommended);
    if content_types.len() <= 1 {
        eligible.extend(app_ids(other));
    }
    let mut wanted = recent_apps::select_recent(&ordered, &eligible);
    wanted.truncate(recent_apps::MAX_PER_TYPE);
    if wanted.is_empty() {
        return Vec::new();
    }
    let mut by_id: HashMap<String, gio::AppInfo> = HashMap::new();
    for app in recommended.iter().chain(other.iter()) {
        if let Some(id) = app.id() {
            by_id.entry(id.to_string()).or_insert_with(|| app.clone());
        }
    }
    let recent: Vec<gio::AppInfo> = wanted.iter().filter_map(|id| by_id.remove(id)).collect();
    if recent.is_empty() {
        return Vec::new();
    }
    let recent_ids: HashSet<String> = wanted.into_iter().collect();
    for apps in [recommended, other] {
        apps.retain(|app| app.id().is_none_or(|id| !recent_ids.contains(id.as_str())));
    }
    recent
}

pub(super) struct Applications {
    pub(super) recommended: Vec<gio::AppInfo>,
    pub(super) other: Vec<gio::AppInfo>,
    pub(super) default: Option<gio::AppInfo>,
    pub(super) content_types: Vec<String>,
}

impl Applications {
    pub(super) fn unavailable_reason(&self) -> Option<&'static str> {
        if !self.recommended.is_empty() || !self.other.is_empty() {
            None
        } else if self.content_types.len() > 1 {
            Some("No application can open all selected file types")
        } else {
            Some("No compatible applications were found")
        }
    }
}

/// Looks up the type of every file, then the handlers they all share. An
/// unreadable file or broken link fails the whole lookup rather than letting
/// another item decide. `None` means `current` stopped holding first.
pub(super) async fn resolve(
    files: &[gio::File],
    current: impl Fn() -> bool,
) -> Option<Result<Applications, &'static str>> {
    let mut content_types = Vec::<String>::new();
    for file in files {
        if !current() {
            return None;
        }
        let info = file
            .query_info_future(
                "standard::type,standard::content-type",
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await;
        if !current() {
            return None;
        }
        let Ok(info) = info else {
            return Some(Err("Unable to read the selected file type"));
        };
        if info.file_type() == gio::FileType::SymbolicLink {
            return Some(Err(
                "Broken symbolic links cannot be opened with an application",
            ));
        }
        let Some(next_type) = info.content_type().map(|value| value.to_string()) else {
            return Some(Err("Unable to determine the selected file type"));
        };
        if !content_types
            .iter()
            .any(|value| gio::content_type_equals(value, &next_type))
        {
            content_types.push(next_type);
        }
    }
    if !current() {
        return None;
    }
    let requires_uris = requires_uri_handlers(files);
    let (recommended, other, default) = common_applications(&content_types, requires_uris);
    Some(Ok(Applications {
        recommended,
        other,
        default,
        content_types,
    }))
}

fn common_applications(
    content_types: &[String],
    requires_uris: bool,
) -> (Vec<gio::AppInfo>, Vec<gio::AppInfo>, Option<gio::AppInfo>) {
    let Some(first) = content_types.first() else {
        return (vec![], vec![], None);
    };
    let (mut recommended, _) = categorized_apps(first, requires_uris);
    let mut default = gio::AppInfo::default_for_type(first, requires_uris);
    for content_type in &content_types[1..] {
        let (next_rec, _) = categorized_apps(content_type, requires_uris);
        recommended.retain(|app| next_rec.iter().any(|candidate| candidate.equal(app)));
        let next_default = gio::AppInfo::default_for_type(content_type, requires_uris);
        default = default.filter(|app| next_default.as_ref().is_some_and(|next| next.equal(app)));
    }
    let other = filter_other_apps(gio::AppInfo::all(), &recommended, requires_uris);
    (recommended, other, default)
}

pub(super) fn launch(
    app: &gio::AppInfo,
    files: &[gio::File],
    content_types: &[String],
    context: Option<&impl IsA<gio::AppLaunchContext>>,
) -> Result<(), glib::Error> {
    launch_with_recent_registration(
        app,
        app,
        files,
        content_types,
        context,
        register_recent_file,
    )
}

pub(in crate::ui) fn launch_at(
    app: &gio::AppInfo,
    file: &gio::File,
    content_type: &str,
    position: Option<std::time::Duration>,
    context: Option<&impl IsA<gio::AppLaunchContext>>,
) -> Result<(), glib::Error> {
    let files = std::slice::from_ref(file);
    let content_types = [content_type.to_owned()];
    let timed = position
        .and_then(|position| {
            let exec = app.commandline()?;
            command_line_at(&exec.to_string_lossy(), position.as_secs_f64())
        })
        .and_then(|command_line| {
            let mut flags = if app.supports_uris() {
                gio::AppInfoCreateFlags::SUPPORTS_URIS
            } else {
                gio::AppInfoCreateFlags::NONE
            };
            if app
                .downcast_ref::<gio_unix::DesktopAppInfo>()
                .is_some_and(|desktop| desktop.boolean("StartupNotify"))
            {
                flags |= gio::AppInfoCreateFlags::SUPPORTS_STARTUP_NOTIFICATION;
            }
            gio::AppInfo::create_from_commandline(command_line, Some(app.name().as_str()), flags)
                .ok()
        });
    if let Some(timed) = timed
        && launch_with_recent_registration(
            app,
            &timed,
            files,
            &content_types,
            context,
            register_recent_file,
        )
        .is_ok()
    {
        return Ok(());
    }
    launch(app, files, &content_types, context)
}

/// Field codes and Flatpak forwarding markers must stay unquoted and mark
/// where the file arguments go.
fn is_placeholder(token: &str) -> bool {
    matches!(
        token,
        "%f" | "%F" | "%u" | "%U" | "%i" | "%c" | "%k" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m"
    ) || token.starts_with("@@")
}

fn basename(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

fn player_name(tokens: &[String]) -> Option<String> {
    let mut command = tokens.iter().peekable();
    let mut program = command.next()?;
    while basename(program) == "env" {
        program = command.find(|token| !token.contains('=') && !token.starts_with('-'))?;
    }
    if basename(program) == "flatpak" {
        if let Some(name) = tokens
            .iter()
            .find_map(|token| token.strip_prefix("--command="))
        {
            return Some(basename(name).to_owned());
        }
        return tokens
            .iter()
            .skip(1)
            .find(|token| {
                token.contains('.')
                    && !token.starts_with('-')
                    && !token.contains('/')
                    && !is_placeholder(token)
            })
            .cloned();
    }
    Some(basename(program).to_owned())
}

fn start_arguments(player: &str, seconds: f64) -> Option<Vec<String>> {
    let seconds = format!("{seconds:.3}");
    Some(match player {
        "mpv" | "io.mpv.Mpv" => vec![format!("--start={seconds}")],
        "vlc" | "cvlc" | "qvlc" | "nvlc" | "org.videolan.VLC" => {
            vec![format!("--start-time={seconds}")]
        }
        "celluloid" | "io.github.celluloid_player.Celluloid" => {
            vec![format!("--mpv-start={seconds}")]
        }
        "mplayer" => vec!["-ss".to_owned(), seconds],
        _ => return None,
    })
}

pub(super) fn command_line_at(exec: &str, seconds: f64) -> Option<String> {
    let tokens: Vec<String> = glib::shell_parse_argv(exec)
        .ok()?
        .into_iter()
        .map(|token| token.to_string_lossy().into_owned())
        .collect();
    let arguments = start_arguments(&player_name(&tokens)?, seconds)?;
    let insert_at = tokens
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, token)| is_placeholder(token) || *token == "--")
        .map_or(tokens.len(), |(index, _)| index);
    let mut tokens = tokens;
    tokens.splice(insert_at..insert_at, arguments);
    Some(
        tokens
            .iter()
            .map(|token| {
                let plain = token
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-./=:@%+,".contains(&byte));
                if plain && !token.is_empty() || is_placeholder(token) {
                    token.clone()
                } else {
                    glib::shell_quote(token).to_string_lossy().into_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn launch_with_recent_registration(
    app: &gio::AppInfo,
    launcher: &gio::AppInfo,
    files: &[gio::File],
    content_types: &[String],
    context: Option<&impl IsA<gio::AppLaunchContext>>,
    register_recent: impl Fn(&gio::File) -> bool + 'static,
) -> Result<(), glib::Error> {
    // GIO drops files without a local path when expanding %f/%F.
    if !launcher.supports_uris() && requires_uri_handlers(files) {
        return Err(glib::Error::new(
            gio::IOErrorEnum::NotSupported,
            &crate::i18n::tr("This application cannot open files at this location"),
        ));
    }
    launcher.launch(files, context)?;
    if let Some(app_id) = app.id() {
        let known_ids = gio::AppInfo::all()
            .into_iter()
            .filter_map(|candidate| candidate.id().map(|id| id.to_string()))
            .collect::<HashSet<_>>();
        recent_apps::record(content_types, app_id.as_str(), &known_ids);
    }
    let candidates: Vec<gio::File> = files
        .iter()
        .filter(|file| !file.has_uri_scheme("recent"))
        .cloned()
        .collect();
    if candidates.is_empty() {
        return Ok(());
    }
    // Advisory history updates must not block launching on slow mounts.
    glib::MainContext::default().spawn_local(async move {
        for file in candidates {
            let file_type = file
                .query_info_future(
                    "standard::type",
                    gio::FileQueryInfoFlags::NONE,
                    glib::Priority::DEFAULT,
                )
                .await
                .map(|info| info.file_type());
            // A successful launch remains worth recording when metadata is unavailable.
            if !matches!(
                file_type,
                Ok(gio::FileType::Directory | gio::FileType::Mountable)
            ) {
                register_recent(&file);
            }
        }
    });
    Ok(())
}

fn register_recent_file(file: &gio::File) -> bool {
    let manager = gtk::RecentManager::default();
    let uri = file.uri();
    let added = manager.add_item(uri.as_str());
    if !added {
        tracing::debug!(uri = %uri, "unable to record file in Recent history");
    }
    added
}

fn application_icon(app: &gio::AppInfo, display: &gtk::gdk::Display) -> gtk::Image {
    let icon = app.icon().and_then(|icon| {
        if let Some(file_icon) = icon.downcast_ref::<gio::FileIcon>() {
            return gtk::gdk::Texture::from_file(&file_icon.file())
                .ok()
                .map(|texture| gtk::Image::from_paintable(Some(&texture)));
        }
        gtk::IconTheme::for_display(display)
            .has_gicon(&icon)
            .then(|| gtk::Image::from_gicon(&icon))
    });
    icon.unwrap_or_else(|| crate::assets::primary_icon(crate::assets::icons::FILE_CODE, 24))
}

fn install_list_tab_navigation(
    content: &gtk::Box,
    search_entry: &gtk::SearchEntry,
    list: &gtk::ListBox,
    always_use: &gtk::CheckButton,
    close: &gtk::Button,
    cancel: &gtk::Button,
) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let search = search_entry.downgrade();
    let list = list.downgrade();
    let always = always_use.downgrade();
    let close = close.downgrade();
    let cancel = cancel.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if !matches!(key, gtk::gdk::Key::Tab | gtk::gdk::Key::ISO_Left_Tab)
            || modifiers.intersects(
                gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK,
            )
        {
            return glib::Propagation::Proceed;
        }
        let (Some(search), Some(list), Some(always), Some(close), Some(cancel)) = (
            search.upgrade(),
            list.upgrade(),
            always.upgrade(),
            close.upgrade(),
            cancel.upgrade(),
        ) else {
            return glib::Propagation::Proceed;
        };
        let Some(focus) = list.root().and_then(|root| root.focus()) else {
            return glib::Propagation::Proceed;
        };
        let backward = key == gtk::gdk::Key::ISO_Left_Tab
            || modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        let focus_selected_row = || {
            if list.is_visible() {
                list.selected_row().is_some_and(|row| row.grab_focus())
            } else {
                false
            }
        };
        // A hidden widget still accepts focus grabs, so gate on visibility like the list.
        let focus_always = || always.is_visible() && always.grab_focus();
        let moved = if focus == search || focus.is_ancestor(&search) {
            if backward {
                close.grab_focus()
            } else if focus_selected_row() || focus_always() {
                true
            } else {
                cancel.grab_focus()
            }
        } else if focus == list || focus.is_ancestor(&list) {
            if backward {
                if search.is_visible() {
                    search.grab_focus()
                } else {
                    close.grab_focus()
                }
            } else {
                focus_always() || cancel.grab_focus()
            }
        } else if backward && focus == cancel {
            if focus_always() || focus_selected_row() {
                true
            } else if search.is_visible() {
                search.grab_focus()
            } else {
                close.grab_focus()
            }
        } else if !backward && focus == close {
            if search.is_visible() {
                search.grab_focus()
            } else if focus_selected_row() || focus_always() {
                true
            } else {
                cancel.grab_focus()
            }
        } else {
            false
        };
        if moved {
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    content.add_controller(keys);
}

struct AppEntry {
    app: gio::AppInfo,
    row: gtk::ListBoxRow,
    section: AppSection,
    haystack: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppSection {
    Recent,
    Recommended,
    Other,
}

#[derive(Clone, Copy)]
pub(super) enum OpenWithContext {
    Explicit,
    ActivationFallback,
}

impl OpenWithContext {
    fn empty_message(self) -> &'static str {
        match self {
            Self::Explicit => "No compatible applications were found.",
            Self::ActivationFallback => "No application is registered for this file",
        }
    }
}

fn create_section_header(title: &str) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("open-with-heading-row");
    row.set_activatable(false);
    row.set_selectable(false);
    row.set_focusable(false);
    row.set_can_focus(false);

    let label = gtk::Label::new(Some(&crate::i18n::tr(title)));
    label.add_css_class("open-with-heading");
    label.set_xalign(0.0);
    row.set_child(Some(&label));
    row
}

fn create_app_row(app: &gio::AppInfo, display: &gtk::gdk::Display) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("open-with-row");
    row.update_property(&[gtk::accessible::Property::Label(&app.display_name())]);
    if let Some(description) = app.description() {
        row.update_property(&[gtk::accessible::Property::Description(&description)]);
    }
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon = application_icon(app, display);
    icon.set_pixel_size(22);
    icon.add_css_class("open-with-icon");
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 0);
    labels.set_valign(gtk::Align::Center);
    let name = gtk::Label::new(Some(&app.display_name()));
    name.add_css_class("open-with-name");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let description_text = app.description().map(|value| value.to_string());
    let description = gtk::Label::new(description_text.as_deref());
    description.add_css_class("open-with-description");
    description.set_xalign(0.0);
    description.set_ellipsize(gtk::pango::EllipsizeMode::End);
    labels.append(&name);
    if description_text
        .as_deref()
        .is_some_and(|value| !value.is_empty() && value != app.display_name())
    {
        labels.append(&description);
    }
    content.append(&icon);
    content.append(&labels);
    row.set_child(Some(&content));
    row
}

pub(super) fn show(
    parent: &impl IsA<gtk::Widget>,
    files: Vec<gio::File>,
    content_types: Vec<String>,
    mut recommended_apps: Vec<gio::AppInfo>,
    mut other_apps: Vec<gio::AppInfo>,
    context: OpenWithContext,
    on_close: Rc<dyn Fn()>,
) {
    let Some(window_overlay) = parent
        .root()
        .and_downcast::<gtk::Window>()
        .and_then(|window| window.child())
        .and_downcast::<gtk::Overlay>()
    else {
        return;
    };
    let blurred_root = window_overlay.child().and_downcast::<BlurBin>();
    if let Some(root) = blurred_root.as_ref() {
        root.set_blurred(true);
    }

    let subtitle = crate::i18n::tr(if files.len() == 1 {
        "Choose an application to open this item"
    } else {
        "Choose an application to open these items"
    });
    let layout = modal_layout(
        crate::assets::icons::EXTERNAL_LINK,
        &crate::i18n::tr("Open With"),
        &subtitle,
        &crate::i18n::tr("Open"),
    );
    layout.content.add_css_class("open-with-dialog");
    layout.content.set_size_request(480, 460);
    layout.body.set_vexpand(true);

    let search_entry = gtk::SearchEntry::new();
    search_entry.add_css_class("open-with-search");
    search_entry.set_placeholder_text(Some(&crate::i18n::tr("Search applications…")));
    layout.body.append(&search_entry);

    let list = gtk::ListBox::new();
    list.add_css_class("open-with-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(false);
    list.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr(
        "Applications",
    ))]);

    let always_use =
        super::controls::form_check_button(&crate::i18n::tr(if content_types.len() > 1 {
            "Always use for these file types"
        } else {
            "Always use for this file type"
        }));
    always_use.set_visible(false);
    layout.actions.prepend(&always_use);

    install_list_tab_navigation(
        &layout.content,
        &search_entry,
        &list,
        &always_use,
        &layout.close,
        &layout.cancel,
    );

    let mut entries = Vec::new();

    let mut recent_heading_row = None;
    let recent = split_recent(&content_types, &mut recommended_apps, &mut other_apps);
    if !recent.is_empty() {
        let heading = create_section_header("Recently Used");
        list.append(&heading);
        recent_heading_row = Some(heading);

        for app in &recent {
            let row = create_app_row(app, &list.display());
            let haystack = format!(
                "{} {} {} {} {}",
                app.display_name(),
                app.name(),
                app.description().unwrap_or_default(),
                app.executable().to_string_lossy(),
                app.id().unwrap_or_default(),
            )
            .to_lowercase();
            list.append(&row);
            entries.push(AppEntry {
                app: app.clone(),
                row,
                section: AppSection::Recent,
                haystack,
            });
        }
    }

    let mut recommended_heading_row = None;
    if !recommended_apps.is_empty() {
        let heading = create_section_header("Recommended Applications");
        list.append(&heading);
        recommended_heading_row = Some(heading);

        for app in &recommended_apps {
            let row = create_app_row(app, &list.display());
            let haystack = format!(
                "{} {} {} {} {}",
                app.display_name(),
                app.name(),
                app.description().unwrap_or_default(),
                app.executable().to_string_lossy(),
                app.id().unwrap_or_default(),
            )
            .to_lowercase();
            list.append(&row);
            entries.push(AppEntry {
                app: app.clone(),
                row,
                section: AppSection::Recommended,
                haystack,
            });
        }
    }

    let mut other_heading_row = None;
    if !other_apps.is_empty() {
        let heading = create_section_header("Other Applications");
        list.append(&heading);
        other_heading_row = Some(heading);

        for app in &other_apps {
            let row = create_app_row(app, &list.display());
            let haystack = format!(
                "{} {} {} {} {}",
                app.display_name(),
                app.name(),
                app.description().unwrap_or_default(),
                app.executable().to_string_lossy(),
                app.id().unwrap_or_default(),
            )
            .to_lowercase();
            list.append(&row);
            entries.push(AppEntry {
                app: app.clone(),
                row,
                section: AppSection::Other,
                haystack,
            });
        }
    }

    if let Some(first_entry) = entries.first() {
        list.select_row(Some(&first_entry.row));
    }

    let list_scroll = gtk::ScrolledWindow::new();
    list_scroll.add_css_class("open-with-scroll");
    list_scroll.set_child(Some(&list));
    list_scroll.set_propagate_natural_height(false);
    list_scroll.set_vexpand(true);
    list_scroll.set_hexpand(true);
    list_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
    layout.body.append(&list_scroll);

    let empty_search = gtk::Label::new(Some(&crate::i18n::tr(
        "No matching applications were found.",
    )));
    empty_search.add_css_class("open-with-empty");
    empty_search.set_wrap(true);
    empty_search.set_xalign(0.5);
    empty_search.set_halign(gtk::Align::Center);
    empty_search.set_valign(gtk::Align::Center);
    empty_search.set_vexpand(true);
    empty_search.set_visible(false);
    layout.body.append(&empty_search);

    let has_apps = !entries.is_empty();
    always_use.set_visible(has_apps && !content_types.is_empty());
    if !has_apps {
        search_entry.set_visible(false);
        list_scroll.set_visible(false);
        let empty = gtk::Label::new(Some(&crate::i18n::tr(context.empty_message())));
        empty.add_css_class("open-with-empty");
        empty.set_wrap(true);
        empty.set_xalign(0.5);
        empty.set_halign(gtk::Align::Center);
        empty.set_valign(gtk::Align::Center);
        empty.set_vexpand(true);
        layout.body.append(&empty);
        layout.confirm.set_sensitive(false);
    }

    let search_keys = gtk::EventControllerKey::new();
    search_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let list_for_search = list.downgrade();
    let scroll_for_search = list_scroll.downgrade();
    search_keys.connect_key_pressed(move |_, key, _, modifiers| {
        if !matches!(key, gtk::gdk::Key::Down | gtk::gdk::Key::Up)
            || modifiers.intersects(
                gtk::gdk::ModifierType::CONTROL_MASK
                    | gtk::gdk::ModifierType::ALT_MASK
                    | gtk::gdk::ModifierType::SUPER_MASK
                    | gtk::gdk::ModifierType::SHIFT_MASK,
            )
        {
            return glib::Propagation::Proceed;
        }
        if let (Some(list), Some(scroll)) = (list_for_search.upgrade(), scroll_for_search.upgrade())
            && let Some(selected) = list.selected_row()
        {
            let mut row = selected.clone();
            let mut sibling = if key == gtk::gdk::Key::Down {
                selected.next_sibling()
            } else {
                selected.prev_sibling()
            };
            while let Some(candidate) = sibling {
                if candidate.is_visible()
                    && let Some(candidate_row) = candidate.downcast_ref::<gtk::ListBoxRow>()
                    && candidate_row.is_selectable()
                {
                    row = candidate_row.clone();
                    break;
                }
                sibling = if key == gtk::gdk::Key::Down {
                    candidate.next_sibling()
                } else {
                    candidate.prev_sibling()
                };
            }
            list.select_row(Some(&row));
            if let Some(bounds) = row.compute_bounds(&list) {
                let adjustment = scroll.vadjustment();
                let top = f64::from(bounds.y());
                let bottom = top + f64::from(bounds.height());
                if top < adjustment.value() {
                    adjustment.set_value(top);
                } else if bottom > adjustment.value() + adjustment.page_size() {
                    adjustment.set_value(bottom - adjustment.page_size());
                }
            }
        }
        glib::Propagation::Stop
    });
    search_entry.add_controller(search_keys);

    let list_keys = gtk::EventControllerKey::new();
    let search_for_list = search_entry.downgrade();
    let list_weak = list.downgrade();
    list_keys.connect_key_pressed(move |_, key, _, modifiers| {
        let (Some(list), Some(search)) = (list_weak.upgrade(), search_for_list.upgrade()) else {
            return glib::Propagation::Proceed;
        };

        if key == gtk::gdk::Key::Up
            && let Some(selected) = list.selected_row()
        {
            let mut prev = selected.prev_sibling();
            let mut has_prev_selectable = false;
            while let Some(w) = prev {
                if w.is_visible() && w.can_focus() {
                    has_prev_selectable = true;
                    break;
                }
                prev = w.prev_sibling();
            }
            if !has_prev_selectable && search.is_visible() {
                search.grab_focus();
                return glib::Propagation::Stop;
            }
        }

        let has_command_modifiers = modifiers.intersects(
            gtk::gdk::ModifierType::CONTROL_MASK
                | gtk::gdk::ModifierType::ALT_MASK
                | gtk::gdk::ModifierType::SUPER_MASK,
        );
        if !has_command_modifiers && search.is_visible() {
            if key == gtk::gdk::Key::BackSpace {
                search.grab_focus();
                let mut text = search.text().to_string();
                text.pop();
                search.set_text(&text);
                search.set_position(-1);
                return glib::Propagation::Stop;
            }
            if let Some(ch) = key.to_unicode().filter(|c| !c.is_control()) {
                search.grab_focus();
                let mut text = search.text().to_string();
                text.push(ch);
                search.set_text(&text);
                search.set_position(-1);
                return glib::Propagation::Stop;
            }
        }

        glib::Propagation::Proceed
    });
    list.add_controller(list_keys);

    let confirm_for_search = layout.confirm.downgrade();
    search_entry.connect_activate(move |_| {
        if let Some(confirm) = confirm_for_search.upgrade()
            && confirm.is_sensitive()
        {
            confirm.emit_clicked();
        }
    });

    let entries_rc = Rc::new(entries);
    let entries_for_filter = entries_rc.clone();
    let list_for_filter = list.downgrade();
    let list_scroll_for_filter = list_scroll.downgrade();
    let empty_search_for_filter = empty_search.downgrade();
    let confirm_for_filter = layout.confirm.downgrade();
    let rec_heading = recommended_heading_row;
    let recent_heading = recent_heading_row;
    let oth_heading = other_heading_row;

    search_entry.connect_changed(move |search| {
        let (Some(list), Some(list_scroll), Some(empty_label), Some(confirm)) = (
            list_for_filter.upgrade(),
            list_scroll_for_filter.upgrade(),
            empty_search_for_filter.upgrade(),
            confirm_for_filter.upgrade(),
        ) else {
            return;
        };
        let query = search.text().trim().to_lowercase();
        let mut recent_count = 0;
        let mut rec_count = 0;
        let mut oth_count = 0;
        let mut first_visible_row: Option<gtk::ListBoxRow> = None;
        let current_selected = list.selected_row();
        let mut selected_is_visible = false;

        for entry in entries_for_filter.iter() {
            let matches = query.is_empty() || entry.haystack.contains(&query);
            entry.row.set_visible(matches);
            if matches {
                match entry.section {
                    AppSection::Recent => recent_count += 1,
                    AppSection::Recommended => rec_count += 1,
                    AppSection::Other => oth_count += 1,
                }
                if first_visible_row.is_none() {
                    first_visible_row = Some(entry.row.clone());
                }
                if let Some(selected) = &current_selected
                    && &entry.row == selected
                {
                    selected_is_visible = true;
                }
            }
        }

        if let Some(heading) = &recent_heading {
            heading.set_visible(recent_count > 0);
        }
        if let Some(heading) = &rec_heading {
            heading.set_visible(rec_count > 0);
        }
        if let Some(heading) = &oth_heading {
            heading.set_visible(oth_count > 0);
        }

        let total_matches = recent_count + rec_count + oth_count;
        if total_matches == 0 {
            empty_label.set_visible(true);
            list_scroll.set_visible(false);
            confirm.set_sensitive(false);
            list.select_row(None::<&gtk::ListBoxRow>);
        } else {
            empty_label.set_visible(false);
            list_scroll.set_visible(true);
            confirm.set_sensitive(true);
            if !selected_is_visible && let Some(row) = &first_visible_row {
                list.select_row(Some(row));
            }
        }
    });

    let layer = modal_layer(&layout.content, &window_overlay, blurred_root.clone(), None);
    layer.connect_unrealize(move |_| {
        let on_close = on_close.clone();
        glib::idle_add_local_once(move || on_close());
    });
    window_overlay.add_overlay(&layer);
    let dismissed = Rc::new(Cell::new(false));
    let dismiss_layer = layer.downgrade();
    let dismiss_overlay = window_overlay.downgrade();
    let dismiss_root = blurred_root.as_ref().map(|root| root.downgrade());
    let dismiss = Rc::new(move || {
        if dismissed.replace(true) {
            return;
        }
        let (Some(layer), Some(overlay)) = (dismiss_layer.upgrade(), dismiss_overlay.upgrade())
        else {
            return;
        };
        let root = dismiss_root.as_ref().and_then(|root| root.upgrade());
        dismiss_modal_layer(&layer, &overlay, root.as_ref());
    });

    let cancel_dismiss = dismiss.clone();
    layout.cancel.connect_clicked(move |_| cancel_dismiss());
    let close_dismiss = dismiss.clone();
    layout.close.connect_clicked(move |_| close_dismiss());

    let open_dismiss = dismiss.clone();
    let open_files = files;
    let open_content_types = content_types;
    let entries_for_open = entries_rc;
    let open_parent = parent.as_ref().downgrade();
    let selected_list = list.downgrade();
    layout.confirm.connect_clicked(move |_| {
        let Some(list) = selected_list.upgrade() else {
            return;
        };
        let Some(row) = list.selected_row() else {
            return;
        };
        let Some(entry) = entries_for_open.iter().find(|e| e.row == row) else {
            return;
        };
        if always_use.is_active() {
            for content_type in &open_content_types {
                if let Err(error) = entry.app.set_as_default_for_type(content_type) {
                    tracing::warn!(%content_type, %error, "unable to set default application");
                }
            }
        }
        let context = list.display().app_launch_context();
        if let Err(error) = launch(&entry.app, &open_files, &open_content_types, Some(&context)) {
            let detail = error.to_string();
            open_dismiss();
            let open_parent = open_parent.clone();
            glib::timeout_add_local_once(Duration::from_millis(250), move || {
                if let Some(parent) = open_parent.upgrade() {
                    show_error_dialog(&parent, &crate::i18n::tr("Unable to open file"), &detail);
                }
            });
            return;
        }
        open_dismiss();
    });
    let activate_confirm = layout.confirm.downgrade();
    list.connect_row_activated(move |_, _| {
        if let Some(confirm) = activate_confirm.upgrade() {
            confirm.emit_clicked();
        }
    });

    let escape_dismiss = dismiss;
    let escape_search = search_entry.downgrade();
    let escape = gtk::EventControllerKey::new();
    escape.set_propagation_phase(gtk::PropagationPhase::Capture);
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if let Some(search) = escape_search.upgrade()
                && !search.text().is_empty()
            {
                search.set_text("");
                search.grab_focus();
                return glib::Propagation::Stop;
            }
            escape_dismiss();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    layer.add_controller(escape);
    if !has_apps {
        layout.cancel.grab_focus();
    } else {
        search_entry.grab_focus();
    }
}

#[cfg(test)]
mod tests;
