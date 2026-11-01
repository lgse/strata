// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{
    Arc, Mutex, OnceLock, PoisonError,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app::AppSrc;

const SAMPLE_RATE: u64 = 48_000;
const FRAME_BYTES: usize = 4;
const MAX_CHUNK_BYTES: usize = 6_400;
const MAX_CHUNK_NS: u64 = 33_333_334;

type Job = Box<dyn FnOnce() + Send>;

fn worker() -> Option<&'static mpsc::Sender<Job>> {
    static JOBS: OnceLock<Option<mpsc::Sender<Job>>> = OnceLock::new();
    JOBS.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("strata-audio".to_owned())
            .spawn(move || receiver.into_iter().for_each(|job| job()))
            .map_err(|error| tracing::warn!(%error, "unable to start the audio worker"))
            .ok()
            .map(|_| sender)
    })
    .as_ref()
}

enum Sink {
    Auto,
    #[cfg(test)]
    Bin {
        description: String,
        clock: Option<gst::Clock>,
    },
}

struct Parts {
    pipeline: gst::Pipeline,
    source: AppSrc,
    sink: gst::Element,
    volume: gst::Element,
}

impl Parts {
    fn build(sink: Sink, max_bytes: u64, max_time_ns: u64) -> Result<Self, String> {
        gst::init().map_err(|error| error.to_string())?;
        let pipeline = gst::Pipeline::new();
        let sink = match sink {
            #[cfg(not(test))]
            Sink::Auto => gst::ElementFactory::make("autoaudiosink")
                .build()
                .map_err(|error| error.to_string())?,
            #[cfg(test)]
            Sink::Auto => gst::ElementFactory::make("fakesink")
                .property("sync", true)
                .build()
                .map_err(|error| error.to_string())?,
            #[cfg(test)]
            Sink::Bin { description, clock } => {
                pipeline.use_clock(clock.as_ref());
                gst::parse::bin_from_description(&description, true)
                    .map_err(|error| error.to_string())?
                    .upcast()
            }
        };
        let source = AppSrc::builder().build();
        source.set_caps(Some(
            &gst::Caps::builder("audio/x-raw")
                .field("format", "S16LE")
                .field("layout", "interleaved")
                .field("rate", SAMPLE_RATE as i32)
                .field("channels", 2i32)
                .build(),
        ));
        source.set_format(gst::Format::Time);
        source.set_block(false);
        source.set_max_bytes(max_bytes);
        source.set_max_time(gst::ClockTime::from_nseconds(max_time_ns));
        source.set_property("do-timestamp", false);
        let convert = gst::ElementFactory::make("audioconvert")
            .build()
            .map_err(|error| error.to_string())?;
        let resample = gst::ElementFactory::make("audioresample")
            .build()
            .map_err(|error| error.to_string())?;
        let volume = gst::ElementFactory::make("volume")
            .build()
            .map_err(|error| error.to_string())?;
        let elements = [source.upcast_ref(), &convert, &resample, &volume, &sink];
        pipeline
            .add_many(elements)
            .map_err(|error| error.to_string())?;
        gst::Element::link_many(elements).map_err(|error| error.to_string())?;
        Ok(Self {
            pipeline,
            source,
            sink,
            volume,
        })
    }
}

#[derive(Clone, Copy)]
struct Gain {
    muted: bool,
    volume: f64,
    fade: f64,
}

impl Gain {
    fn set_audio(&mut self, muted: bool, volume: f64) {
        self.muted = muted;
        self.volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }

    fn set_fade(&mut self, fade: f64) {
        self.fade = if fade.is_finite() {
            fade.clamp(0.0, 1.0)
        } else {
            1.0
        };
    }

    fn apply(self, volume: &gst::Element) {
        volume.set_property("mute", self.muted);
        volume.set_property("volume", self.volume * self.fade);
    }
}

struct Shared {
    parts: OnceLock<Result<Parts, String>>,
    gain: Mutex<Gain>,
    opened: AtomicBool,
}

impl Shared {
    fn parts(&self) -> Option<&Parts> {
        self.parts.get()?.as_ref().ok()
    }
}

pub(super) struct PcmOutput {
    shared: Option<Arc<Shared>>,
    jobs: &'static mpsc::Sender<Job>,
    max_bytes: u64,
    max_time_ns: u64,
    frames: Cell<u64>,
    finished: Cell<bool>,
    failure: RefCell<Option<String>>,
    _thread: PhantomData<Rc<()>>,
}

impl PcmOutput {
    /// `lookahead` bounds the PCM queued in appsrc beyond what the sink holds.
    pub(super) fn new(muted: bool, volume: f64, lookahead: Duration) -> Result<Self, String> {
        Self::start(Sink::Auto, muted, volume, lookahead)
    }

    /// A sink bin driven by `clock`, or without a pipeline clock so that its
    /// position follows rendered data like an audio ring buffer.
    #[cfg(test)]
    pub(super) fn test_sink(
        muted: bool,
        volume: f64,
        lookahead: Duration,
        description: &str,
        clock: Option<&gst::Clock>,
    ) -> Result<Self, String> {
        let sink = Sink::Bin {
            description: description.to_owned(),
            clock: clock.cloned(),
        };
        Self::start(sink, muted, volume, lookahead)
    }

    fn start(sink: Sink, muted: bool, volume: f64, lookahead: Duration) -> Result<Self, String> {
        let jobs = worker().ok_or("The preview worker stopped unexpectedly.")?;
        let max_time_ns = u64::try_from(lookahead.as_nanos())
            .map_err(|_| "PCM lookahead exceeds the clock range".to_owned())?;
        let max_bytes = (u128::from(max_time_ns) * u128::from(SAMPLE_RATE * FRAME_BYTES as u64)
            / 1_000_000_000) as u64;
        let mut gain = Gain {
            muted: false,
            volume: 1.0,
            fade: 1.0,
        };
        gain.set_audio(muted, volume);
        let output = Self {
            shared: Some(Arc::new(Shared {
                parts: OnceLock::new(),
                gain: Mutex::new(gain),
                opened: AtomicBool::new(false),
            })),
            jobs,
            max_bytes,
            max_time_ns,
            frames: Cell::new(0),
            finished: Cell::new(false),
            failure: RefCell::new(None),
            _thread: PhantomData,
        };
        output.queue(move |shared| {
            let built = Parts::build(sink, max_bytes, max_time_ns);
            let gain = shared.gain.lock().unwrap_or_else(PoisonError::into_inner);
            if let Ok(parts) = &built {
                gain.apply(&parts.volume);
            }
            let _ = shared.parts.set(built);
        });
        output.pause();
        Ok(output)
    }

    fn shared(&self) -> &Arc<Shared> {
        self.shared.as_ref().expect("held until dropped")
    }

    fn queue(&self, job: impl FnOnce(&Shared) + Send + 'static) {
        let shared = Arc::clone(self.shared());
        let _ = self.jobs.send(Box::new(move || job(&shared)));
    }

    fn queue_state(&self, state: gst::State) {
        self.queue(move |shared| {
            let Some(parts) = shared.parts() else {
                return;
            };
            match parts.pipeline.set_state(state) {
                Ok(_) => shared.opened.store(true, Ordering::Release),
                Err(error) => {
                    let _ = parts.pipeline.post_message(
                        gst::message::Error::builder(
                            gst::CoreError::StateChange,
                            &error.to_string(),
                        )
                        .build(),
                    );
                }
            }
        });
    }

    pub(super) fn push(&self, data: Vec<u8>, timestamp_us: u64) -> Result<(), String> {
        if data.is_empty()
            || data.len() > MAX_CHUNK_BYTES
            || !data.len().is_multiple_of(FRAME_BYTES)
        {
            return Err("Invalid PCM chunk length".into());
        }
        let frames = self.frames.get();
        let end = frames
            .checked_add((data.len() / FRAME_BYTES) as u64)
            .ok_or("PCM sample count overflow")?;
        // Compare in the caller's microsecond precision, but retain sample-exact timing.
        if u128::from(timestamp_us) != u128::from(frames) * 1_000_000 / u128::from(SAMPLE_RATE) {
            return Err("PCM timestamps must start at zero and be contiguous".into());
        }
        if let Some(error) = self.error() {
            return Err(error);
        }
        let parts = self.writable().ok_or("PCM output is full or finished")?;
        let nanos = |samples: u64| -> Result<u64, String> {
            u64::try_from(u128::from(samples) * 1_000_000_000 / u128::from(SAMPLE_RATE))
                .ok()
                .filter(|time| *time < u64::MAX)
                .ok_or_else(|| "PCM timestamp exceeds the clock range".into())
        };
        let start_ns = nanos(frames)?;
        let end_ns = nanos(end)?;
        let duration = end_ns.checked_sub(start_ns).ok_or("Invalid PCM duration")?;
        let mut buffer = gst::Buffer::from_mut_slice(data);
        let writable = buffer.get_mut().ok_or("PCM buffer is not writable")?;
        writable.set_pts(gst::ClockTime::from_nseconds(start_ns));
        writable.set_duration(gst::ClockTime::from_nseconds(duration));
        parts
            .source
            .push_buffer(buffer)
            .map_err(|error| error.to_string())?;
        self.frames.set(end);
        Ok(())
    }

    fn writable(&self) -> Option<&Parts> {
        let shared = self.shared();
        if !shared.opened.load(Ordering::Acquire)
            || self.finished.get()
            || self.failure.borrow().is_some()
        {
            return None;
        }
        shared.parts().filter(|parts| {
            parts.source.current_level_bytes() + MAX_CHUNK_BYTES as u64 <= self.max_bytes
                && parts.source.current_level_time().nseconds() + MAX_CHUNK_NS <= self.max_time_ns
        })
    }

    pub(super) fn has_capacity(&self) -> bool {
        self.writable().is_some()
    }

    pub(super) fn play(&self) {
        self.queue_state(gst::State::Playing);
    }

    pub(super) fn pause(&self) {
        self.queue_state(gst::State::Paused);
    }

    pub(super) fn position_us(&self) -> Option<u64> {
        // Query the sink, not appsrc's queued-buffer position: this follows playback's clock.
        self.shared()
            .parts()?
            .sink
            .query_position::<gst::ClockTime>()
            .map(|time| time.useconds())
    }

    pub(super) fn set_audio(&self, muted: bool, volume: f64) {
        self.adjust(|gain| gain.set_audio(muted, volume));
    }

    pub(super) fn set_fade(&self, fade: f64) {
        self.adjust(|gain| gain.set_fade(fade));
    }

    fn adjust(&self, change: impl FnOnce(&mut Gain)) {
        let shared = self.shared();
        let mut gain = shared.gain.lock().unwrap_or_else(PoisonError::into_inner);
        change(&mut gain);
        if let Some(parts) = shared.parts() {
            gain.apply(&parts.volume);
        }
    }

    #[cfg(test)]
    pub(super) fn effective_volume(&self) -> f64 {
        self.shared()
            .parts()
            .expect("built output")
            .volume
            .property::<f64>("volume")
    }

    pub(super) fn finish(&self) -> Result<(), String> {
        if !self.finished.get() {
            if let Some(parts) = self.shared().parts() {
                parts
                    .source
                    .end_of_stream()
                    .map_err(|error| error.to_string())?;
            }
            self.finished.set(true);
        }
        Ok(())
    }

    pub(super) fn error(&self) -> Option<String> {
        match self.shared().parts.get() {
            Some(Ok(parts)) => {
                if let Some(bus) = parts.pipeline.bus() {
                    while let Some(message) = bus.pop() {
                        if let gst::MessageView::Error(error) = message.view() {
                            self.failure
                                .borrow_mut()
                                .get_or_insert_with(|| error.error().to_string());
                        }
                    }
                }
            }
            Some(Err(error)) => {
                self.failure
                    .borrow_mut()
                    .get_or_insert_with(|| error.clone());
            }
            None => {}
        }
        self.failure.borrow().clone()
    }
}

impl Drop for PcmOutput {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.take() {
            let _ = self.jobs.send(Box::new(move || {
                if let Some(parts) = shared.parts() {
                    let _ = parts.pipeline.set_state(gst::State::Null);
                }
            }));
        }
    }
}
