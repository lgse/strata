// SPDX-License-Identifier: MIT
use super::*;
use std::io::Cursor;

#[test]
fn identities_reject_wrong_role_job_version_and_truncation() {
    let mut bytes = Vec::new();
    hello(&mut bytes, PARSER, 7, "v1.2.3-rc.4", "abc123").expect("identity");
    for (role, job, release, commit) in [
        (PCM, 7, "v1.2.3-rc.4", "abc123"),
        (PARSER, 8, "v1.2.3-rc.4", "abc123"),
        (PARSER, 7, "v1.2.3", "abc123"),
        (PARSER, 7, "v1.2.3-rc.4", "def456"),
    ] {
        assert!(check_hello(&mut Cursor::new(&bytes), role, job, release, commit).is_err());
    }
    for n in 0..HELLO_BYTES {
        assert!(
            check_hello(
                &mut Cursor::new(&bytes[..n]),
                PARSER,
                7,
                "v1.2.3-rc.4",
                "abc123"
            )
            .is_err()
        );
    }
    check_hello(&mut Cursor::new(bytes), PARSER, 7, "v1.2.3-rc.4", "abc123")
        .expect("matching identity");
}

#[test]
fn audio_framing_is_bounded_and_rejects_stale_or_malformed_messages() {
    let message = Message {
        kind: Kind::Samples,
        time_us: 0,
        data: vec![0; AUDIO_BYTES],
    };
    let mut bytes = Vec::new();
    message.write(&mut bytes, 42, 2).expect("packet");
    for (job, sequence) in [(41, 2), (42, 1), (42, 3)] {
        assert!(Message::read(&mut Cursor::new(&bytes), job, sequence).is_err());
    }
    for offset in [0, 24, 28, 32] {
        let mut bad = bytes.clone();
        bad[offset..offset + 4].fill(0xff);
        assert!(Message::read(&mut Cursor::new(bad), 42, 2).is_err());
    }
    for length in [0, 39, 40, bytes.len() - 1] {
        assert!(Message::read(&mut Cursor::new(&bytes[..length]), 42, 2).is_err());
    }
    assert_eq!(
        Message::read(&mut Cursor::new(bytes), 42, 2)
            .expect("valid packet")
            .data
            .len(),
        AUDIO_BYTES
    );
}

#[test]
fn pcm_requires_contiguous_bounded_samples_and_terminal_eos() {
    let mut sequence = PcmSequence::default();
    assert!(sequence.finish().is_err());
    for length in [0, 3, AUDIO_BYTES + 4] {
        assert!(sequence.push(length, 0).is_err());
    }
    assert!(sequence.push(AUDIO_BYTES, 1).is_err());
    for tick in 0..900 {
        sequence
            .push(AUDIO_BYTES, crate::timestamp(tick))
            .expect("bounded samples");
    }
    assert!(sequence.push(4, LIMIT_US).is_err());
    sequence.finish().expect("end");
    assert!(sequence.push(4, LIMIT_US).is_err());
}
