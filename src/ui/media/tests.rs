// SPDX-License-Identifier: MIT

use super::*;
use std::{path::PathBuf, rc::Rc, time::Duration};

const TEST_DURATION_US: u64 = 60_000_000;

fn test_header(start_tick: u32) -> Header {
    Header {
        width: 16,
        height: 16,
        audio: false,
        duration_us: TEST_DURATION_US,
        start_tick,
    }
}

fn test_source(path: &str) -> SandboxedMedia {
    SandboxedMedia {
        path: PathBuf::from(path),
        size: MediaPreviewSize::new(320, 240),
        backend: crate::sandbox::MediaPreviewBackend::Software,
        input_owner: None,
    }
}

fn fake_loader(calls: &Rc<RefCell<Vec<u32>>>) -> TestLoader {
    let calls = calls.clone();
    Rc::new(move |_source: SandboxedMedia, tick: u32| {
        calls.borrow_mut().push(tick);
        crate::sandbox::media::tests::stream(test_header(tick))
    })
}

fn drive_until(media: &DecodedMedia, calls: &Rc<RefCell<Vec<u32>>>, want: usize) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while calls.borrow().len() < want {
        match media.tick() {
            Ok(()) => {}
            // Worker slots are shared with other tests.
            Err(error) if error.contains("busy") => {}
            Err(error) => panic!("tick failed: {error}"),
        }
        assert!(Instant::now() < deadline, "decode worker deadline");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn open_restoring(path: &str, position: u64) -> (DecodedMedia, Rc<RefCell<Vec<u32>>>) {
    remember_media_position(path.into(), position);
    let media = DecodedMedia::new(test_source(path));
    let calls = Rc::new(RefCell::new(Vec::new()));
    media.imp().loader.replace(Some(fake_loader(&calls)));
    media.upcast_ref::<gtk::MediaStream>().play();
    (media, calls)
}

#[test]
fn reopening_resumes_where_the_preview_closed() {
    gtk::init().expect("GTK display");

    let (media, calls) = open_restoring("/remembered", media::timestamp(900));
    let deadline = Instant::now() + Duration::from_secs(15);
    while (media.timestamp() as u64) < media::timestamp(900) {
        media.tick().expect("tick succeeds");
        assert!(Instant::now() < deadline, "resume deadline");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(*calls.borrow(), vec![0, 900]);
    let position = media.timestamp() as u64;
    media.close();
    assert_eq!(
        recall_media_position(Path::new("/remembered")),
        Some(position)
    );

    let (media, calls) = open_restoring("/shortened", TEST_DURATION_US + 1);
    drive_until(&media, &calls, 1);
    for _ in 0..20 {
        media.tick().expect("tick succeeds");
    }
    assert_eq!(*calls.borrow(), vec![0]);
    assert!(media.imp().first_frame.get());
    media.close();
    assert_eq!(recall_media_position(Path::new("/shortened")), None);

    let (media, calls) = open_restoring("/ended", media::timestamp(900));
    drive_until(&media, &calls, 2);
    for _ in 0..20 {
        media.tick().expect("tick succeeds");
    }
    media.imp().position.set(TEST_DURATION_US);
    media.close();
    assert_eq!(recall_media_position(Path::new("/ended")), None);

    let (media, calls) = open_restoring("/ended", media::timestamp(900));
    drive_until(&media, &calls, 2);
    media.imp().position.set(RESTORE_MIN_US);
    media.close();
    assert_eq!(recall_media_position(Path::new("/ended")), None);
}

#[test]
fn silent_samples_produce_minimal_bands() {
    let silent = vec![0u8; 4096];
    let bands = compute_spectrum_bands(&silent);
    assert_eq!(bands.len(), 24);
    for &b in &bands {
        assert!(b < 0.05, "Silence should produce minimal energy, got {b}");
    }
}

#[test]
fn low_frequency_tone_excites_bass_bands() {
    // Generate a 100 Hz sine wave at 48 kHz
    let mut samples = Vec::with_capacity(4096);
    for k in 0..1024 {
        let t = k as f32 / 48000.0;
        let val = (2.0 * std::f32::consts::PI * 100.0 * t).sin();
        let s = (val * 30000.0) as i16;
        samples.extend_from_slice(&s.to_le_bytes()); // Left
        samples.extend_from_slice(&s.to_le_bytes()); // Right
    }
    let bands = compute_spectrum_bands(&samples);
    // 100 Hz falls in the lower log bands (around bands 3..7)
    let bass_energy: f32 = bands[2..8].iter().copied().fold(0.0, f32::max);
    let treble_energy: f32 = bands[18..24].iter().copied().fold(0.0, f32::max);
    assert!(
        bass_energy > 0.4,
        "100 Hz tone should excite lower bands, got {bass_energy}"
    );
    assert!(
        bass_energy > treble_energy,
        "Bass energy ({bass_energy}) should exceed treble energy ({treble_energy})"
    );
}

#[test]
fn high_frequency_tone_excites_treble_bands() {
    // Generate a 6000 Hz sine wave at 48 kHz
    let mut samples = Vec::with_capacity(4096);
    for k in 0..1024 {
        let t = k as f32 / 48000.0;
        let val = (2.0 * std::f32::consts::PI * 6000.0 * t).sin();
        let s = (val * 30000.0) as i16;
        samples.extend_from_slice(&s.to_le_bytes()); // Left
        samples.extend_from_slice(&s.to_le_bytes()); // Right
    }
    let bands = compute_spectrum_bands(&samples);
    // 6000 Hz falls in the upper log bands (around bands 18..22)
    let treble_energy: f32 = bands[18..23].iter().copied().fold(0.0, f32::max);
    let sub_bass_energy: f32 = bands[0..4].iter().copied().fold(0.0, f32::max);
    assert!(
        treble_energy > 0.4,
        "6000 Hz tone should excite treble bands, got {treble_energy}"
    );
    assert!(
        treble_energy > sub_bass_energy,
        "Treble energy ({treble_energy}) should exceed sub-bass energy ({sub_bass_energy})"
    );
}
