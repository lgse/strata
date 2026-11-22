// SPDX-License-Identifier: MIT

use super::*;

fn pcm(values: impl IntoIterator<Item = i16>) -> Vec<u8> {
    values
        .into_iter()
        .flat_map(|value| {
            let bytes = value.to_le_bytes();
            [bytes[0], bytes[1], bytes[0], bytes[1]]
        })
        .collect()
}

#[test]
fn played_audio_window_ends_at_the_playhead_not_the_decoder() {
    let mut history = PcmHistory::default();
    history.push(&pcm((0..8).map(|value| value * 4096)));
    let mut window = [1.0; 4];

    assert!(history.window_ending_at(6, &mut window));
    assert_eq!(window, [0.25, 0.375, 0.5, 0.625]);

    assert!(history.window_ending_at(2, &mut window));
    assert_eq!(window, [0.0, 0.0, 0.0, 0.125]);

    assert!(history.window_ending_at(100, &mut window));
    assert_eq!(
        window[3], 0.875,
        "a lagging sink never reads past decoded audio"
    );
}

#[test]
fn played_audio_history_keeps_unplayed_samples_and_drops_old_ones() {
    let mut history = PcmHistory::default();
    history.push(&pcm(std::iter::repeat_n(1, 5 * HISTORY_SAMPLES)));
    let mut window = [0.0; 4];
    let window_samples = HISTORY_SAMPLES as u64;

    history.trim_behind(window_samples / 2);
    assert!(
        history.window_ending_at(window_samples / 2, &mut window),
        "a sink far behind the decoder still finds its samples"
    );
    assert!(history.window_ending_at(5, &mut window));

    history.trim_behind(3 * window_samples);
    assert!(!history.window_ending_at(5, &mut window));
    assert!(!history.window_ending_at(2 * window_samples - 1, &mut window));
    assert!(history.window_ending_at(2 * window_samples + 4, &mut window));
    assert!(history.window_ending_at(5 * window_samples, &mut window));

    history.clear();
    assert!(!history.window_ending_at(1, &mut window));
}
