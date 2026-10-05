// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn decoded_peaks_become_displayed_levels_and_clearing_lowers_them() {
    crate::test_support::gtk_test(
        "ui::preview::waveform::tests::decoded_peaks_become_displayed_levels_and_clearing_lowers_them",
        || {
            let waveform = Waveform::new();
            let mut levels = vec![0u8; BUCKETS as usize];
            levels[0] = 255;
            levels[1] = 64;
            waveform.add_levels(0, &levels);
            let shown = waveform.shown_levels();
            assert!(
                shown[0] > shown[1] && shown[1] > 0.0,
                "a quieter bucket reads lower, not flat"
            );
            assert_eq!(shown[2], 0.0, "silence stays flat");

            waveform.clear_levels();
            assert!(
                waveform.shown_levels().iter().all(|level| *level == 0.0),
                "clearing lowers the waveform to silence"
            );
        },
    );
}
