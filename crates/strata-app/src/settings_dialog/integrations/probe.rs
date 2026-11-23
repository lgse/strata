// SPDX-License-Identifier: MIT

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use rune::Pixels;
use strata_core::i18n::tr;
use strata_desktop::services::{
    integration_catalog::{self, Entry, Installation, RequirementResult, Status},
    package_manager::PackageManager,
};

use super::{CatalogRow, ManualRow, reload_providers};
use crate::{App, settings_dialog::PageId, slots::WeakLater};

struct DecodedIcon {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl DecodedIcon {
    fn decode(bytes: &[u8]) -> Option<Self> {
        let rgba = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .ok()?
            .into_rgba8();
        let (width, height) = rgba.dimensions();
        Some(Self {
            width,
            height,
            rgba: rgba.into_raw(),
        })
    }

    fn into_pixels(self) -> Pixels {
        let pixels = Pixels::new();
        pixels.set(self.width, self.height, self.rgba);
        pixels
    }
}

struct ScannedCatalogRow {
    entry: Entry,
    status: Status,
    enabled: bool,
    icon: Option<DecodedIcon>,
}

struct ScannedManualRow {
    installation: Installation,
    icon: Option<DecodedIcon>,
}

struct ScanPayload {
    rows: Vec<ScannedCatalogRow>,
    manual: Vec<ScannedManualRow>,
    manager: Option<PackageManager>,
}

pub(super) fn start_scan(app: &mut App) {
    let state = &mut app.settings_dialog.integrations;
    let generation = state.next_scan_generation();
    let active_scan = Arc::clone(&state.active_scan);
    let weak = state.later.downgrade();
    let catalog_loader = Arc::clone(&state.source.catalog);
    let detect_manager = Arc::clone(&state.source.detect_manager);
    let root = state.source.root.clone();

    glib::MainContext::default().spawn_local(async move {
        let scanned = gio::spawn_blocking(move || {
            scan_blocking(&*catalog_loader, &*detect_manager, root.as_deref())
        })
        .await;
        if active_scan.load(Ordering::Acquire) != generation {
            return;
        }
        let Ok(payload) = scanned else {
            return;
        };
        let Some(later) = weak.upgrade() else {
            return;
        };
        later.then(move |app, _| apply_scan(app, generation, payload));
    });
}

fn scan_blocking(
    catalog_loader: &dyn Fn() -> Vec<Entry>,
    detect_manager: &dyn Fn() -> Option<PackageManager>,
    root: Option<&std::path::Path>,
) -> ScanPayload {
    let entries = catalog_loader();
    let rows: Vec<ScannedCatalogRow> = entries
        .into_iter()
        .map(|entry| {
            let status = root.map_or(Status::NotInstalled, |dir| {
                integration_catalog::status(&entry, dir)
            });
            let enabled = root.is_none_or(|dir| integration_catalog::enabled(dir, &entry.id));
            let icon = entry.icon_bytes().and_then(DecodedIcon::decode);
            ScannedCatalogRow {
                entry,
                status,
                enabled,
                icon,
            }
        })
        .collect();
    let manual: Vec<ScannedManualRow> = root
        .map(|dir| {
            let catalog: Vec<Entry> = rows.iter().map(|row| row.entry.clone()).collect();
            integration_catalog::manual_installations(dir, &catalog)
        })
        .unwrap_or_default()
        .into_iter()
        .map(|installation| {
            let icon = installation.icon.as_deref().and_then(DecodedIcon::decode);
            ScannedManualRow { installation, icon }
        })
        .collect();
    let manager = detect_manager();
    ScanPayload {
        rows,
        manual,
        manager,
    }
}

fn apply_scan(app: &mut App, generation: u64, payload: ScanPayload) -> bool {
    if !app.settings_dialog.is_open() {
        return false;
    }
    let state = &mut app.settings_dialog.integrations;
    if state.scan_generation != generation {
        return false;
    }
    state.manager = payload.manager;
    state.rows = payload
        .rows
        .into_iter()
        .map(|row| CatalogRow {
            entry: row.entry,
            status: row.status,
            enabled: row.enabled,
            logo: row.icon.map(DecodedIcon::into_pixels),
        })
        .collect();
    state.manual = payload
        .manual
        .into_iter()
        .map(|row| ManualRow {
            installation: row.installation,
            logo: row.icon.map(DecodedIcon::into_pixels),
        })
        .collect();
    state.loaded = true;
    if state
        .category
        .is_some_and(|selected| state.rows.iter().all(|row| row.entry.category != selected))
    {
        state.category = None;
    }
    let unchecked: Vec<Entry> = state
        .rows
        .iter()
        .filter(|row| !state.requirements.contains_key(&row.entry.id))
        .map(|row| row.entry.clone())
        .collect();
    if !unchecked.is_empty() {
        let handle = ScanHandle {
            weak: state.later.downgrade(),
            active_scan: Arc::clone(&state.active_scan),
            generation,
        };
        let checker = Arc::clone(&state.source.check);
        spawn_requirement_check(handle, checker, unchecked);
    }
    true
}

struct ScanHandle {
    weak: WeakLater<App>,
    active_scan: Arc<AtomicU64>,
    generation: u64,
}

fn spawn_requirement_check(handle: ScanHandle, checker: super::Checker, entries: Vec<Entry>) {
    glib::MainContext::default().spawn_local(async move {
        let active_scan = handle.active_scan;
        let generation = handle.generation;
        let results = gio::spawn_blocking(move || {
            let mut checked = Vec::with_capacity(entries.len());
            for entry in entries {
                if active_scan.load(Ordering::Acquire) != generation {
                    return Vec::new();
                }
                let reqs = checker(&entry);
                checked.push((entry.id, reqs));
            }
            checked
        })
        .await;
        let Ok(results) = results else {
            return;
        };
        if results.is_empty() {
            return;
        }
        let Some(later) = handle.weak.upgrade() else {
            return;
        };
        later.then(move |app, _| apply_requirements(app, generation, results));
    });
}

fn apply_requirements(
    app: &mut App,
    generation: u64,
    results: Vec<(String, Vec<RequirementResult>)>,
) -> bool {
    if !app.settings_dialog.is_open() {
        return false;
    }
    let state = &mut app.settings_dialog.integrations;
    if state.scan_generation != generation {
        return false;
    }
    state.requirements.extend(results);
    true
}

pub(super) fn run_mutation(
    app: &mut App,
    id: String,
    work: impl FnOnce() -> Result<(), String> + Send + 'static,
) {
    let state = &mut app.settings_dialog.integrations;
    let token = state.next_mutation_token();
    state.busy.insert(id.clone(), token);
    state.row_messages.remove(&id);
    state.confirmations.remove(&id);
    let weak = state.later.downgrade();

    glib::MainContext::default().spawn_local(async move {
        let outcome = gio::spawn_blocking(work).await.ok();
        let Some(later) = weak.upgrade() else {
            return;
        };
        later.then(move |app, _| finish_mutation(app, id, token, outcome));
    });
}

fn finish_mutation(
    app: &mut App,
    id: String,
    token: u64,
    outcome: Option<Result<(), String>>,
) -> bool {
    let owned = app.settings_dialog.integrations.busy.get(&id).copied() == Some(token);
    if owned {
        app.settings_dialog.integrations.busy.remove(&id);
    }
    let show_in_page =
        app.settings_dialog.is_open() && app.settings_dialog.page == PageId::Integrations;
    match outcome {
        Some(Ok(())) => {
            reload_providers(app);
            if show_in_page {
                start_scan(app);
            }
            owned || show_in_page
        }
        Some(Err(error)) => {
            if !app.settings_dialog.is_open() || !owned {
                return false;
            }
            app.settings_dialog.integrations.row_messages.insert(
                id,
                rust_i18n::t!("The change failed: %{error}", error = error).into_owned(),
            );
            true
        }
        None => {
            if !app.settings_dialog.is_open() || !owned {
                return false;
            }
            app.settings_dialog
                .integrations
                .row_messages
                .insert(id, tr("The change failed."));
            true
        }
    }
}
