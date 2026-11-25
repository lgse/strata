// SPDX-License-Identifier: MIT

use std::cell::Cell;

use rune::{Cx, El, Pixels, Tx};
use strata_components::viewers::{
    route::{RouteCommand, RouteData, RouteLabels, RouteSample, RouteState, RouteView, Units},
    viewport::ViewportState,
};
use strata_core::{
    i18n::{decimal, tr},
    model::FileEntry,
};
use strata_desktop::{
    keys::{Keys as Shown, bindings::MEDIA_PLAY_PAUSE},
    settings::UnitSystem,
};
use strata_fs::services::route_preview::{RouteFormat, RoutePreview};

use super::{
    labels,
    viewer::{self, Viewer},
};
use crate::{App, location::labels::with_keys, shell};

/// Key of the route viewer inside the preview drawer.
pub(crate) const ROUTE_KEY: &str = "preview-route";

const PROFILE_PX: [f32; 2] = [800.0, 70.0];

thread_local! {
    static NEXT_DECODE: Cell<u64> = const { Cell::new(1) };
}

pub(super) struct RouteViewer {
    pub(super) state: RouteState,
    pixels: Pixels,
}

/// The decoded route picture, and the profile's size.
struct Decoded {
    image: image::RgbaImage,
    profile_px: Option<[f32; 2]>,
}

/// A route arrived: the view shows once its pictures are decoded, `Viewer::Decoding` meanwhile.
pub(super) fn ready(app: &App, entry: FileEntry, preview: RoutePreview) -> Viewer {
    let token = NEXT_DECODE.with(|next| next.replace(next.get().wrapping_add(1)));
    let later = app.preview.loader.later.clone();
    let (png, profile) = (preview.png.clone(), preview.profile.clone());
    glib::MainContext::default().spawn_local(async move {
        let decoded = gio::spawn_blocking(move || decode(&png, profile.as_deref())).await;
        let decoded = decoded.unwrap_or_else(|_| Err(tr("Preview unavailable")));
        later.then(move |app: &mut App, _: &mut Tx| show(app, token, (entry, preview), decoded));
    });
    Viewer::Decoding(token)
}

fn decode(png: &[u8], profile: Option<&[u8]>) -> Result<Decoded, String> {
    let image = image::load_from_memory(png)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    let profile_px = profile
        .and_then(|png| image::load_from_memory(png).ok())
        .map(|profile| [profile.width() as f32, profile.height() as f32]);
    Ok(Decoded { image, profile_px })
}

fn show(
    app: &mut App,
    token: u64,
    (entry, preview): (FileEntry, RoutePreview),
    decoded: Result<Decoded, String>,
) -> bool {
    if !matches!(app.preview.viewer, Viewer::Decoding(shown) if shown == token) {
        return false;
    }
    app.preview.viewer = match decoded {
        Ok(decoded) => {
            let data = route_data(app, &entry, preview, &decoded);
            let pixels = Pixels::new();
            let (width, height) = decoded.image.dimensions();
            shell::upload_rgba(
                app.window.image_cache().map(std::sync::Arc::as_ref),
                &pixels,
                width,
                height,
                decoded.image.into_raw(),
            );
            let mut state = RouteState::new(data, route_labels());
            state.set_units(units(app));
            Viewer::Route(Box::new(RouteViewer { state, pixels }))
        }
        Err(detail) => Viewer::Message {
            title: tr("Preview unavailable"),
            detail: Some(detail),
            can_open: false,
        },
    };
    true
}

fn route_data(app: &App, entry: &FileEntry, preview: RoutePreview, decoded: &Decoded) -> RouteData {
    let is_route = |file: &FileEntry| RouteFormat::for_name(&file.native_name).is_some();
    let (width, height) = decoded.image.dimensions();
    RouteData {
        name: entry.display_name.clone(),
        format: RouteFormat::for_name(&entry.native_name)
            .map(|format| format.label().to_owned())
            .unwrap_or_default(),
        position: labels::listing_position(app, entry, is_route),
        distance_m: preview.distance_m,
        duration_s: preview.duration_seconds,
        elevation: preview.elevation,
        samples: preview
            .samples
            .into_iter()
            .map(|sample| RouteSample {
                position: sample.position,
                distance_m: sample.distance_m,
                elevation_y: sample.elevation_y,
                starts_segment: sample.starts_segment,
            })
            .collect(),
        image_px: [width as f32, height as f32],
        profile_px: decoded.profile_px.unwrap_or(PROFILE_PX),
    }
}

fn units(app: &App) -> Units {
    match shell::preferences(app).unit_system {
        UnitSystem::Metric => Units::Metric,
        UnitSystem::Imperial => Units::Imperial,
    }
}

/// Playback's frame and the saved units, from the preview's poll. True when the view changed.
pub(super) fn poll(app: &mut App, clock: f64, tx: &mut Tx) -> bool {
    let units = units(app);
    let Viewer::Route(route) = &mut app.preview.viewer else {
        return false;
    };
    let unit_changed = route.state.units() != units;
    if unit_changed {
        route.state.set_units(units);
    }
    route.state.tick(clock, tx) | unit_changed
}

/// A media key's command on the shown route (`super::transport`): false when it takes none.
pub(super) fn command(app: &mut App, command: RouteCommand) -> bool {
    match &mut app.preview.viewer {
        Viewer::Route(route) => route.state.command(command),
        _ => false,
    }
}

fn route_labels() -> RouteLabels {
    let mut labels = RouteLabels::default();
    labels.route = tr("Route preview");
    labels.profile = tr("Elevation profile");
    labels.heading = tr("Elevation");
    labels.play = tr("Play or pause");
    labels.play_tooltip = with_keys(
        "Play/Pause (%{keys})",
        None,
        Shown::One(MEDIA_PLAY_PAUSE.shown()),
    );
    labels.distance = distance;
    labels.height = height;
    labels.range = range;
    labels.duration = clock;
    labels.position = labels::position_caption;
    labels
}

fn distance(metres: f64, units: Units) -> String {
    match units {
        Units::Metric => rust_i18n::t!("%{distance} km", distance = decimal(metres / 1000.0, 1)),
        Units::Imperial => {
            rust_i18n::t!("%{distance} mi", distance = decimal(metres / 1609.344, 1))
        }
    }
    .into_owned()
}

fn height(metres: f64, units: Units) -> String {
    match units {
        Units::Metric => rust_i18n::t!("%{elevation} m", elevation = decimal(metres, 0)),
        Units::Imperial => {
            rust_i18n::t!("%{elevation} ft", elevation = decimal(metres / 0.3048, 0))
        }
    }
    .into_owned()
}

fn range([low, high]: [f64; 2], units: Units) -> String {
    match units {
        Units::Metric => rust_i18n::t!(
            "%{minimum}–%{maximum} m",
            minimum = decimal(low, 0),
            maximum = decimal(high, 0)
        ),
        Units::Imperial => rust_i18n::t!(
            "%{minimum}–%{maximum} ft",
            minimum = decimal(low / 0.3048, 0),
            maximum = decimal(high / 0.3048, 0)
        ),
    }
    .into_owned()
}

fn clock(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

pub(super) fn build_view(route: &RouteViewer, app: &App, cx: &mut Cx<App>) -> El {
    RouteView::new(&route.state, route_mut, viewport_mut)
        .key(ROUTE_KEY)
        .pixels(&route.pixels)
        .drag_source(crate::dnd::binder(app), crate::dnd::preview_drag_source())
        .build(&app.look, cx)
}

fn route_mut(app: &mut App) -> &mut RouteState {
    let preview = &mut app.preview;
    match &mut preview.viewer {
        Viewer::Route(route) => &mut route.state,
        _ => viewer::spare("route", &mut preview.detached.route),
    }
}

fn viewport_mut(app: &mut App) -> &mut ViewportState {
    let preview = &mut app.preview;
    match &mut preview.viewer {
        Viewer::Route(route) => &mut route.state.viewport,
        _ => viewer::spare("route", &mut preview.detached.viewport),
    }
}
