// SPDX-License-Identifier: MIT

mod checks;
mod format;
mod runner;
#[cfg(test)]
mod tests;
mod view;

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Sender, TryRecvError},
    },
    time::Duration,
};

use rune::Tx;
use strata_core::i18n::tr;
use strata_desktop::services::package_manager::PackageManager;

use checks::{Check, CheckId, GROUPS, custom_actions, terminal};
pub(super) use view::build;

use crate::{
    App,
    settings_dialog::install_command::{self, COPY_FEEDBACK_SECS as COMMAND_COPY_FEEDBACK_SECS},
    slots::{Later, WeakLater},
    terminal::Emulator,
};

const COPY_FEEDBACK_SECS: f64 = 1.5;
const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[cfg(test)]
type TestSpawner = std::rc::Rc<dyn Fn(Sender<Check>)>;

pub(super) struct DiagnosticsState {
    later: Later<App>,
    generation: u64,
    active_generation: Arc<AtomicU64>,
    running: bool,
    has_run: bool,
    completed: bool,
    pending_parts: u8,
    manager: Option<PackageManager>,
    environment: Vec<(String, String)>,
    results: HashMap<CheckId, Check>,
    copied_until: Option<f64>,
    copied_command: Option<(CheckId, f64)>,
    terminal_status: HashMap<CheckId, Result<String, String>>,
    #[cfg(test)]
    spawner: Option<TestSpawner>,
}

impl DiagnosticsState {
    pub(super) fn new(later: Later<App>) -> Self {
        Self {
            later,
            generation: 0,
            active_generation: Arc::new(AtomicU64::new(0)),
            running: false,
            has_run: false,
            completed: false,
            pending_parts: 0,
            manager: None,
            environment: Vec::new(),
            results: HashMap::new(),
            copied_until: None,
            copied_command: None,
            terminal_status: HashMap::new(),
            #[cfg(test)]
            spawner: None,
        }
    }

    pub(super) fn can_copy(&self) -> bool {
        self.completed && !self.running
    }

    pub(super) fn closed(&mut self) {
        self.copied_until = None;
        self.copied_command = None;
        if !self.running {
            return;
        }
        self.generation = self.generation.saturating_add(1);
        self.active_generation
            .store(self.generation, Ordering::Release);
        self.running = false;
        self.pending_parts = 0;
        if !self.completed {
            self.has_run = false;
            self.results.clear();
            self.terminal_status.clear();
        }
    }

    fn apply_check(&mut self, generation: u64, check: Check) -> bool {
        if self.generation != generation {
            return false;
        }
        self.results.insert(check.id, check);
        true
    }

    fn apply_environment(
        &mut self,
        generation: u64,
        manager: Option<PackageManager>,
        environment: Vec<(String, String)>,
    ) -> bool {
        if self.generation != generation {
            return false;
        }
        self.manager = manager;
        self.environment = environment;
        true
    }

    fn finish_part(&mut self, generation: u64) -> bool {
        if self.generation != generation {
            return false;
        }
        self.pending_parts = self.pending_parts.saturating_sub(1);
        if self.pending_parts > 0 {
            return false;
        }
        for (_, ids) in GROUPS {
            for &id in ids {
                self.results.entry(id).or_insert_with(|| Check::not_run(id));
            }
        }
        self.running = false;
        self.completed = true;
        true
    }
}

pub(super) fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let state = &mut app.settings_dialog.diagnostics;
    let mut changed = false;
    if let Some(until) = state.copied_until {
        if clock >= until {
            state.copied_until = None;
            changed = true;
        } else {
            tx.wake_after(Duration::from_secs_f64(until - clock));
        }
    }
    if let Some((_, until)) = state.copied_command {
        if clock >= until {
            state.copied_command = None;
            changed = true;
        } else {
            tx.wake_after(Duration::from_secs_f64(until - clock));
        }
    }
    changed
}

pub(super) fn start(app: &mut App, _tx: &mut Tx) {
    let state = &mut app.settings_dialog.diagnostics;
    if state.running {
        return;
    }
    state.generation = state.generation.saturating_add(1);
    let generation = state.generation;
    state.active_generation.store(generation, Ordering::Release);
    state.running = true;
    state.has_run = true;
    state.completed = false;
    state.pending_parts = 2;
    state.results.clear();
    state.terminal_status.clear();
    state.copied_until = None;
    state.copied_command = None;
    state.environment = runner::initial_environment();

    let (sender, receiver) = mpsc::channel();
    spawn_background_checks(state, sender);

    let weak_local = state.later.downgrade();
    let weak_poll = state.later.downgrade();
    let active_generation = Arc::clone(&state.active_generation);
    let registry = crate::custom_actions::shared_registry(app);
    glib::MainContext::default().spawn_local(async move {
        let facts =
            gio::spawn_blocking(|| (PackageManager::detect(), runner::detect_environment())).await;
        if active_generation.load(Ordering::Acquire) != generation {
            return;
        }
        if let Ok((manager, environment)) = facts
            && let Some(later) = weak_local.upgrade()
        {
            later.then(move |app, _| {
                app.settings_dialog
                    .diagnostics
                    .apply_environment(generation, manager, environment)
            });
        }
        attach_receiver_poll(
            weak_poll,
            Arc::clone(&active_generation),
            generation,
            receiver,
        );
        let local = local_checks(registry).await;
        if active_generation.load(Ordering::Acquire) != generation {
            return;
        }
        if let Some(later) = weak_local.upgrade() {
            later.then(move |app, _| {
                let diag = &mut app.settings_dialog.diagnostics;
                let mut changed = false;
                for check in local {
                    changed |= diag.apply_check(generation, check);
                }
                changed | diag.finish_part(generation)
            });
        }
    });
}

fn spawn_background_checks(_state: &DiagnosticsState, sender: Sender<Check>) {
    #[cfg(test)]
    if let Some(spawner) = &_state.spawner {
        spawner(sender);
        return;
    }
    runner::spawn(sender);
}

fn attach_receiver_poll(
    weak: WeakLater<App>,
    active_generation: Arc<AtomicU64>,
    generation: u64,
    receiver: mpsc::Receiver<Check>,
) {
    glib::timeout_add_local(POLL_INTERVAL, move || {
        if active_generation.load(Ordering::Acquire) != generation {
            return glib::ControlFlow::Break;
        }
        let Some(later) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let mut batch = Vec::new();
        let mut disconnected = false;
        loop {
            match receiver.try_recv() {
                Ok(check) => batch.push(check),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if !batch.is_empty() || disconnected {
            later.then(move |app, _| {
                let diag = &mut app.settings_dialog.diagnostics;
                let mut changed = false;
                for check in batch {
                    changed |= diag.apply_check(generation, check);
                }
                if disconnected {
                    changed |= diag.finish_part(generation);
                }
                changed
            });
        }
        if disconnected {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

async fn local_checks(registry: std::rc::Rc<strata_fs::services::ActionRegistry>) -> [Check; 4] {
    let blocking = gio::spawn_blocking(|| {
        let terminal_prog =
            Emulator::resolve().map(|term| term.program().to_string_lossy().into_owned());
        let languages = strata_components::code_view::highlight::language_count();
        let watch_limit = runner::read_inotify_watch_limit();
        (terminal_prog, languages, watch_limit)
    });
    let catalog = registry.reload().await;
    let unavailable = catalog
        .actions()
        .iter()
        .filter(|action| action.definition.enabled && !action.is_available())
        .map(|action| action.name().to_owned())
        .collect();
    let failed = catalog
        .failures()
        .iter()
        .map(|failure| failure.directory.clone())
        .collect();
    let (terminal_prog, languages, watch_limit) = blocking.await.unwrap_or((None, 0, None));
    [
        runner::syntax_highlighting(languages),
        runner::folder_monitoring(watch_limit),
        terminal(terminal_prog),
        custom_actions(catalog.actions().len(), unavailable, failed),
    ]
}

pub(super) fn copy_report(app: &mut App, tx: &mut Tx) {
    let state = &mut app.settings_dialog.diagnostics;
    if !state.can_copy() {
        return;
    }
    let text = format::report(&state.environment, &state.results, state.manager);
    state.copied_until = Some(tx.clock() + COPY_FEEDBACK_SECS);
    app.clipboard.write_text(&text);
    tx.wake_after(Duration::from_secs_f64(COPY_FEEDBACK_SECS));
}

fn copy_command(app: &mut App, id: CheckId, command: &str, tx: &mut Tx) {
    let state = &mut app.settings_dialog.diagnostics;
    state.copied_command = Some((id, tx.clock() + COMMAND_COPY_FEEDBACK_SECS));
    app.clipboard.write_text(command);
    tx.wake_after(Duration::from_secs_f64(COMMAND_COPY_FEEDBACK_SECS));
}

fn run_install_command(app: &mut App, id: CheckId, command: &str) {
    let state = &app.settings_dialog.diagnostics;
    let generation = state.generation;
    let weak = state.later.downgrade();
    let finished =
        tr("The command opened in your terminal. Run diagnostics again when it finishes.");
    install_command::run_in_terminal(weak, command, move |app, outcome| {
        let diag = &mut app.settings_dialog.diagnostics;
        if diag.generation != generation {
            return false;
        }
        let status = outcome.map(|()| finished);
        diag.terminal_status.insert(id, status);
        true
    });
}
