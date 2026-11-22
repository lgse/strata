// SPDX-License-Identifier: MIT

//! Settings -> Integrations page state, catalog/provider coordination, and actions,
//! ported from `src/ui/settings/integrations.rs`.

mod catalog;
mod installed;
mod probe;
#[cfg(test)]
mod tests;
mod view;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use rune::{Pixels, Tx};
use strata_core::{i18n::tr, model::Location};
use strata_desktop::services::{
    integration_catalog::{self, Category, Entry, Installation, RequirementResult, Status},
    package_manager::PackageManager,
};
use strata_fs::services::file_providers;

pub(in crate::settings_dialog) use view::build;

use crate::{
    App,
    settings_dialog::install_command::{self, COPY_FEEDBACK_SECS},
    slots::Later,
};

pub(super) type Checker = Arc<dyn Fn(&Entry) -> Vec<RequirementResult> + Send + Sync>;
pub(super) type CatalogLoader = Arc<dyn Fn() -> Vec<Entry> + Send + Sync>;
pub(super) type ManagerDetector = Arc<dyn Fn() -> Option<PackageManager> + Send + Sync>;
pub(super) type ReloadHook = Rc<dyn Fn(&mut App)>;
pub(super) type OpenFolderHook = Rc<dyn Fn(&mut App, PathBuf)>;
pub(super) type CommandKey = (String, usize);

/// Where the page reads the catalog, provider config root, requirement checks, and side effects;
/// tests substitute any of these without touching the host environment.
pub(super) struct Source {
    pub(super) catalog: CatalogLoader,
    pub(super) root: Option<PathBuf>,
    pub(super) check: Checker,
    pub(super) detect_manager: ManagerDetector,
    pub(super) reload: Option<ReloadHook>,
    pub(super) open_folder: Option<OpenFolderHook>,
}

impl Default for Source {
    fn default() -> Self {
        Self {
            catalog: Arc::new(integration_catalog::catalog),
            root: file_providers::config_root(),
            check: Arc::new(integration_catalog::check_requirements),
            detect_manager: Arc::new(PackageManager::detect),
            reload: None,
            open_folder: None,
        }
    }
}

pub(super) struct CatalogRow {
    pub(super) entry: Entry,
    pub(super) status: Status,
    pub(super) enabled: bool,
    pub(super) logo: Option<Pixels>,
}

pub(super) struct ManualRow {
    pub(super) installation: Installation,
    pub(super) logo: Option<Pixels>,
}

#[derive(Clone)]
pub(super) enum ConfirmAction {
    Replace(Box<Entry>),
    Remove,
}

#[derive(Clone)]
pub(super) struct PendingConfirm {
    pub(super) action: ConfirmAction,
    pub(super) question: &'static str,
    pub(super) accept_label: &'static str,
    pub(super) accessible: String,
}

pub(in crate::settings_dialog) struct IntegrationsState {
    pub(super) later: Later<App>,
    pub(super) source: Source,
    pub(super) scan_generation: u64,
    pub(super) active_scan: Arc<AtomicU64>,
    pub(super) session_generation: u64,
    next_mutation: u64,
    needs_scan: bool,
    pub(super) loaded: bool,
    pub(super) rows: Vec<CatalogRow>,
    pub(super) manual: Vec<ManualRow>,
    pub(super) manager: Option<PackageManager>,
    pub(super) category: Option<Category>,
    pub(super) expanded: HashSet<String>,
    pub(super) requirements: HashMap<String, Vec<RequirementResult>>,
    pub(super) busy: HashMap<String, u64>,
    pub(super) confirmations: HashMap<String, PendingConfirm>,
    pub(super) row_messages: HashMap<String, String>,
    pub(super) copied_command: Option<(CommandKey, f64)>,
    pub(super) terminal_status: HashMap<CommandKey, Result<String, String>>,
}

impl IntegrationsState {
    pub(in crate::settings_dialog) fn new(later: Later<App>) -> Self {
        Self {
            later,
            source: Source::default(),
            scan_generation: 0,
            active_scan: Arc::new(AtomicU64::new(0)),
            session_generation: 0,
            next_mutation: 0,
            needs_scan: false,
            loaded: false,
            rows: Vec::new(),
            manual: Vec::new(),
            manager: None,
            category: None,
            expanded: HashSet::new(),
            requirements: HashMap::new(),
            busy: HashMap::new(),
            confirmations: HashMap::new(),
            row_messages: HashMap::new(),
            copied_command: None,
            terminal_status: HashMap::new(),
        }
    }

    pub(in crate::settings_dialog) fn mark_needs_scan(&mut self) {
        self.needs_scan = true;
    }

    /// Discards in-flight probe/scan completions and clears transient row state when Settings closes.
    pub(in crate::settings_dialog) fn closed(&mut self) {
        self.scan_generation = self.scan_generation.saturating_add(1);
        self.active_scan
            .store(self.scan_generation, Ordering::Release);
        self.session_generation = self.session_generation.saturating_add(1);
        self.needs_scan = false;
        self.busy.clear();
        self.confirmations.clear();
        self.row_messages.clear();
        self.copied_command = None;
        self.terminal_status.clear();
    }

    pub(super) fn next_scan_generation(&mut self) -> u64 {
        self.scan_generation = self.scan_generation.saturating_add(1);
        self.active_scan
            .store(self.scan_generation, Ordering::Release);
        self.scan_generation
    }

    pub(super) fn next_mutation_token(&mut self) -> u64 {
        self.next_mutation = self.next_mutation.saturating_add(1);
        self.next_mutation
    }
}

pub(in crate::settings_dialog) fn ensure_scanned(app: &mut App) {
    if !app.settings_dialog.integrations.needs_scan {
        return;
    }
    app.settings_dialog.integrations.needs_scan = false;
    probe::start_scan(app);
}

pub(in crate::settings_dialog) fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let state = &mut app.settings_dialog.integrations;
    let Some((_, until)) = state.copied_command.as_ref() else {
        return false;
    };
    if clock >= *until {
        state.copied_command = None;
        return true;
    }
    tx.wake_after(Duration::from_secs_f64(*until - clock));
    false
}

pub(super) fn refresh(app: &mut App) {
    reload_providers(app);
    let state = &mut app.settings_dialog.integrations;
    state.session_generation = state.session_generation.saturating_add(1);
    state.requirements.clear();
    state.terminal_status.clear();
    state.row_messages.clear();
    state.confirmations.clear();
    state.copied_command = None;
    probe::start_scan(app);
}

pub(super) fn open_folder(app: &mut App) {
    let Some(root) = app.settings_dialog.integrations.source.root.clone() else {
        return;
    };
    if let Some(hook) = app.settings_dialog.integrations.source.open_folder.clone() {
        hook(app, root);
        return;
    }
    super::close_settings(app);
    app.open(Location::local(root));
}

pub(super) fn select_category(app: &mut App, category: Option<Category>) {
    app.settings_dialog.integrations.category = category;
}

pub(super) fn toggle_requirements(app: &mut App, id: String) {
    let expanded = &mut app.settings_dialog.integrations.expanded;
    if !expanded.remove(&id) {
        expanded.insert(id);
    }
}

pub(super) fn copy_command(app: &mut App, key: CommandKey, command: &str, tx: &mut Tx) {
    app.settings_dialog.integrations.copied_command = Some((key, tx.clock() + COPY_FEEDBACK_SECS));
    app.clipboard.write_text(command);
    tx.wake_after(Duration::from_secs_f64(COPY_FEEDBACK_SECS));
}

pub(super) fn run_command(app: &mut App, key: CommandKey, command: &str) {
    let state = &app.settings_dialog.integrations;
    let session = state.session_generation;
    let weak = state.later.downgrade();
    let finished = tr("The command opened in your terminal. Select Refresh when it finishes.");
    install_command::run_in_terminal(weak, command, move |app, outcome| {
        if !app.settings_dialog.is_open() {
            return false;
        }
        let state = &mut app.settings_dialog.integrations;
        if state.session_generation != session {
            return false;
        }
        state
            .terminal_status
            .insert(key, outcome.map(|()| finished));
        true
    });
}

pub(super) fn install_entry(app: &mut App, entry: Entry) {
    let id = entry.id.clone();
    let Some(root) = app.settings_dialog.integrations.source.root.clone() else {
        app.settings_dialog
            .integrations
            .row_messages
            .insert(id, tr("No configuration folder is available."));
        return;
    };
    probe::run_mutation(app, id, move || {
        integration_catalog::install(&entry, &root).map(|_| ())
    });
}

pub(super) fn set_enabled(app: &mut App, id: String, enabled: bool) {
    let Some(root) = app.settings_dialog.integrations.source.root.clone() else {
        return;
    };
    let target = id.clone();
    probe::run_mutation(app, id, move || {
        integration_catalog::set_enabled(&root, &target, enabled)
    });
}

pub(super) fn request_update(app: &mut App, entry: Entry, name: String, modified: bool) {
    if !modified {
        install_entry(app, entry);
        return;
    }
    let id = entry.id.clone();
    app.settings_dialog.integrations.confirmations.insert(
        id,
        PendingConfirm {
            action: ConfirmAction::Replace(Box::new(entry)),
            question: "The installed copy has local changes. Updating replaces them with the bundled files.",
            accept_label: "Replace",
            accessible: rust_i18n::t!("Confirm replacing %{name}", name = name).into_owned(),
        },
    );
}

pub(super) fn request_remove(
    app: &mut App,
    id: String,
    name: String,
    question: Option<&'static str>,
) {
    let Some(question) = question else {
        remove_entry(app, id);
        return;
    };
    app.settings_dialog.integrations.confirmations.insert(
        id,
        PendingConfirm {
            action: ConfirmAction::Remove,
            question,
            accept_label: "Remove",
            accessible: rust_i18n::t!("Confirm removing %{name}", name = name).into_owned(),
        },
    );
}

pub(super) fn cancel_confirm(app: &mut App, id: &str) {
    app.settings_dialog.integrations.confirmations.remove(id);
}

pub(super) fn accept_confirm(app: &mut App, id: String) {
    let Some(confirm) = app.settings_dialog.integrations.confirmations.remove(&id) else {
        return;
    };
    match confirm.action {
        ConfirmAction::Replace(entry) => install_entry(app, *entry),
        ConfirmAction::Remove => remove_entry(app, id),
    }
}

fn remove_entry(app: &mut App, id: String) {
    let Some(root) = app.settings_dialog.integrations.source.root.clone() else {
        return;
    };
    let target = id.clone();
    probe::run_mutation(app, id, move || integration_catalog::remove(&root, &target));
}

pub(super) fn reload_providers(app: &mut App) {
    if let Some(hook) = app.settings_dialog.integrations.source.reload.clone() {
        hook(app);
    } else {
        crate::file_providers::reload(app);
    }
}
