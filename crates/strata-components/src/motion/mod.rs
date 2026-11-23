// SPDX-License-Identifier: MIT

mod dissolve;
mod flight;
mod layer;
mod model;
mod pulse;

#[cfg(test)]
mod tests;

pub use dissolve::{DURATION, Dissolve, MAX_FRAGMENTS};
pub use flight::{
    Flight, MAX_FLYERS, MAX_STAGGER, Mode, RELEASE_TRAVEL, RESTORE_TRAVEL, STAGGER, TRAVEL,
    flyer_key, launch_key,
};
pub use layer::{LAYER, MotionLayer};
pub use model::{EffectId, Effects, Event};
pub use pulse::{BOUNCE, PulseKind, RECEIVING, SHAKE, dress_target, ring_key};
