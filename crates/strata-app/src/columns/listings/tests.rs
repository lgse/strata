// SPDX-License-Identifier: MIT

use rune::Tx;
use strata_components::columns::ColumnSources;
use strata_core::model::Location;

use super::{Listings, SPINNER_DELAY};
use crate::{
    listing::{Column, Load},
    slots::Worn,
};

#[test]
fn column_spinner_waits_for_grace_period_unless_explicitly_busy() {
    let mut listings = Listings::translated(Worn::empty());
    listings
        .columns
        .push(Column::new(Location::local("/tmp/slow"), false, None));
    let mut tx = Tx::default();

    // Initial poll arms the 120 ms grace timer without mounting the spinner.
    assert!(!listings.watch_spinners(1.0, &mut tx));
    assert!(listings.loading(0));
    assert!(
        !listings.spinner(0),
        "spinner stays hidden during grace period"
    );

    // Still inside the grace period.
    assert!(!listings.watch_spinners(1.0 + 0.05, &mut tx));
    assert!(!listings.spinner(0));

    // Past the 120 ms grace period: spinner becomes visible.
    assert!(listings.watch_spinners(1.0 + SPINNER_DELAY.as_secs_f64() + 0.001, &mut tx));
    assert!(
        listings.spinner(0),
        "spinner shows once grace period elapses"
    );

    // Completing the load hides the spinner immediately.
    listings.columns[0].set_load(Load::Listed);
    assert!(!listings.watch_spinners(1.2, &mut tx));
    assert!(!listings.loading(0));
    assert!(!listings.spinner(0));

    // Explicit busy (sorting/refresh) shows the spinner immediately without waiting.
    listings.columns[0].set_busy(true);
    assert!(listings.watch_spinners(1.2, &mut tx));
    assert!(listings.spinner(0));
}
