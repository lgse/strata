//! [`ViewportState`]: the mode, the level, the pan and what they're measured against.

use rune::{Drag, Live, Phase};
use strata_kit::look::Ladder;

use super::zoom::{
    self, Mode, ZoomLimits, ZoomReport, clamp_pan, default_steps, drawn_size, mode_level, overflows,
};

/// One viewport's zoom and pan. The host keeps it in its state and hands the viewport an
/// accessor, as rune-kit's Table takes its state.
///
/// The content's size arrives as data ([`ViewportState::set_content`]); the frame's size and
/// the display scale arrive from the viewport's own layout. Until both are known,
/// [`ViewportState::level`] is `None` and the zoom methods do nothing.
///
/// The pan lives in a `rune::Live` the content element is bound to, so panning (drag, wheel,
/// arrows) moves the content without re-running the view. That makes the state shared with
/// what it draws: it isn't `Clone`, and replacing it with a new one is how a host starts over.
pub struct ViewportState {
    default_mode: Mode,
    mode: Mode,
    /// The level in [`Mode::Free`].
    free: f32,
    pan: [f32; 2],
    pub(super) live_pan: Live<[f32; 2]>,
    frame: Option<[f32; 2]>,
    scale: f32,
    content: Option<[f32; 2]>,
    steps: Ladder,
    limits: ZoomLimits,
    /// The pan when the current drag pressed.
    drag_from: Option<[f32; 2]>,
    /// The last level reported, so a report goes out only when the level changes.
    reported: Option<f32>,
}

impl std::fmt::Debug for ViewportState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The live pan is the same value as `pan`.
        f.debug_struct("ViewportState")
            .field("mode", &self.mode)
            .field("default_mode", &self.default_mode)
            .field("free", &self.free)
            .field("pan", &self.pan)
            .field("frame", &self.frame)
            .field("scale", &self.scale)
            .field("content", &self.content)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl Default for ViewportState {
    fn default() -> ViewportState {
        ViewportState::new(Mode::Fit)
    }
}

impl ViewportState {
    /// A viewport that starts in, and resets to, `default_mode`. [`Mode::Free`] isn't a default:
    /// it means Fit here.
    pub fn new(default_mode: Mode) -> ViewportState {
        let default_mode = match default_mode {
            Mode::Free => Mode::Fit,
            m => m,
        };
        let steps = default_steps();
        ViewportState {
            default_mode,
            mode: default_mode,
            free: 1.0,
            pan: [0.0, 0.0],
            live_pan: Live::new([0.0, 0.0]),
            frame: None,
            scale: 1.0,
            content: None,
            limits: ZoomLimits::of(&steps),
            steps,
            drag_from: None,
            reported: None,
        }
    }

    /// Steps for `+` and `-` ([`zoom::default_steps`] unless set). The limits follow them unless
    /// [`ViewportState::with_limits`] comes after.
    pub fn with_steps(mut self, steps: Ladder) -> ViewportState {
        self.limits = ZoomLimits::of(&steps);
        self.steps = steps;
        self
    }

    /// The range of levels (the steps' range unless set). It widens to take in fit when the
    /// content is larger or smaller than the limits allow.
    pub fn with_limits(mut self, limits: ZoomLimits) -> ViewportState {
        self.limits = limits;
        self
    }

    /// The content's own size in content pixels (an image's width and height), or `None` while
    /// there is none. A different size starts over in the default mode, centred; the same size
    /// keeps the zoom, as when a video's next frame or a page's sharper render arrives.
    ///
    /// A new size changes the level, and the host calls this outside the viewport's handlers, so
    /// the report comes back here: forward it to whatever the host's `on_zoom` does. (The
    /// viewport can't send it from its next layout: the frame keeps its size, so no layout
    /// fires.) Before the first layout there is no level yet and this returns `None`; the first
    /// layout then reports through `on_zoom`.
    pub fn set_content(&mut self, size: Option<[f32; 2]>) -> Option<ZoomReport> {
        let size =
            size.filter(|s| s[0] > 0.0 && s[1] > 0.0 && s[0].is_finite() && s[1].is_finite());
        if size == self.content {
            return None;
        }
        self.content = size;
        self.mode = self.default_mode;
        self.set_pan([0.0, 0.0]);
        self.reported = None;
        self.changed()
    }

    pub fn content(&self) -> Option<[f32; 2]> {
        self.content
    }

    /// The frame's size in points, once the viewport has been laid out.
    pub fn frame(&self) -> Option<[f32; 2]> {
        self.frame
    }

    /// The display scale the viewport last drew at (device pixels per point).
    pub fn scale(&self) -> f32 {
        self.scale
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn default_mode(&self) -> Mode {
        self.default_mode
    }

    pub fn steps(&self) -> &Ladder {
        &self.steps
    }

    pub fn limits(&self) -> ZoomLimits {
        self.limits
    }

    /// The level shown: device pixels per content pixel, the frame's fit resolved. `None` until
    /// the content and the frame are known.
    pub fn level(&self) -> Option<f32> {
        let (frame, content) = (self.frame?, self.content?);
        Some(mode_level(self.mode, frame, content, self.scale, self.free))
    }

    /// [`ViewportState::level`] at display `scale`: what the view draws, which may run before a
    /// handler has seen a new scale.
    pub(super) fn level_at(&self, scale: f32) -> Option<f32> {
        let (frame, content) = (self.frame?, self.content?);
        Some(mode_level(self.mode, frame, content, scale, self.free))
    }

    /// The content's drawn size in points.
    pub fn drawn(&self) -> Option<[f32; 2]> {
        Some(drawn_size(self.content?, self.level()?, self.scale))
    }

    /// The pan: the content's offset from centred, in points.
    pub fn pan(&self) -> [f32; 2] {
        self.pan
    }

    /// Whether the content overflows the frame, per axis: where it can be panned.
    pub fn overflow(&self) -> [bool; 2] {
        match (self.drawn(), self.frame) {
            (Some(d), Some(f)) => overflows(d, f),
            _ => [false, false],
        }
    }

    /// The content pixel under `point`, a point in the frame from its top-left corner. It may lie
    /// outside the content.
    pub fn content_at(&self, point: [f32; 2]) -> Option<[f32; 2]> {
        let (frame, content, drawn) = (self.frame?, self.content?, self.drawn()?);
        let left = (frame[0] - drawn[0]) / 2.0 + self.pan[0];
        let top = (frame[1] - drawn[1]) / 2.0 + self.pan[1];
        Some([
            (point[0] - left) / drawn[0] * content[0],
            (point[1] - top) / drawn[1] * content[1],
        ])
    }

    /// The current level as a report, whether or not it changed.
    pub fn report(&self) -> Option<ZoomReport> {
        Some(ZoomReport::new(self.level()?, self.mode, self.content?))
    }

    /// Changes the mode, centred. Reports when the level changes.
    pub fn set_mode(&mut self, mode: Mode) -> Option<ZoomReport> {
        if mode == Mode::Free {
            self.free = self.level().unwrap_or(self.free);
        }
        self.mode = mode;
        self.set_pan([0.0, 0.0]);
        self.changed()
    }

    /// Back to the default mode, centred.
    pub fn reset(&mut self) -> Option<ZoomReport> {
        self.set_mode(self.default_mode)
    }

    /// The lowest and highest levels the zoom reaches now: the limits, widened to take in fit,
    /// so a content far larger or smaller than the limits allow can still be fitted and zoomed
    /// back to fit.
    pub fn range(&self) -> Option<ZoomLimits> {
        let fit = mode_level(Mode::Fit, self.frame?, self.content?, self.scale, 1.0);
        Some(self.limits.widened_to(fit))
    }

    /// Zooms `delta` steps, about the frame's centre. A step never goes the wrong way: from a
    /// level outside the steps (a huge image's fit below the first step, a tiny one's above the
    /// last), `-` and `+` do nothing rather than jump to the nearest end.
    pub fn zoom_step(&mut self, delta: i32) -> Option<ZoomReport> {
        let now = self.level()?;
        let next = self.steps.step(now, delta);
        let next = if delta > 0 {
            next.max(now)
        } else {
            next.min(now)
        };
        self.zoom_to(next, None)
    }

    /// Zooms by `factor`, keeping the content point under `about` (a frame point from its
    /// top-left corner; the centre when `None`) where it is.
    pub fn zoom_by(&mut self, factor: f32, about: Option<[f32; 2]>) -> Option<ZoomReport> {
        if !factor.is_finite() || factor <= 0.0 {
            return None;
        }
        let now = self.level()?;
        self.zoom_to(now * factor, about)
    }

    /// Zooms to `level` (held in [`ViewportState::range`]), as [`ViewportState::zoom_by`].
    pub fn zoom_to(&mut self, level: f32, about: Option<[f32; 2]>) -> Option<ZoomReport> {
        let (now, frame, range) = (self.level()?, self.frame?, self.range()?);
        let next = range.clamp(level);
        if (next - now).abs() <= now * 1e-4 {
            return None;
        }
        let centre = [frame[0] / 2.0, frame[1] / 2.0];
        let q = about.map_or([0.0, 0.0], |p| [p[0] - centre[0], p[1] - centre[1]]);
        let pan = zoom::zoom_about(self.pan, q, next / now);
        self.mode = Mode::Free;
        self.free = next;
        self.set_pan(pan);
        self.changed()
    }

    /// Pans by `d` points, held at the content's edges. True when the content moved.
    pub fn pan_by(&mut self, d: [f32; 2]) -> bool {
        let before = self.pan;
        self.set_pan([self.pan[0] + d[0], self.pan[1] + d[1]]);
        self.pan != before
    }

    /// Records the frame's size and the display scale from the viewport's layout, holds the pan
    /// at the content's new edges, and reports when the level changed (a resize in a mode that
    /// follows the frame, or the first layout).
    pub(super) fn set_frame(&mut self, frame: [f32; 2], scale: f32) -> Option<ZoomReport> {
        self.frame = Some(frame);
        self.set_scale(scale);
        self.set_pan(self.pan);
        self.changed()
    }

    /// Brings the display scale up to `scale`, the one a handler's view drew at, before the
    /// handler works on the level: a window moved to a display of another scale changes the
    /// level of every mode but Free. With the frame known it goes through
    /// [`ViewportState::set_frame`], so the pan holds at the new edges and a change is reported.
    pub(super) fn sync_scale(&mut self, scale: f32) -> Option<ZoomReport> {
        match self.frame {
            Some(frame) if scale != self.scale => self.set_frame(frame, scale),
            _ => {
                self.set_scale(scale);
                None
            }
        }
    }

    fn set_scale(&mut self, scale: f32) {
        if scale.is_finite() && scale > 0.0 {
            self.scale = scale;
        }
    }

    /// A pan drag: the content follows the pointer from where the press found it, and a
    /// cancelled drag puts it back.
    pub(super) fn drag(&mut self, d: &Drag) {
        if d.phase == Phase::Down {
            self.drag_from = Some(self.pan);
        }
        let Some(from) = self.drag_from else { return };
        if d.cancelled() {
            self.set_pan(from);
        } else {
            self.set_pan([from[0] + d.dx, from[1] + d.dy]);
        }
        if d.phase == Phase::Up || d.cancelled() {
            self.drag_from = None;
        }
    }

    /// Sets the pan, clamped, and the live value the content draws with.
    fn set_pan(&mut self, pan: [f32; 2]) {
        let pan = match (self.drawn(), self.frame) {
            (Some(d), Some(f)) => clamp_pan(pan, d, f),
            _ => [0.0, 0.0],
        };
        if pan != self.pan {
            self.pan = pan;
        }
        self.live_pan.set(self.pan);
    }

    /// A report when the level differs from the last one reported.
    fn changed(&mut self) -> Option<ZoomReport> {
        let report = self.report()?;
        let same = self
            .reported
            .is_some_and(|r| (r - report.level).abs() <= r * 1e-4);
        if same {
            return None;
        }
        self.reported = Some(report.level);
        Some(report)
    }
}
