// SPDX-License-Identifier: MIT

//! Internal audio backend abstraction and Linux probe order (PulseAudio -> ALSA -> Silent).

use super::{BackendKind, PcmSpec, alsa::AlsaBackend, pulse::PulseBackend, silent::SilentBackend};

/// Low-level blocking PCM device owned by the dedicated audio worker thread.
pub(super) trait AudioBackend: Send {
    fn kind(&self) -> BackendKind;
    fn write(&mut self, bytes: &[u8]) -> Result<(), ()>;
    fn latency_us(&mut self) -> Option<u64>;
    fn flush(&mut self) -> Result<(), ()>;
}

pub(super) fn open_backend(
    spec: PcmSpec,
    app_name: &str,
    stream_name: &str,
    silent_only: bool,
) -> Box<dyn AudioBackend> {
    if silent_only {
        return Box::new(SilentBackend::new(spec));
    }
    if let Ok(pulse) = PulseBackend::open(spec, app_name, stream_name) {
        return Box::new(pulse);
    }
    if let Ok(alsa) = AlsaBackend::open(spec) {
        return Box::new(alsa);
    }
    Box::new(SilentBackend::new(spec))
}
