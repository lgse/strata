// SPDX-License-Identifier: MIT

//! The generator's random numbers. SplitMix64 in wrapping integer arithmetic only, so a seed
//! gives the same sequence on every machine and with every toolchain; nothing here may come
//! to depend on a crate whose stream can change between versions.

pub(crate) struct Random(u64);

impl Random {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    }

    /// A number below `bound`, each equally likely: a draw from the uneven low end of the
    /// range is thrown away, not folded in.
    pub(crate) fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "a draw needs something to draw from");
        let uneven = bound.wrapping_neg() % bound;
        loop {
            let drawn = self.next();
            if drawn >= uneven {
                return drawn % bound;
            }
        }
    }
}
