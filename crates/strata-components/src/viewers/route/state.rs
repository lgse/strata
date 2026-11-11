//! The route view's state: the route as data, the marker's place on it, and playback.
//!
//! Playback runs on the poll's clock: the host calls [`RouteState::tick`] from its poll on every
//! frame, as it does `Collection::tick`. A tick moves the markers through `Live` values, which draw
//! with no view commit. It reports a change, and so a commit, only when the text the view shows
//! changes (the distance to a tenth, the height to a metre), and then at most every
//! [`TEXT_INTERVAL`]: on a long route the tenths change faster than frames, and a commit a frame
//! would rebuild the whole window for a label (C1d's auto-scroll applies at about 10 Hz the same
//! way). Whatever stops playback (its end, a pause, a seek, a hover) commits the exact text at
//! once.
//!
//! That holds for what asks the view to commit: [`RouteState::tick`]'s answer, the view's own
//! handlers and [`RouteState::command_tx`]. [`RouteState::pause`] and [`RouteState::command`]
//! only change the state, so a host that calls them directly commits itself: from its poll it
//! returns true, and a handler mustn't say `live_only`. Otherwise the text stays behind the
//! marker until something else commits.

use std::cell::RefCell;

use rune::{Live, Tx};

use super::trace::{self, RouteSample};
use crate::viewers::layout::Position;
use crate::viewers::viewport::{Mode, ViewportState};

/// How long playback takes from start to end, whatever the route's length (Strata's
/// `PLAYBACK_SECONDS`): the marker moves at a steady ground-distance pace.
pub const PLAYBACK_SECONDS: f64 = 15.0;

/// The shortest time between two text commits during playback, in seconds: about 10 Hz.
pub const TEXT_INTERVAL: f64 = 0.1;

/// A route as Strata's helper returns it (Strata's `RoutePreview`, less its PNG images: the route
/// picture arrives as the host's `Pixels`, and the profile is drawn here from the samples).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RouteData {
    /// The display name.
    pub name: String,
    /// The format badge ("GPX").
    pub format: String,
    /// Its place among the route files around it, if the host counts one.
    pub position: Option<Position>,
    /// The route's length.
    pub distance_m: f64,
    /// The elapsed time between its first and last timestamps, if it has two.
    pub duration_s: Option<u64>,
    /// The lowest and highest recorded heights, if any pair of connected points has one.
    pub elevation: Option<[f64; 2]>,
    pub samples: Vec<RouteSample>,
    /// The route picture's size in pixels (Strata's 800 × 600).
    pub image_px: [f32; 2],
    /// The space the samples' `elevation_y` is in (Strata's 800 × 70).
    pub profile_px: [f32; 2],
}

/// Distances in kilometres and heights in metres, or miles and feet (Settings → Units, the
/// host's preference).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Units {
    #[default]
    Metric,
    Imperial,
}

/// How a seek moves the marker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seek {
    Start,
    End,
    /// By this many seconds of playback (a third of the route is 5 s).
    By(f64),
}

/// What the media keys ask of a route. Mute and volume aren't among them: they do nothing to a
/// route, and the host doesn't send them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RouteCommand {
    PlayPause,
    Seek(Seek),
}

/// Which picture a pointer is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Plot {
    Route,
    Profile,
}

/// The words the route view shows and reads. Strata passes translated text and its keymap's
/// keys; the defaults are Strata's English with the default key map.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RouteLabels {
    pub route: String,
    pub profile: String,
    pub heading: String,
    pub play: String,
    /// The play button's tooltip: "Play/Pause" and its shortcut.
    pub play_tooltip: String,
    pub distance: fn(f64, Units) -> String,
    pub height: fn(f64, Units) -> String,
    pub range: fn([f64; 2], Units) -> String,
    pub duration: fn(u64) -> String,
}

impl Default for RouteLabels {
    fn default() -> Self {
        RouteLabels {
            route: "Route preview".into(),
            profile: "Elevation profile".into(),
            heading: "Elevation".into(),
            play: "Play or pause".into(),
            play_tooltip: "Play/Pause (Ctrl+Alt+Space)".into(),
            distance: |m, units| match units {
                Units::Metric => format!("{:.1} km", m / 1000.0),
                Units::Imperial => format!("{:.1} mi", m / 1609.344),
            },
            height: |m, units| match units {
                Units::Metric => format!("{:.0} m", m),
                Units::Imperial => format!("{:.0} ft", m / 0.3048),
            },
            range: |[low, high], units| match units {
                Units::Metric => format!("{low:.0}–{high:.0} m"),
                Units::Imperial => format!("{:.0}–{:.0} ft", low / 0.3048, high / 0.3048),
            },
            duration: |seconds| {
                let (h, m, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
                if h > 0 {
                    format!("{h}:{m:02}:{s:02}")
                } else {
                    format!("{m}:{s:02}")
                }
            },
        }
    }
}

/// What the view shows that a marker move can change: a move that leaves it alone commits
/// nothing.
#[derive(Clone, Debug, PartialEq)]
struct Shown {
    distance: String,
    height: Option<String>,
    playing: bool,
}

/// The route view's state. The host makes one per route ([`RouteState::new`]), sets the route
/// picture's `Pixels`, calls [`RouteState::tick`] from its poll, sends the media keys to
/// [`RouteState::command`], and [`RouteState::pause`]s when the preview hides.
pub struct RouteState {
    /// The route picture's viewport (Fit, fixed).
    pub viewport: ViewportState,
    data: RouteData,
    /// The profile's polylines, in profile pixels, built once.
    profile_lines: Vec<Vec<[f32; 2]>>,
    labels: RouteLabels,
    units: Units,
    position: Option<RouteSample>,
    playing: bool,
    last_tick: Option<f64>,
    last_pointer: Option<(Plot, [f32; 2])>,
    /// The profile's laid-out size, in points.
    profile_size: [f32; 2],
    route_marker: Live<[f32; 2]>,
    profile_marker: Live<[f32; 2]>,
    profile_cursor: Live<[f32; 2]>,
    /// What the view last built shows ([`RouteState::note_built`]).
    built: RefCell<Option<Shown>>,
    /// When playback last asked to commit its text, on the host's clock.
    text_at: f64,
}

impl std::fmt::Debug for RouteState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouteState")
            .field("name", &self.data.name)
            .field("units", &self.units)
            .field("position", &self.position)
            .field("playing", &self.playing)
            .finish_non_exhaustive()
    }
}

impl Default for RouteState {
    fn default() -> Self {
        RouteState::new(RouteData::default(), RouteLabels::default())
    }
}

impl RouteState {
    /// The route, with the marker at its start.
    pub fn new(data: RouteData, labels: RouteLabels) -> RouteState {
        let mut viewport = ViewportState::new(Mode::Fit);
        viewport.set_content(Some(data.image_px));
        RouteState {
            viewport,
            position: data.samples.first().copied(),
            profile_lines: super::plot::profile_lines(&data),
            data,
            labels,
            units: Units::Metric,
            playing: false,
            last_tick: None,
            last_pointer: None,
            profile_size: [0.0, 0.0],
            route_marker: Live::new([0.0, 0.0]),
            profile_marker: Live::new([0.0, 0.0]),
            profile_cursor: Live::new([0.0, 0.0]),
            built: RefCell::new(None),
            text_at: f64::NEG_INFINITY,
        }
    }

    pub fn data(&self) -> &RouteData {
        &self.data
    }

    pub fn labels(&self) -> &RouteLabels {
        &self.labels
    }

    pub(super) fn profile_lines(&self) -> &[Vec<[f32; 2]>] {
        &self.profile_lines
    }

    pub fn units(&self) -> Units {
        self.units
    }

    /// Changes the units, keeping the marker and playback where they are.
    pub fn set_units(&mut self, units: Units) {
        self.units = units;
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Whether there is anything to play: a route of zero length has an inactive play button.
    pub fn playable(&self) -> bool {
        self.data.distance_m > 0.0
    }

    /// The marker's distance from the start.
    pub fn distance(&self) -> f64 {
        self.position.map_or(0.0, |p| p.distance_m)
    }

    /// The marker's distance, as the control row and both pictures' descriptions read it.
    pub fn distance_text(&self) -> String {
        (self.labels.distance)(self.distance(), self.units)
    }

    pub fn total_text(&self) -> String {
        (self.labels.distance)(self.data.distance_m, self.units)
    }

    /// The marker's height, while it has one.
    pub fn height_text(&self) -> Option<String> {
        let y = self.position?.elevation_y?;
        let range = self.data.elevation?;
        let h = trace::profile_elevation(y, range, f64::from(self.data.profile_px[1]));
        Some((self.labels.height)(h, self.units))
    }

    pub fn range_text(&self) -> Option<String> {
        self.data
            .elevation
            .map(|r| (self.labels.range)(r, self.units))
    }

    pub fn duration_text(&self) -> Option<String> {
        self.data.duration_s.map(self.labels.duration)
    }

    fn shown(&self) -> Shown {
        Shown {
            distance: self.distance_text(),
            height: self.height_text(),
            playing: self.playing,
        }
    }

    /// The view built what the state shows now: [`RouteView`](super::RouteView) calls it.
    pub(super) fn note_built(&self) {
        *self.built.borrow_mut() = Some(self.shown());
    }

    /// What the view shows is behind the state.
    fn stale(&self) -> bool {
        self.built.borrow().as_ref() != Some(&self.shown())
    }

    /// Play or pause. Play at the route's end starts again from the start.
    pub fn toggle(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    fn play(&mut self) {
        if !self.playable() {
            return;
        }
        if self.distance() >= self.data.distance_m {
            self.position = self.data.samples.first().copied();
        }
        self.playing = true;
        self.last_tick = None;
        self.place_markers();
    }

    /// Stops playback where the marker is: another file was selected, the preview closed or
    /// hid in a narrow window. It doesn't commit: the caller does (module docs).
    pub fn pause(&mut self) {
        self.playing = false;
        self.last_tick = None;
    }

    /// A media key. False when it does nothing here (a route of zero length). It doesn't
    /// commit: the caller does (module docs), or calls [`RouteState::command_tx`].
    pub fn command(&mut self, command: RouteCommand) -> bool {
        if !self.playable() {
            return false;
        }
        match command {
            RouteCommand::PlayPause => self.toggle(),
            RouteCommand::Seek(seek) => {
                self.pause();
                let total = self.data.distance_m;
                let to = match seek {
                    Seek::Start => 0.0,
                    Seek::End => total,
                    Seek::By(seconds) => self.distance() + seconds / PLAYBACK_SECONDS * total,
                };
                self.move_to(to.clamp(0.0, total));
            }
        }
        true
    }

    /// The command, from a handler: commits only if what the view shows is behind.
    pub fn command_tx(&mut self, command: RouteCommand, tx: &mut Tx) -> bool {
        let taken = self.command(command);
        if taken && !self.stale() {
            tx.live_only();
        }
        taken
    }

    /// One frame of playback at `now` seconds on the host's clock. True when the view should
    /// commit (the host's poll returns it): the text it shows is behind, and [`TEXT_INTERVAL`]
    /// has passed since playback last asked, or playback just reached the end. The markers move
    /// either way. While playing it asks for the next frame.
    pub fn tick(&mut self, now: f64, tx: &mut Tx) -> bool {
        if !self.playing {
            return false;
        }
        tx.wake_after(std::time::Duration::from_millis(16));
        if let Some(last) = self.last_tick.replace(now) {
            let total = self.data.distance_m;
            let to = self.distance() + (now - last).max(0.0) / PLAYBACK_SECONDS * total;
            self.move_to(to.min(total));
            if to >= total {
                self.pause();
                return self.stale();
            }
        }
        if self.stale() && now - self.text_at >= TEXT_INTERVAL {
            self.text_at = now;
            return true;
        }
        false
    }

    fn move_to(&mut self, distance: f64) {
        self.position = trace::at_distance(&self.data.samples, distance);
        self.place_markers();
    }

    /// The pointer moved over a picture, at `point` in that picture's own points: playback
    /// pauses and both markers go to the point it picks. True when what the view shows is
    /// behind, so the handler commits.
    pub fn hover(&mut self, plot: Plot, point: [f32; 2]) -> bool {
        if self.last_pointer.replace((plot, point)) == Some((plot, point)) {
            return false;
        }
        self.pause();
        let picked = match plot {
            Plot::Route => self.viewport.content_at(point).and_then(|[x, y]| {
                let [w, h] = self.data.image_px;
                let inside = (0.0..=w).contains(&x) && (0.0..=h).contains(&y);
                let k = self.points_per_px()?;
                inside.then(|| {
                    trace::nearest(
                        &self.data.samples,
                        [f64::from(x), f64::from(y)],
                        self.position.map(|p| p.distance_m),
                        2.0 / f64::from(k),
                    )
                })?
            }),
            Plot::Profile => {
                let [w, _] = self.profile_size;
                (w > 0.0)
                    .then(|| {
                        let pw = f64::from(self.data.profile_px[0]);
                        let x = f64::from(point[0] / w) * pw;
                        let d = trace::profile_distance(x, self.data.distance_m, pw);
                        trace::at_distance(&self.data.samples, d)
                    })
                    .flatten()
            }
        };
        if let Some(p) = picked {
            self.position = Some(p);
        }
        self.place_markers();
        self.stale()
    }

    /// The pointer left the pictures: the next move over one counts again.
    pub fn pointer_left(&mut self) {
        self.last_pointer = None;
    }

    /// The profile's laid-out size.
    pub fn set_profile_size(&mut self, size: [f32; 2]) {
        if self.profile_size != size {
            self.profile_size = size;
            self.place_markers();
        }
    }

    /// Route points per route pixel, once laid out.
    fn points_per_px(&self) -> Option<f32> {
        let level = self.viewport.level()?;
        Some(level / self.viewport.scale())
    }

    /// Puts the markers where the marker's sample draws: on the route picture in its content's
    /// points, and on the profile in its own.
    pub fn place_markers(&self) {
        let Some(p) = self.position else { return };
        if let Some(k) = self.points_per_px() {
            self.route_marker
                .set([p.position[0] * k, p.position[1] * k]);
        }
        let [w, h] = self.profile_size;
        let [pw, ph] = self.data.profile_px;
        if w > 0.0 && pw > 0.0 && ph > 0.0 {
            let x =
                trace::profile_x(p.distance_m, self.data.distance_m, f64::from(pw)) as f32 * w / pw;
            let y = p.elevation_y.map_or(-100.0, |y| y * h / ph);
            self.profile_marker.set([x, y]);
            self.profile_cursor.set([x, 0.0]);
        }
    }

    pub(super) fn route_marker(&self) -> &Live<[f32; 2]> {
        &self.route_marker
    }

    pub(super) fn profile_marker(&self) -> &Live<[f32; 2]> {
        &self.profile_marker
    }

    pub(super) fn profile_cursor(&self) -> &Live<[f32; 2]> {
        &self.profile_cursor
    }
}
