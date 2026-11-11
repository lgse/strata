//! The playback clock the player follows, and a fake one that plays on the host's clock.

use std::time::Duration;

use super::intent::MediaIntent;

/// What playback is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PlaybackState {
    /// Nothing to play yet: the decoder starts, or the audio sink waits for its first buffer.
    #[default]
    Loading,
    Paused,
    Playing,
    /// A seek runs: [`PlaybackClock::position`] is its target, and the frame on screen is
    /// still the old one.
    Seeking,
    /// Playback reached the end and stopped there.
    Ended,
}

/// The clock playback follows: P7's audio sink implements it, and so does [`FakeClock`].
///
/// The player only reads it, once a frame from [`MediaState::tick`](super::MediaState::tick).
/// It asks for play, pause and seeks through [`MediaIntent`]s, which the host applies.
///
/// **Wakes are the clock's.** The player asks for a wake each frame only while
/// [`PlaybackState::Playing`], while the waveform grows in or the spectrum's bars fall, and
/// while the stand-in fades. A clock that changes on its own otherwise (a seek landing, a
/// start finishing [`PlaybackState::Loading`], an underrun, the end of a stream) must wake the
/// host for each state change, so its poll runs the tick that shows it: P7 through the app's
/// setup waker (or `AppHandle::update`), [`FakeClock`] through [`FakeClock::next_due`] and the
/// host's `Tx::wake_after`. A clock that never does leaves the view on the old state in a
/// resting window.
pub trait PlaybackClock {
    /// Where playback is: while seeking, the seek's target.
    fn position(&self) -> Duration;
    /// The media's length, `None` while unknown (seeking is off until it is known).
    fn duration(&self) -> Option<Duration>;
    fn state(&self) -> PlaybackState;
    /// Whether a seek can run. By default, whenever the duration is known.
    fn seekable(&self) -> bool {
        self.duration().is_some()
    }
}

/// A clock that plays on the host's clock (seconds, as `poll` gets them): for the gallery,
/// tests, and hosts without P7 yet.
///
/// ```ignore
/// clock.advance(now);                       // in poll, before MediaState::tick
/// clock.apply(&intent);                     // for each intent the player sends
/// ```
///
/// Play and seeks can take a while ([`FakeClock::latency`]): until then it reports
/// [`PlaybackState::Loading`] or [`PlaybackState::Seeking`], as a real sink does.
#[derive(Clone, Debug)]
pub struct FakeClock {
    duration: Option<Duration>,
    /// Where playback stood at `since`.
    at: f64,
    since: f64,
    now: f64,
    state: PlaybackState,
    /// What the state becomes at the time due, after a start or a seek.
    pending: Option<(f64, PlaybackState)>,
    start_latency: f64,
    seek_latency: f64,
}

impl FakeClock {
    /// A clip of `duration`, paused at its start.
    pub fn new(duration: Option<Duration>) -> FakeClock {
        FakeClock {
            duration,
            at: 0.0,
            since: 0.0,
            now: 0.0,
            state: PlaybackState::Paused,
            pending: None,
            start_latency: 0.0,
            seek_latency: 0.0,
        }
    }

    /// The time a play from Loading, and a seek, take, in seconds.
    pub fn latency(mut self, start: f64, seek: f64) -> FakeClock {
        self.start_latency = start;
        self.seek_latency = seek;
        self
    }

    /// Starts loading, as a fresh decoder does: [`PlaybackState::Loading`] until a play.
    pub fn loading(mut self) -> FakeClock {
        self.state = PlaybackState::Loading;
        self
    }

    /// The duration becomes known (or unknown).
    pub fn set_duration(&mut self, duration: Option<Duration>) {
        self.duration = duration;
    }

    /// Puts playback at `position`, paused, at once (a fixture's starting point).
    pub fn set_position(&mut self, position: Duration) {
        self.at = position.as_secs_f64();
        self.since = self.now;
        if self.state == PlaybackState::Playing {
            self.state = PlaybackState::Paused;
        }
    }

    /// Plays from where it is, at once.
    pub fn play_now(&mut self) {
        self.at = self.seconds();
        self.since = self.now;
        self.state = PlaybackState::Playing;
        self.pending = None;
    }

    /// The host's clock moved to `now`: playback advances, ends at the duration, and a start
    /// or a seek that came due lands.
    pub fn advance(&mut self, now: f64) {
        if let Some((due, next)) = self.pending
            && now >= due
        {
            // Playback resumes from the seek's target at the moment the seek landed.
            self.at = self.seconds_at(due);
            self.since = due;
            self.state = next;
            self.pending = None;
        }
        self.now = now;
        if self.state == PlaybackState::Playing
            && let Some(d) = self.duration
            && self.seconds() >= d.as_secs_f64()
        {
            self.at = d.as_secs_f64();
            self.since = now;
            self.state = PlaybackState::Ended;
        }
    }

    /// When a start or seek comes due, on the host's clock: a host polls then.
    pub fn next_due(&self) -> Option<f64> {
        self.pending.map(|(due, _)| due)
    }

    /// Applies what the player asked. Volume, steps, hand-off and the rest are the host's, so
    /// they do nothing here.
    pub fn apply(&mut self, intent: &MediaIntent) {
        match *intent {
            MediaIntent::Play => self.play(),
            MediaIntent::Pause => self.pause(),
            MediaIntent::Seek(to) => self.seek(to),
            _ => {}
        }
    }

    fn play(&mut self) {
        let now = self.now;
        match self.state {
            PlaybackState::Playing => {}
            PlaybackState::Seeking => {
                if let Some((_, next)) = &mut self.pending {
                    *next = PlaybackState::Playing;
                }
            }
            PlaybackState::Loading if self.start_latency > 0.0 => {
                self.pending = Some((now + self.start_latency, PlaybackState::Playing));
            }
            PlaybackState::Ended => {
                self.at = 0.0;
                self.since = now;
                self.state = PlaybackState::Playing;
            }
            _ => {
                self.at = self.seconds();
                self.since = now;
                self.state = PlaybackState::Playing;
            }
        }
    }

    fn pause(&mut self) {
        match self.state {
            PlaybackState::Playing => {
                self.at = self.seconds();
                self.since = self.now;
                self.state = PlaybackState::Paused;
            }
            PlaybackState::Seeking | PlaybackState::Loading => {
                if let Some((_, next)) = &mut self.pending {
                    *next = PlaybackState::Paused;
                } else {
                    self.state = PlaybackState::Paused;
                }
            }
            _ => {}
        }
    }

    fn seek(&mut self, to: Duration) {
        if !self.seekable() {
            return;
        }
        let resume = match (self.state, self.pending) {
            (PlaybackState::Playing, _) => PlaybackState::Playing,
            (PlaybackState::Seeking | PlaybackState::Loading, Some((_, next))) => next,
            _ => PlaybackState::Paused,
        };
        self.at = to.as_secs_f64();
        self.since = self.now;
        if self.seek_latency > 0.0 {
            self.state = PlaybackState::Seeking;
            self.pending = Some((self.now + self.seek_latency, resume));
        } else {
            self.state = resume;
            self.pending = None;
        }
    }

    fn seconds(&self) -> f64 {
        self.seconds_at(self.now)
    }

    fn seconds_at(&self, t: f64) -> f64 {
        let moving = self.state == PlaybackState::Playing;
        let s = self.at
            + if moving {
                (t - self.since).max(0.0)
            } else {
                0.0
            };
        match self.duration {
            Some(d) => s.min(d.as_secs_f64()),
            None => s,
        }
    }
}

impl PlaybackClock for FakeClock {
    fn position(&self) -> Duration {
        Duration::from_secs_f64(self.seconds().max(0.0))
    }

    fn duration(&self) -> Option<Duration> {
        self.duration
    }

    fn state(&self) -> PlaybackState {
        self.state
    }
}
