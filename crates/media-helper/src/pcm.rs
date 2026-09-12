// SPDX-License-Identifier: MIT

use crate::audio::PcmOutput;
use std::{io, time::Instant};
use strata_media_protocol::{
    Cancellation, FRAME_TIMEOUT, TimedReader, TimedWriter,
    ipc::{self, Kind, Message, PcmSequence, u64_at},
};

pub(super) fn run(job: u64, test_sink: bool) -> Result<(), String> {
    gstreamer::init().map_err(|e| e.to_string())?;
    for name in [
        "appsrc",
        "audioconvert",
        "audioresample",
        "volume",
        if test_sink { "fakesink" } else { "pulsesink" },
    ] {
        if gstreamer::ElementFactory::find(name).is_none() {
            eprintln!("STRATA_MEDIA:audio-plugin");
            return Err("Missing audio plugin".into());
        }
    }
    let output = if test_sink {
        PcmOutput::with_sink(
            gstreamer::ElementFactory::make("fakesink")
                .property("sync", true)
                .build()
                .map_err(|e| e.to_string())?,
            true,
            0.0,
        )
    } else {
        PcmOutput::new(true, 0.0)
    }
    .map_err(|_| {
        eprintln!("STRATA_MEDIA:audio-server");
        "Audio output unavailable".to_owned()
    })?;
    let cancellation = Cancellation::default();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = TimedReader {
        fd: &stdin,
        deadline: Instant::now() + FRAME_TIMEOUT,
        cancellation: &cancellation,
    };
    let mut writer = TimedWriter::new(&stdout, Instant::now() + FRAME_TIMEOUT, &cancellation)
        .map_err(|e| e.to_string())?;
    ipc::hello(
        &mut writer,
        ipc::PCM,
        job,
        env!("STRATA_RELEASE_TAG"),
        env!("STRATA_BUILD_COMMIT"),
    )
    .map_err(|e| e.to_string())?;
    let mut sequence = 0_u64;
    let mut samples = PcmSequence::default();
    loop {
        reader.deadline = Instant::now() + FRAME_TIMEOUT;
        let message = Message::read(&mut reader, job, sequence).map_err(|e| e.to_string())?;
        match message.kind {
            Kind::Poll => {}
            Kind::Settings => {
                let muted = u64_at(&message.data, 0);
                let volume = f64::from_bits(u64_at(&message.data, 8));
                if muted > 1 || !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
                    return Err("Invalid audio preferences".into());
                }
                output.set_audio(muted == 1, volume);
            }
            Kind::Play => output.play().map_err(audio_failure)?,
            Kind::Pause => output.pause().map_err(audio_failure)?,
            Kind::Samples => {
                samples
                    .push(message.data.len(), message.time_us)
                    .map_err(|e| e.to_string())?;
                output
                    .push(message.data, message.time_us)
                    .map_err(audio_failure)?;
            }
            Kind::Finish => {
                samples.finish().map_err(|e| e.to_string())?;
                output.finish().map_err(audio_failure)?;
            }
            Kind::Status => return Err("Unexpected PCM reply".into()),
        }
        let failed = output.error().is_some();
        let position = output.position_us().unwrap_or(u64::MAX);
        if position != u64::MAX && position > 30_000_000 {
            return Err("Audio clock exceeded interval".into());
        }
        let mut data = Vec::with_capacity(24);
        for value in [
            u64::from(output.has_capacity()),
            position,
            u64::from(failed),
        ] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        writer.deadline = Instant::now() + FRAME_TIMEOUT;
        Message {
            kind: Kind::Status,
            time_us: 0,
            data,
        }
        .write(&mut writer, job, sequence)
        .map_err(|e| e.to_string())?;
        if failed {
            eprintln!("STRATA_MEDIA:audio-server");
            return Err("Audio output failed".into());
        }
        sequence = sequence.checked_add(1).ok_or("PCM sequence exhausted")?;
    }
}

fn audio_failure(error: String) -> String {
    eprintln!("STRATA_MEDIA:audio-server");
    error
}
