// SPDX-License-Identifier: MIT

//! A close the window manager asked for, refused by hand ([`super`], "Refused closes"). The
//! workaround retires with Rune packet entry 110, a close request a window can refuse.

use strata_platform::csd::{Backend, Csd, Policy};

use super::{Start, UI, is_last_window, next_key, window};
use crate::{App, shell, shell::close_guard};

/// Keeps window `app` open when a close guard refuses its close: its state moves into a window
/// opened in its place, which says why, and the closing window keeps a stand-in. True when it
/// did, and the rest of the close is not to run.
pub(super) fn keep_open(app: &mut App) -> bool {
    let last = is_last_window(shell::window_id(app));
    let Some(refusal) = close_guard::refusal(app, last) else {
        return false;
    };
    let Some(ui) = UI.with(|ui| ui.get().copied()) else {
        return false;
    };
    let csd = Csd::new(Policy::Auto.frame(), Backend::from_env());
    let stand_in = App::new(&ui, csd, shell::share_settings(app));
    let mut kept = std::mem::replace(app, stand_in);
    close_guard::tell(&mut kept, refusal);
    tracing::debug!(
        ?refusal,
        window = shell::window_id(&kept),
        "the window manager's close was refused: the window opens again"
    );
    let kept = window(ui, Start::Kept(Box::new(kept)));
    rune::open_window(rune::WindowSpec::from_app(next_key(), kept));
    true
}
