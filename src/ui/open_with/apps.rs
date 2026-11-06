// SPDX-License-Identifier: MIT

use std::collections::{HashMap, HashSet};

use gtk::{gio, glib, prelude::*};

use super::uses_portal;
use crate::ui::recent_apps;

fn categorized_apps(
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
pub(in crate::ui) fn requires_uri_handlers(files: &[gio::File]) -> bool {
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

fn filter_other_apps(
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
    history: &recent_apps::State,
    content_types: &[String],
    recommended: &mut Vec<gio::AppInfo>,
    other: &mut Vec<gio::AppInfo>,
) -> Vec<gio::AppInfo> {
    let ordered = history.recent_ids(content_types);
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

#[derive(Clone, Default)]
pub(in crate::ui) struct Choices {
    pub(super) recent: Vec<gio::AppInfo>,
    pub(super) recommended: Vec<gio::AppInfo>,
    pub(super) other: Vec<gio::AppInfo>,
}

impl Choices {
    fn new(
        history: &recent_apps::State,
        content_types: &[String],
        mut recommended: Vec<gio::AppInfo>,
        mut other: Vec<gio::AppInfo>,
    ) -> Self {
        let recent = split_recent(history, content_types, &mut recommended, &mut other);
        Self {
            recent,
            recommended,
            other,
        }
    }

    fn is_empty(&self) -> bool {
        self.recent.is_empty() && self.recommended.is_empty() && self.other.is_empty()
    }
}

pub(in crate::ui) fn choices_for_type(
    history: &recent_apps::State,
    content_type: &str,
    requires_uris: bool,
) -> Choices {
    let (recommended, other) = categorized_apps(content_type, requires_uris);
    Choices::new(history, &[content_type.to_owned()], recommended, other)
}

pub(in crate::ui) struct Applications {
    pub(in crate::ui) choices: Choices,
    pub(in crate::ui) default: Option<gio::AppInfo>,
    pub(in crate::ui) content_types: Vec<String>,
}

impl Applications {
    pub(in crate::ui) fn unavailable_reason(&self) -> Option<&'static str> {
        if uses_portal() || !self.choices.is_empty() {
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
pub(in crate::ui) async fn resolve(
    files: &[gio::File],
    current: impl Fn() -> bool,
) -> Option<Result<Applications, &'static str>> {
    let mut content_types = Vec::<String>::new();
    for file in files {
        if !current() {
            return None;
        }
        let info = query_type_info(file).await;
        if !current() {
            return None;
        }
        let next_type = match content_type_from(info) {
            Ok(next_type) => next_type,
            Err(message) => return Some(Err(message)),
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
    let history = recent_apps::read().await.unwrap_or_default();
    if !current() {
        return None;
    }
    let requires_uris = requires_uri_handlers(files);
    let (recommended, other, default) = common_applications(&content_types, requires_uris);
    Some(Ok(Applications {
        choices: Choices::new(&history, &content_types, recommended, other),
        default,
        content_types,
    }))
}

async fn query_type_info(file: &gio::File) -> Result<gio::FileInfo, glib::Error> {
    file.query_info_future(
        &format!(
            "standard::type,{}",
            crate::adapters::CONTENT_TYPE_ATTRIBUTES
        ),
        gio::FileQueryInfoFlags::NONE,
        glib::Priority::DEFAULT,
    )
    .await
}

fn content_type_from(info: Result<gio::FileInfo, glib::Error>) -> Result<String, &'static str> {
    let Ok(info) = info else {
        return Err("Unable to read the selected file type");
    };
    if info.file_type() == gio::FileType::SymbolicLink {
        return Err("Broken symbolic links cannot be opened with an application");
    }
    crate::adapters::content_type_of(&info)
        .map(|value| value.to_string())
        .ok_or("Unable to determine the selected file type")
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
