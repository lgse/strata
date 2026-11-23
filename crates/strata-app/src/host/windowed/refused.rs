// SPDX-License-Identifier: MIT

use strata_platform::csd::{Backend, Csd, Policy};

use super::{Start, UI, is_last_window, next_key, window};
use crate::{App, shell, shell::close_guard};

pub(super) fn keep_open(app: &mut App) -> bool {
    let last = is_last_window(shell::window_id(app));
    let Some(refusal) = close_guard::refusal(app, last) else {
        return false;
    };
    let Some(ui) = UI.with(|ui| ui.get().copied()) else {
        return false;
    };
    shell::detach_backdrop_blur(app);
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
