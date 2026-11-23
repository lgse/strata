// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_components::preview_drawer::{Arrangement, DrawerRules, solve};

use super::session;
use crate::{App, shell};

const RAIL_RELEASE_MARGIN: f32 = 24.0;

pub(super) fn arrangement(app: &App) -> Arrangement {
    if session::browsing_columns(app) {
        let fit = crate::columns::view(app).fit(app, &DrawerRules::STRATA);
        Arrangement::Columns(fit)
    } else if session::browsing_icons(app) {
        Arrangement::Icons
    } else {
        Arrangement::List
    }
}

pub(crate) fn draws_divider(app: &App) -> bool {
    let Some(available) = app.preview.last_split_width else {
        return false;
    };
    let arrangement = arrangement(app);
    let solved = solve(
        &DrawerRules::STRATA,
        &arrangement,
        available,
        app.preview.drawer.manual_width(),
    );
    let has_body = app.preview.current.is_some() || app.preview.reserves_empty_preview(app);
    if app.preview.drawer.is_open() && has_body && solved.fits {
        return true;
    }
    let keeps_slot = app.preview.drawer.is_reserved() || app.preview.drawer.is_open();
    matches!(arrangement, Arrangement::Columns(_)) && keeps_slot && solved.empty_slot > 0.0
}

pub(super) fn on_split_layout(app: &mut App, rect: [f32; 4], tx: &mut Tx) {
    let width = rect[2];
    if !width.is_finite() || width <= 0.0 {
        tx.live_only();
        return;
    }
    let width_changed = app
        .preview
        .last_split_width
        .is_none_or(|prev| (prev - width).abs() > 0.5);
    app.preview.last_split_width = Some(width);

    publish_column_width_cap(app, width);
    let railed = sync_sidebar_rail(app, width);
    let busy = app.preview.drawer.is_dragging() || app.columns.is_resizing();
    if width_changed && session::browsing_columns(app) && !busy {
        app.columns.reveal_active(&app.look, tx);
        return;
    }
    if !width_changed && !railed {
        tx.live_only();
    }
}

fn publish_column_width_cap(app: &mut App, split_width: f32) {
    let line = if app.preview.drawer.is_open() || app.preview.reserves_column_space(app) {
        DrawerRules::STRATA.line
    } else {
        0.0
    };
    let cap = (split_width - line - shell::sidebar_held_back(app)).max(0.0);
    app.columns.set_width_cap(Some(cap));
}

fn sync_sidebar_rail(app: &mut App, split_width: f32) -> bool {
    if !app.sidebar.is_started() || !app.sidebar.chosen_full() {
        return false;
    }
    if app.preview.drawer.is_dragging() || app.columns.is_resizing() {
        return false;
    }
    let rules = &DrawerRules::STRATA;
    let room_after_full_sidebar = split_width - shell::sidebar_held_back(app);
    let needs = if app.preview.preview_present(app) {
        let files_need = if session::browsing_columns(app) {
            crate::columns::view(app)
                .fit(app, rules)
                .keep_focused_standard
                .max(rules.files_min)
        } else {
            rules.files_min
        };
        files_need + rules.line + app.preview.drawer.demand(rules)
    } else {
        rules.files_min
    };
    let margin = if app.sidebar.is_auto_railed() {
        RAIL_RELEASE_MARGIN
    } else {
        0.0
    };
    let wants_rail = room_after_full_sidebar < needs + margin;
    app.sidebar.set_auto_rail(wants_rail)
}
