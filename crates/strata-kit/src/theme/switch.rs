// SPDX-License-Identifier: MIT

//! Moving from one look to the next with the colours animating.

use rune::{Spring, Tx};

use crate::look::Look;

#[cfg(test)]
mod tests;

pub const MOTION: Spring = rune::tok::motion::CALM;

pub fn switch(look: &mut Look, next: Look, tx: &mut Tx) {
    tx.color_motion(if next.animations {
        MOTION
    } else {
        Spring::NONE
    });
    *look = next;
}
