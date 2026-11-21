// SPDX-License-Identifier: MIT

//! The shrinker's bounds, on `compare_tests`' sequences and oracle.

use super::{
    compare_tests::{double, steps},
    shrink::{Budget, shrink},
};

/// An alphabet's opening stays whole, whether or not the divergence needs it.
pub(crate) fn the_shrinker_keeps_an_alphabet_s_opening_whole() {
    let long = steps("x y a x x y b y x x a y c x x y z a b c");
    let found = double(&long).expect("the long sequence diverges");
    let budget = Budget { asks: 200, kept: 2 };
    let opened = shrink(&long, &found, budget, &mut |candidate| {
        Ok(double(candidate))
    })
    .expect("the oracle never fails");
    assert_eq!(opened.steps, steps("x y a b c"));
    assert_eq!(opened.divergence.step, 5);
    assert!(opened.is_confirmed());
}
