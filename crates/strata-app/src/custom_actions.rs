// SPDX-License-Identifier: MIT

mod dialog;
mod icons;
mod jobs;
mod menu;
mod shared;
mod slots;
#[cfg(test)]
mod tests;

use std::{cell::Cell, rc::Rc};

use rune::Tx;
use strata_components::progress_dock::jobs::{JobRow, JobsState};
use strata_fs::{
    adapters::LocalActionStore,
    services::{ActionRegistry, JobId, JobService, ListenerGuard},
};

pub(crate) use icons::{ACTION_ICON_SLUGS, action_icon, action_icon_glyph, is_known_action_icon};
pub(crate) use jobs::build_footer_jobs as jobs_el;
pub(crate) use menu::{activate_menu_action, begin_menu, is_menu_action};
#[cfg_attr(
    not(test),
    expect(
        unused_imports,
        reason = "re-exported for A11 10xer numbered action chords"
    )
)]
pub(crate) use slots::{NumberedActions, numbered_actions, run_numbered};

use crate::{
    App,
    commands::{Action, Holds},
    slots::{Later, Part, Place, Scope, Setup},
};

struct Services {
    registry: Rc<ActionRegistry>,
    /// The process's job service ([`shared`]).
    jobs: Rc<JobService>,
    /// This window's observer of it, for its Jobs dashboard.
    _jobs_guard: ListenerGuard<Rc<dyn Fn()>>,
}

pub(crate) struct CustomActions {
    later: Later<App>,
    started: bool,
    services: Option<Services>,
    pub(crate) jobs_state: JobsState,
    pub(crate) job_rows: Vec<JobRow>,
    featured_job: Option<JobId>,
    pump_scheduled: Rc<Cell<bool>>,
    clock: f64,
    menu_context: Option<menu::MenuContext>,
    confirm: Option<dialog::ConfirmSpec>,
    confirm_open: bool,
    confirm_generation: u64,
}

impl Part for CustomActions {
    const SCOPE: Scope = Scope::Window;

    const PLACE: Place<App> = Place::Dialogs(dialog::build);

    const COMMANDS: &'static [Action<App>] = slots::COMMANDS;

    fn new(setup: &Setup<'_>) -> Self {
        let mut jobs_state = JobsState::new();
        jobs_state.words = jobs::localized_words();
        Self {
            later: setup.later.clone(),
            started: false,
            services: None,
            jobs_state,
            job_rows: Vec::new(),
            featured_job: None,
            pump_scheduled: Rc::new(Cell::new(false)),
            clock: 0.0,
            menu_context: None,
            confirm: None,
            confirm_open: false,
            confirm_generation: 0,
        }
    }

    fn keyboard(app: &App) -> Option<Holds> {
        if app.custom_actions.confirm_open {
            Some(Holds::Modal { field: false })
        } else if app.custom_actions.jobs_state.is_open() {
            Some(Holds::Popover)
        } else {
            None
        }
    }

    fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
        app.custom_actions.clock = clock;
        if !app.custom_actions.started {
            app.custom_actions.started = true;
            let _ = shared_registry(app);
        }
        let mut changed = false;
        if let Some(service) = app.custom_actions.job_service() {
            service.pump();
            changed |= jobs::sync_rows(app);
            if jobs::has_active_jobs(&service) {
                tx.wake_after(jobs::PUMP_INTERVAL);
                jobs::schedule_glib_pump(app, jobs::PUMP_INTERVAL);
            }
        }
        changed |= app
            .custom_actions
            .jobs_state
            .tick(clock, &app.custom_actions.job_rows, tx);
        changed
    }
}

impl CustomActions {
    fn ensure_services(&mut self) -> &Services {
        let later = self.later.clone();
        self.services.get_or_insert_with(move || {
            let registry = ActionRegistry::new(LocalActionStore::new());
            let jobs = shared::join();
            let jobs_later = later;
            let jobs_guard = jobs.observe(Rc::new(move || {
                jobs_later.then(|app, _| jobs::sync_rows(app));
            }));
            Services {
                registry,
                jobs,
                _jobs_guard: jobs_guard,
            }
        })
    }

    fn job_service(&self) -> Option<Rc<JobService>> {
        self.services.as_ref().map(|s| s.jobs.clone())
    }

    pub(crate) fn catalog(&self) -> Option<Rc<strata_fs::services::ActionCatalog>> {
        self.services.as_ref().map(|s| s.registry.catalog())
    }
}

pub(crate) fn shared_registry(app: &mut App) -> Rc<ActionRegistry> {
    let registry = app.custom_actions.ensure_services().registry.clone();
    if !registry.reload_waiting() {
        glib::MainContext::default().spawn_local(registry.clone().reload());
    }
    registry
}

pub(crate) fn jobs_active(app: &App) -> bool {
    app.custom_actions
        .job_service()
        .is_some_and(|service| jobs::has_active_jobs(&service))
}

/// The process's job service, if this window has joined it yet.
#[cfg(test)]
pub(crate) fn job_service(app: &App) -> Option<Rc<JobService>> {
    app.custom_actions.job_service()
}

pub(crate) fn shared_jobs(app: &mut App) -> Rc<JobService> {
    app.custom_actions.ensure_services().jobs.clone()
}
