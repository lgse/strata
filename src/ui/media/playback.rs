// SPDX-License-Identifier: MIT

use std::{
    ops::ControlFlow,
    time::{Duration, Instant},
};

use gtk::{gdk, glib, prelude::*, subclass::prelude::*};

use crate::{
    media::{self, Frame, Header, Packet},
    sandbox::media::{Event, Session},
};

use super::{
    AUDIO_LEAD_CAP_US, AUDIO_STUCK_TIMEOUT, DecodedMedia, EDGE_SAMPLE_INTERVAL, PAUSED_IDLE,
    PRESENTATION_QUEUE, RESIZE_DELAY, RESIZE_GROWTH, SEEK_DELAY, ambient, audio::PcmOutput,
};

const MAX_STALL_RECOVERIES: u32 = 3;
// PCM appsrc may hold: the lead the first record carries plus slack, so the
// sink's own buffer fills before the playhead has to move.
const AUDIO_LOOKAHEAD: Duration =
    Duration::from_micros((media::AUDIO_LEAD_TICKS as u64 + 6) * 1_000_000 / media::FPS as u64);

/// What the event pump does after handling one session event.
enum Pump {
    /// Keep draining events this tick.
    Continue,
    /// Stop draining; the rest of the tick still runs.
    Drained,
    /// The tick is done, with its remaining stages skipped.
    Yield,
}

impl DecodedMedia {
    pub(super) fn tick(&self) -> Result<(), String> {
        let imp = self.imp();
        self.check_startup_timeout()?;
        self.settle_resize();
        self.settle_seek();
        self.release_idle_worker();
        if imp.dormant.get() {
            return Ok(());
        }
        if let Some(tick) = imp.restart.get()
            && self.start_session(tick)?.is_break()
        {
            return Ok(());
        }
        if self.pump_events()?.is_break() {
            return Ok(());
        }
        self.trim_history();
        if self.check_audio_error()?.is_break() {
            return Ok(());
        }
        if self.is_playing() && imp.first_frame.get() {
            self.advance_playback()?;
        }
        Ok(())
    }

    fn check_startup_timeout(&self) -> Result<(), String> {
        if self
            .imp()
            .starting
            .get()
            .is_some_and(|time| time.elapsed() > media::STARTUP_TIMEOUT)
        {
            return Err("Media startup or seek timed out".into());
        }
        Ok(())
    }

    fn settle_resize(&self) {
        let imp = self.imp();
        if !imp
            .resized
            .get()
            .is_some_and(|time| time.elapsed() >= RESIZE_DELAY)
        {
            return;
        }
        imp.resized.set(None);
        if !imp.dormant.get() && self.needs_resize() {
            self.capture_position();
            self.restart_at(imp.position.get());
        }
    }

    // A settled seek must wake a dormant, paused worker before idle handling.
    fn settle_seek(&self) {
        let imp = self.imp();
        if let Some((position, since)) = imp.seek_pending.get()
            && since.elapsed() >= SEEK_DELAY
        {
            imp.seek_pending.take();
            self.restart_at(position);
            if self.is_prepared() {
                self.update(imp.position.get() as i64);
            }
        }
    }

    fn release_idle_worker(&self) {
        let imp = self.imp();
        if !self.paused_idle() {
            return;
        }
        imp.session.borrow_mut().take();
        imp.audio.borrow_mut().take();
        imp.frames.borrow_mut().clear();
        imp.restart.set(None);
        imp.dormant.set(true);
        tracing::debug!("sandboxed media paused worker released");
    }

    fn paused_idle(&self) -> bool {
        let imp = self.imp();
        let paused_long_enough = imp
            .paused
            .get()
            .is_some_and(|time| time.elapsed() >= PAUSED_IDLE);
        !self.is_playing()
            && paused_long_enough
            && !imp.dormant.get()
            && imp.seek_pending.get().is_none()
    }

    /// Spawns the pending decoder once the previous session is gone and the
    /// start dwell has passed; `Break` leaves the tick waiting.
    fn start_session(&self, tick: u32) -> Result<ControlFlow<()>, String> {
        let imp = self.imp();
        if imp
            .session
            .borrow()
            .as_ref()
            .is_some_and(|session| !session.finished())
        {
            return Ok(ControlFlow::Break(()));
        }
        if imp
            .start_after
            .get()
            .is_some_and(|after| Instant::now() < after)
        {
            return Ok(ControlFlow::Break(()));
        }
        imp.start_after.set(None);
        imp.session.borrow_mut().take();
        let source = imp
            .source
            .borrow()
            .as_ref()
            .cloned()
            .ok_or("Preview closed")?;
        let size = source.size;
        #[cfg(test)]
        let started = match imp.loader.borrow().as_ref() {
            Some(loader) => loader(source, tick),
            None => Session::start(source, tick),
        };
        #[cfg(not(test))]
        let started = Session::start(source, tick);
        match started {
            Ok(session) => {
                imp.session.replace(Some(session));
                imp.loaded_size.set(Some(size));
                imp.restart.set(None);
                Ok(ControlFlow::Continue(()))
            }
            Err(error)
                if imp
                    .starting
                    .get()
                    .is_some_and(|time| time.elapsed() < Duration::from_millis(250)) =>
            {
                let _ = error;
                Ok(ControlFlow::Break(()))
            }
            Err(error) => Err(error),
        }
    }

    fn pump_events(&self) -> Result<ControlFlow<()>, String> {
        let imp = self.imp();
        while imp.end.get().is_none() && self.can_queue_more() {
            let event = imp.session.borrow().as_ref().and_then(Session::receive);
            let next = match event {
                Some(Event::Prepared(header)) => self.on_prepared(header)?,
                Some(Event::Packet(Packet::Frame(frame))) => {
                    self.on_frame(frame)?;
                    Pump::Continue
                }
                Some(Event::Packet(Packet::End(duration))) => {
                    self.on_end(duration)?;
                    Pump::Drained
                }
                Some(Event::Failed(error)) => {
                    self.on_failed(error)?;
                    Pump::Yield
                }
                None => Pump::Drained,
            };
            match next {
                Pump::Continue => {}
                Pump::Drained => break,
                Pump::Yield => return Ok(ControlFlow::Break(())),
            }
        }
        Ok(ControlFlow::Continue(()))
    }

    fn on_prepared(&self, header: Header) -> Result<Pump, String> {
        let imp = self.imp();
        let start = self.start_position(&header);
        if let Some(saved) = self.resumable_position(&header) {
            imp.header.set(Some(header));
            self.invalidate_size();
            tracing::debug!(position_us = saved, "resuming media preview");
            self.restart_at(saved);
            return Ok(Pump::Drained);
        }
        if let Some(position) = start {
            imp.header.set(Some(header));
            self.invalidate_size();
            tracing::debug!(position_us = position, "starting video preview");
            self.restart_at(position);
            return Ok(Pump::Drained);
        }
        let audio = header.audio.then(|| self.pcm_output()).transpose()?;
        imp.audio.replace(audio);
        imp.header.set(Some(header));
        self.invalidate_size();
        if imp.source.borrow().as_ref().map(|source| source.size) != imp.loaded_size.get() {
            imp.resized.set(Some(Instant::now()));
        }
        if !self.is_prepared() {
            let known_duration = header.duration_us != media::MAX_DURATION_US;
            self.stream_prepared(
                header.audio,
                header.width > 0,
                known_duration,
                if known_duration {
                    header.duration_us as i64
                } else {
                    0
                },
            );
        }
        Ok(Pump::Continue)
    }

    /// Consumes the saved position; only video that still contains it resumes there.
    fn resumable_position(&self, header: &Header) -> Option<u64> {
        self.imp()
            .restore
            .take()
            .filter(|&saved| header.width > 0 && saved < header.duration_us)
    }

    /// Consumes the configured start; only video of known duration starts there.
    fn start_position(&self, header: &Header) -> Option<u64> {
        let percent = self.imp().start_percent.take();
        (header.width > 0 && header.duration_us != media::MAX_DURATION_US)
            .then(|| header.duration_us / 100 * u64::from(percent))
            .filter(|&position| media::seek_tick(position, header.duration_us) > 0)
    }

    fn on_frame(&self, mut frame: Frame) -> Result<(), String> {
        let imp = self.imp();
        let header = imp.header.get().ok_or("Missing media header")?;
        if let Some(audio) = imp.audio.borrow().as_ref() {
            self.push_audio(audio, &header, std::mem::take(&mut frame.samples))?;
        }
        if imp.first_frame.get() {
            imp.frames.borrow_mut().push_back(frame);
        } else {
            self.present_first_frame(frame, &header);
        }
        Ok(())
    }

    // Blocks belong to successive audio ticks, running ahead of the frame.
    fn push_audio(
        &self,
        audio: &PcmOutput,
        header: &Header,
        samples: Vec<u8>,
    ) -> Result<(), String> {
        let imp = self.imp();
        for block in samples.chunks(media::AUDIO_BYTES) {
            let chunk = imp.audio_chunks.get();
            let tick = header.start_tick.saturating_add(chunk);
            let samples_left = header.duration_us.saturating_sub(media::timestamp(tick))
                * media::SAMPLE_RATE
                / 1_000_000;
            let block = &block[..samples_left.min(block.len() as u64 / 4) as usize * 4];
            if !block.is_empty() {
                if let Some(history) = imp.history.borrow_mut().as_mut() {
                    history.push(block);
                }
                audio.push(block.to_vec(), media::timestamp(chunk))?;
            }
            imp.audio_chunks.set(chunk + 1);
        }
        Ok(())
    }

    fn present_first_frame(&self, frame: Frame, header: &Header) {
        let imp = self.imp();
        let latency_ms = imp
            .starting
            .get()
            .map_or(0, |time| time.elapsed().as_millis()) as u64;
        imp.first_frame.set(true);
        imp.seek_origin.set(None);
        imp.starting.set(None);
        imp.last_progress.set(Some(Instant::now()));
        if self.is_playing() {
            imp.clock.set(self.wall_clock_allowed().then(Instant::now));
            if let Some(audio) = imp.audio.borrow().as_ref() {
                audio.play();
            }
        }
        self.present(frame);
        tracing::debug!(
            latency_ms,
            position_us = media::timestamp(header.start_tick),
            width = header.width,
            height = header.height,
            "sandboxed media first frame"
        );
        if self.is_seeking() {
            self.seek_success();
        }
        self.update(imp.position.get() as i64);
    }

    fn on_end(&self, duration: u64) -> Result<(), String> {
        let imp = self.imp();
        imp.end.set(Some(duration));
        if let Some(audio) = imp.audio.borrow().as_ref() {
            audio.finish()?;
        }
        if self.duration() != duration as i64 {
            self.adopt_duration(duration)?;
        }
        Ok(())
    }

    fn adopt_duration(&self, duration: u64) -> Result<(), String> {
        let imp = self.imp();
        let playing = self.is_playing();
        self.capture_position();
        let header = imp.header.get().ok_or("Missing media header")?;
        let _notifications = self.freeze_notify();
        self.stream_unprepared();
        self.stream_prepared(header.audio, header.width > 0, true, duration as i64);
        self.update(imp.position.get().min(duration) as i64);
        if playing {
            self.play();
        }
        Ok(())
    }

    fn on_failed(&self, error: String) -> Result<(), String> {
        let imp = self.imp();
        if imp.first_frame.get() {
            return self.recover(&error);
        }
        // Truncated downloads can advertise a duration beyond their data.
        let Some(origin) = imp.seek_origin.take() else {
            return Err(error);
        };
        tracing::warn!(error, origin_us = origin, "seek failed; resuming");
        if self.is_seeking() {
            self.seek_failed();
        }
        self.restart_at(origin);
        if self.is_prepared() {
            self.update(imp.position.get() as i64);
        }
        Ok(())
    }

    // Trim behind the sink itself, not the extrapolated playhead: a stalled
    // sink resumes from its last position and still needs those samples.
    fn trim_history(&self) {
        let imp = self.imp();
        if let Some(history) = imp.history.borrow_mut().as_mut() {
            history.trim_behind(imp.clock_base.get() * media::SAMPLE_RATE / 1_000_000);
        }
    }

    fn check_audio_error(&self) -> Result<ControlFlow<()>, String> {
        let imp = self.imp();
        let audio_error = imp.audio.borrow().as_ref().and_then(PcmOutput::error);
        let Some(error) = audio_error else {
            return Ok(ControlFlow::Continue(()));
        };
        if imp.first_frame.get() {
            self.recover(&error)?;
            return Ok(ControlFlow::Break(()));
        }
        Err(error)
    }

    fn advance_playback(&self) -> Result<(), String> {
        let imp = self.imp();
        self.capture_position();
        if imp
            .audio_stuck_since
            .get()
            .is_some_and(|since| since.elapsed() > AUDIO_STUCK_TIMEOUT)
        {
            return self.recover("Audio output stopped advancing");
        }
        let ended = self.reached_end();
        self.present_due_frames();
        self.update(imp.position.get() as i64);
        if ended {
            self.finish_playback();
        } else if imp
            .last_progress
            .get()
            .is_some_and(|time| time.elapsed() > media::FRAME_TIMEOUT)
        {
            self.recover("Media playback clock stopped making progress")?;
        }
        Ok(())
    }

    /// Snaps the playhead to the end once it comes within a sample of it.
    fn reached_end(&self) -> bool {
        let imp = self.imp();
        imp.end.get().is_some_and(|end| {
            // The advertised duration can fall between 48-kHz sample boundaries.
            let tolerance = if imp.header.get().is_some_and(|header| header.audio) {
                1_000_000_u64.div_ceil(media::SAMPLE_RATE) + 1
            } else {
                0
            };
            if imp.position.get().saturating_add(tolerance) >= end {
                imp.position.set(end);
                true
            } else {
                false
            }
        })
    }

    fn present_due_frames(&self) {
        let imp = self.imp();
        loop {
            let frame = {
                let mut frames = imp.frames.borrow_mut();
                if frames
                    .front()
                    .is_some_and(|frame| media::timestamp(frame.tick) <= imp.position.get())
                {
                    frames.pop_front()
                } else {
                    None
                }
            };
            let Some(frame) = frame else {
                break;
            };
            self.present(frame);
        }
    }

    fn finish_playback(&self) {
        let imp = self.imp();
        if self.is_loop() {
            self.restart_at(0);
        } else {
            imp.session.borrow_mut().take();
            imp.audio.borrow_mut().take();
            self.stream_ended();
        }
    }

    fn recover(&self, reason: &str) -> Result<(), String> {
        let imp = self.imp();
        let recoveries = imp.recoveries.get();
        if recoveries >= MAX_STALL_RECOVERIES {
            return Err(reason.into());
        }
        imp.recoveries.set(recoveries + 1);
        tracing::warn!(
            recoveries = recoveries + 1,
            reason,
            "restarting stalled media playback"
        );
        self.capture_position_progress(false);
        let position = imp.position.get();
        self.restart_at(position);
        imp.last_progress.set(Some(Instant::now()));
        Ok(())
    }

    pub(super) fn capture_position(&self) {
        self.capture_position_progress(true);
    }

    // A failure snapshot must not clear the strike even if the playhead moved.
    fn capture_position_progress(&self, clears_recoveries: bool) {
        let imp = self.imp();
        if !imp.first_frame.get() {
            return;
        }
        let base = imp.clock_base.get();
        let elapsed = imp
            .clock
            .get()
            .map_or(0, |clock| clock.elapsed().as_micros() as u64);
        let (relative, audio_flowing) = self.sink_position(base, base + elapsed);
        let Some(header) = imp.header.get() else {
            return;
        };
        let position = (media::timestamp(header.start_tick) + relative)
            .min(imp.end.get().unwrap_or(header.duration_us));
        if position != imp.position.get() {
            imp.last_progress.set(Some(Instant::now()));
            // Wall-clock drift over stuck audio must not reset the strike cap.
            if clears_recoveries && audio_flowing {
                imp.recoveries.set(0);
            }
        }
        imp.position.set(position);
    }

    /// The playhead relative to the generation start, anchored to the audio sink
    /// when it has advanced, and whether audio is flowing.
    fn sink_position(&self, base: u64, wall: u64) -> (u64, bool) {
        let imp = self.imp();
        let audio = imp.audio.borrow();
        let Some(audio) = audio.as_ref() else {
            return (wall, true);
        };
        match audio.position_us() {
            Some(position) if position > base => {
                imp.clock_base.set(position);
                imp.clock.set(self.is_playing().then(Instant::now));
                imp.audio_stuck_since.set(None);
                imp.audio_anchored.set(true);
                (position, true)
            }
            // Keep the audio anchor so a recovered sink can take over again.
            // The stuck timer only watches a clock that has run; a sink that is
            // still starting is bounded by the progress watchdog instead.
            _ => {
                if self.audio_clock_stalled() {
                    imp.audio_stuck_since.set(Some(Instant::now()));
                }
                (wall.min(base + AUDIO_LEAD_CAP_US), false)
            }
        }
    }

    fn audio_clock_stalled(&self) -> bool {
        let imp = self.imp();
        self.is_playing() && imp.audio_anchored.get() && imp.audio_stuck_since.get().is_none()
    }

    // Until the sink has advanced since the generation started or playback
    // resumed, the playhead waits for it rather than leading on wall time, which
    // would later snap back by the sink's delay. Afterwards a stalled clock
    // still lets video lead.
    pub(super) fn wall_clock_allowed(&self) -> bool {
        let imp = self.imp();
        imp.audio.borrow().is_none() || imp.audio_anchored.get()
    }

    // Audio files read ahead until appsrc is full, so a sink that only starts
    // once its own buffer fills is fed regardless of the playhead. Video keeps
    // the three-frame presentation queue; its audio runs ahead inside the
    // records instead of pinning frames.
    fn can_queue_more(&self) -> bool {
        let imp = self.imp();
        let frames = imp.frames.borrow().len();
        let Some(header) = imp.header.get() else {
            return true;
        };
        match imp.audio.borrow().as_ref() {
            Some(audio) => {
                audio.has_capacity() && (header.width == 0 || frames < PRESENTATION_QUEUE)
            }
            None => frames < PRESENTATION_QUEUE,
        }
    }

    fn pcm_output(&self) -> Result<PcmOutput, String> {
        #[cfg(test)]
        let output = if let Some(description) = self.imp().audio_sink.borrow().as_deref() {
            PcmOutput::test_sink(
                self.is_muted(),
                self.volume(),
                AUDIO_LOOKAHEAD,
                description,
                self.imp().audio_clock.borrow().as_ref(),
            )?
        } else {
            PcmOutput::new(self.is_muted(), self.volume(), AUDIO_LOOKAHEAD)?
        };
        #[cfg(not(test))]
        let output = PcmOutput::new(self.is_muted(), self.volume(), AUDIO_LOOKAHEAD)?;
        if let Some(fade) = self.imp().fade.get() {
            output.set_fade(fade);
        }
        Ok(output)
    }

    fn needs_resize(&self) -> bool {
        let imp = self.imp();
        let (Some(header), Some(loaded)) = (imp.header.get(), imp.loaded_size.get()) else {
            return false;
        };
        if header.width == 0 {
            return false;
        }
        let Some(size) = imp.source.borrow().as_ref().map(|source| source.size) else {
            return false;
        };
        let mut scale = (f64::from(size.width) / f64::from(header.width))
            .min(f64::from(size.height) / f64::from(header.height));
        if header.width + 1 < loaded.width as u32 && header.height + 1 < loaded.height as u32 {
            scale = scale.min(1.0);
        }
        // Downsampling avoids a decoder restart and audible drop on shrink.
        scale >= 1.0 + RESIZE_GROWTH
            && (f64::from(header.width) * (scale - 1.0) >= 2.0
                || f64::from(header.height) * (scale - 1.0) >= 2.0)
    }

    pub(super) fn present(&self, frame: Frame) {
        let imp = self.imp();
        let Some(header) = imp.header.get().filter(|header| header.width > 0) else {
            return;
        };
        self.sample_edges(&frame, &header);
        #[cfg(debug_assertions)]
        let pixels = super::diagnostics::Pixels::new(frame.pixels);
        #[cfg(not(debug_assertions))]
        let pixels = frame.pixels;
        let texture = gdk::MemoryTexture::new(
            header.width as i32,
            header.height as i32,
            gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(pixels),
            header.width as usize * 4,
        )
        .upcast::<gdk::Texture>();
        let size_changed =
            imp.texture.borrow().as_ref().is_none_or(|old| {
                old.width() != texture.width() || old.height() != texture.height()
            });
        imp.texture.replace(Some(texture));
        if size_changed {
            self.invalidate_size();
        }
        self.invalidate_contents();
    }

    fn sample_edges(&self, frame: &Frame, header: &Header) {
        let imp = self.imp();
        if imp
            .edge_sampled
            .get()
            .is_none_or(|sampled| sampled.elapsed() >= EDGE_SAMPLE_INTERVAL)
        {
            imp.edge_sampled.set(Some(Instant::now()));
            imp.edge_grid
                .set(ambient::sample(&frame.pixels, header.width, header.height));
        }
    }
}
