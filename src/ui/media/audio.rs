// SPDX-License-Identifier: MIT

use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::Rc;

use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app::AppSrc;

const SAMPLE_RATE: u64 = 48_000;
const FRAME_BYTES: usize = 4;
const MAX_CHUNK_BYTES: usize = 6_400;
const MAX_BYTES: u64 = 38_400;
const MAX_TIME_NS: u64 = 200_000_000;
const MAX_CHUNK_NS: u64 = 33_333_334;

pub(super) struct PcmOutput {
    pipeline: gst::Pipeline,
    source: AppSrc,
    sink: gst::Element,
    volume: gst::Element,
    frames: Cell<u64>,
    finished: Cell<bool>,
    failure: RefCell<Option<String>>,
    _thread: PhantomData<Rc<()>>,
}

impl PcmOutput {
    pub(super) fn new(muted: bool, volume: f64) -> Result<Self, String> {
        gst::init().map_err(|error| error.to_string())?;
        #[cfg(test)]
        let sink = gst::ElementFactory::make("fakesink")
            .property("sync", true)
            .build();
        #[cfg(not(test))]
        let sink = gst::ElementFactory::make("autoaudiosink").build();
        Self::with_sink(sink.map_err(|error| error.to_string())?, muted, volume)
    }

    fn with_sink(sink: gst::Element, muted: bool, volume: f64) -> Result<Self, String> {
        let pipeline = gst::Pipeline::new();
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
        source.set_max_bytes(MAX_BYTES);
        source.set_max_time(gst::ClockTime::from_nseconds(MAX_TIME_NS));
        source.set_property("do-timestamp", false);
        let convert = gst::ElementFactory::make("audioconvert")
            .build()
            .map_err(|error| error.to_string())?;
        let resample = gst::ElementFactory::make("audioresample")
            .build()
            .map_err(|error| error.to_string())?;
        let gain = gst::ElementFactory::make("volume")
            .build()
            .map_err(|error| error.to_string())?;
        let elements = [source.upcast_ref(), &convert, &resample, &gain, &sink];
        pipeline
            .add_many(elements)
            .map_err(|error| error.to_string())?;
        gst::Element::link_many(elements).map_err(|error| error.to_string())?;
        let output = Self {
            pipeline,
            source,
            sink,
            volume: gain,
            frames: Cell::new(0),
            finished: Cell::new(false),
            failure: RefCell::new(None),
            _thread: PhantomData,
        };
        output.set_audio(muted, volume);
        output.pause()?;
        Ok(output)
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
        if !self.has_capacity() {
            return Err("PCM output is full or finished".into());
        }
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
        self.source
            .push_buffer(buffer)
            .map_err(|error| error.to_string())?;
        self.frames.set(end);
        Ok(())
    }

    pub(super) fn has_capacity(&self) -> bool {
        !self.finished.get()
            && self.failure.borrow().is_none()
            && self.source.current_level_bytes() <= MAX_BYTES - MAX_CHUNK_BYTES as u64
            && self.source.current_level_time().nseconds() <= MAX_TIME_NS - MAX_CHUNK_NS
    }

    pub(super) fn play(&self) -> Result<(), String> {
        self.pipeline
            .set_state(gst::State::Playing)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub(super) fn pause(&self) -> Result<(), String> {
        self.pipeline
            .set_state(gst::State::Paused)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub(super) fn position_us(&self) -> Option<u64> {
        // Query the sink, not appsrc's queued-buffer position: this follows playback's clock.
        self.sink
            .query_position::<gst::ClockTime>()
            .map(|time| time.useconds())
    }

    pub(super) fn set_audio(&self, muted: bool, volume: f64) {
        let volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.volume.set_property("mute", muted);
        self.volume.set_property("volume", volume);
    }

    pub(super) fn finish(&self) -> Result<(), String> {
        if !self.finished.get() {
            self.source
                .end_of_stream()
                .map_err(|error| error.to_string())?;
            self.finished.set(true);
        }
        Ok(())
    }

    pub(super) fn error(&self) -> Option<String> {
        if let Some(bus) = self.pipeline.bus() {
            while let Some(message) = bus.pop() {
                if let gst::MessageView::Error(error) = message.view() {
                    self.failure
                        .borrow_mut()
                        .get_or_insert_with(|| error.error().to_string());
                }
            }
        }
        self.failure.borrow().clone()
    }
}

impl Drop for PcmOutput {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[cfg(test)]
mod tests;
