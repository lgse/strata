// SPDX-License-Identifier: MIT

use std::{error::Error, io::Read};

use super::super::{MemberContent, MemberMetadata};
use super::harness::{Harness, file, unread};

/// The free space the session is told about: what the folder reports, or an injected figure.
enum Space {
    Measured,
    Injected(Option<u64>),
}

enum Payload {
    Bytes(&'static [u8]),
    /// A member the session must refuse before reading it.
    Unread,
}

enum Message {
    Exact(&'static str),
    Containing(&'static str),
}

struct Refusal {
    space: Space,
    name: &'static str,
    payload: Payload,
    declared: Option<u64>,
    message: Message,
}

/// A member the destination cannot hold fails the extraction, leaving nothing behind.
fn a_member_is_refused(refusal: Refusal) -> Result<(), Box<dyn Error>> {
    let mut harness = Harness::new()?;
    let mut session = match refusal.space {
        Space::Measured => harness.open()?,
        Space::Injected(available) => harness.open_with_available_bytes(available)?,
    };
    let mut unread = unread("a member the destination cannot hold must not be read");
    let mut bytes: &[u8];
    let reader: &mut dyn Read = match refusal.payload {
        Payload::Bytes(payload) => {
            bytes = payload;
            &mut bytes
        }
        Payload::Unread => &mut unread,
    };

    let result = session.extract_member(
        refusal.name,
        MemberContent::File(reader, refusal.declared),
        MemberMetadata::NONE,
    );
    let message = session.finish_failed(result);

    match refusal.message {
        Message::Exact(expected) => assert_eq!(message, expected),
        Message::Containing(fragment) => assert!(message.contains(fragment), "{message}"),
    }
    assert!(!message.contains("remain in"), "{message}");
    session.assert_extracted(0, [])
}

#[test]
fn declared_size_overflow_removes_partial_output() -> Result<(), Box<dyn Error>> {
    a_member_is_refused(Refusal {
        space: Space::Injected(Some(1024)),
        name: "overflow.txt",
        payload: Payload::Bytes(b"abcdefgh"),
        declared: Some(4),
        message: Message::Exact("Archive member “overflow.txt” declared 4 bytes but produced more"),
    })
}

#[test]
fn declared_size_shortfall_removes_partial_output() -> Result<(), Box<dyn Error>> {
    a_member_is_refused(Refusal {
        space: Space::Measured,
        name: "short.txt",
        payload: Payload::Bytes(b"four"),
        declared: Some(8),
        message: Message::Exact("Archive member “short.txt” declared 8 bytes but produced 4 bytes"),
    })
}

#[test]
fn member_preflight_refuses_when_destination_lacks_space() -> Result<(), Box<dyn Error>> {
    a_member_is_refused(Refusal {
        space: Space::Injected(Some(4)),
        name: "huge.txt",
        payload: Payload::Unread,
        declared: Some(8),
        message: Message::Containing("declared 8 B, but only 4 B is free"),
    })
}

#[test]
fn copy_without_declared_size_stops_at_free_space() -> Result<(), Box<dyn Error>> {
    a_member_is_refused(Refusal {
        space: Space::Injected(Some(4)),
        name: "payload.txt",
        payload: Payload::Bytes(b"12345678"),
        declared: None,
        message: Message::Exact(
            "Not enough free space at the destination to extract “payload.txt” (4 B available)",
        ),
    })
}

#[test]
fn unreported_free_space_still_enforces_declared_size() -> Result<(), Box<dyn Error>> {
    a_member_is_refused(Refusal {
        space: Space::Injected(None),
        name: "overflow.txt",
        payload: Payload::Bytes(b"abcdefgh"),
        declared: Some(4),
        message: Message::Containing("declared 4 bytes but produced more"),
    })
}

#[test]
fn claimed_total_preflight_refuses_before_any_member() -> Result<(), Box<dyn Error>> {
    let mut harness = Harness::new()?;
    let mut session = harness.open_with_available_bytes(Some(10))?;

    let result = session.preflight_claimed_size(100);
    let message = session.finish_failed(result);

    assert!(
        message.contains(
            "Archive declared size (100 B) exceeds the free space at the destination (10 B)"
        ),
        "{message}"
    );
    assert!(!message.contains("remain in"), "{message}");
    session.assert_extracted(0, [])
}

#[test]
fn second_member_preflight_uses_remaining_space() -> Result<(), Box<dyn Error>> {
    let mut harness = Harness::new()?;
    let mut session = harness.open_with_available_bytes(Some(10))?;
    session.extract_file("first.txt", b"12345678")?;
    let mut reader = unread("second member that cannot fit must not be read");

    let result = session.extract_member(
        "second.txt",
        MemberContent::File(&mut reader, Some(4)),
        MemberMetadata::NONE,
    );
    let message = session.finish_failed(result);

    assert!(
        message.contains("declared 4 B, but only 2 B is free"),
        "{message}"
    );
    assert!(
        message.ends_with("free at the destination. Extracted entries remain in “archive”."),
        "{message}"
    );
    session.assert_extracted(1, [("archive/first.txt", file(b"12345678"))])
}

#[test]
fn matching_declared_size_completes_under_an_injected_quota() -> Result<(), Box<dyn Error>> {
    let mut harness = Harness::new()?;
    let mut session = harness.open_with_available_bytes(Some(16))?;
    session.extract_file("ok.txt", b"contents")?;

    let published = session.finish_completed(Ok(()), Vec::new)?;

    assert_eq!(published.as_deref(), Some("ok.txt"));
    session.assert_extracted(1, [("ok.txt", file(b"contents"))])
}

#[test]
fn unreported_free_space_skips_capacity_checks() -> Result<(), Box<dyn Error>> {
    let mut harness = Harness::new()?;
    let mut session = harness.open_with_available_bytes(None)?;

    session.preflight_claimed_size(u128::MAX)?;
    session.extract_file("declared.txt", b"12345678")?;
    session.extract_member(
        "undeclared.txt",
        MemberContent::File(&mut &b"12345678"[..], None),
        MemberMetadata::NONE,
    )?;

    let published = session.finish_completed(Ok(()), Vec::new)?;
    assert_eq!(published.as_deref(), Some("archive"));
    session.assert_extracted(
        2,
        [
            ("archive/declared.txt", file(b"12345678")),
            ("archive/undeclared.txt", file(b"12345678")),
        ],
    )
}
