// SPDX-License-Identifier: MIT

use std::cell::RefCell;

use strata_components::thumbnails::Settings;

use crate::{
    App,
    slots::{Later, WeakLater},
};

thread_local! {
    /// Every window's queue whose store takes a share: the interface thread's.
    static WINDOWS: RefCell<Vec<WeakLater<App>>> = const { RefCell::new(Vec::new()) };
}

/// A window's place in the split, held by its state: dropping it gives its share back.
pub(super) struct Share {
    window: WeakLater<App>,
}

impl Share {
    pub(super) fn join(later: &Later<App>) -> Self {
        let window = later.downgrade();
        WINDOWS.with(|windows| {
            let mut windows = windows.borrow_mut();
            windows.retain(WeakLater::is_live);
            windows.push(later.downgrade());
        });
        rebalance(&window);
        Self { window }
    }
}

impl Drop for Share {
    fn drop(&mut self) {
        WINDOWS.with(|windows| {
            let Ok(mut windows) = windows.try_borrow_mut() else {
                return;
            };
            windows.retain(|open| open.is_live() && !open.same_as(&self.window));
        });
        rebalance(&self.window);
    }
}

/// The bytes each window's store may hold now.
pub(super) fn share() -> usize {
    let open = WINDOWS.with(|windows| windows.borrow().iter().filter(|w| w.is_live()).count());
    Settings::default().budget_bytes / open.max(1)
}

/// Has every window but `own` take its share at its next update.
fn rebalance(own: &WeakLater<App>) {
    let others: Vec<Later<App>> = WINDOWS.with(|windows| {
        let windows = windows.borrow();
        let others = windows.iter().filter(|open| !open.same_as(own));
        others.filter_map(WeakLater::upgrade).collect()
    });
    for later in others {
        later.then(|app, _| take_share(app));
    }
}

/// Holds `app`'s store to its share; a smaller share evicts at once. True when it changed.
fn take_share(app: &mut App) -> bool {
    let store = app.window.thumbnails_mut();
    let settings = store.settings();
    let share = share();
    if settings.budget_bytes == share {
        return false;
    }
    store.set_settings(settings.budget(share));
    true
}

/// The budget `app`'s store holds to, for a test of the split.
#[cfg(test)]
pub(crate) fn budget(app: &App) -> usize {
    crate::shell::thumbnails(app).settings().budget_bytes
}

#[cfg(test)]
mod tests;
