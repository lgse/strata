use super::wayland::{attach_failed, no_shared_conn};
use super::x11::with_state;
use super::*;
use crate::transfer::{Error, Status, Unsupported};

fn xid(s: &str) -> u32 {
    match Parent::parse(s) {
        Ok(Parent::X11(x)) => x.get(),
        other => panic!("{s:?}: {other:?}"),
    }
}

fn wayland(s: &str) -> String {
    match Parent::parse(s) {
        Ok(Parent::Wayland(h)) => h.as_str().to_owned(),
        other => panic!("{s:?}: {other:?}"),
    }
}

#[test]
fn an_empty_or_blank_handle_is_no_parent() {
    for s in ["", "   ", "\t\n"] {
        assert_eq!(Parent::parse(s), Ok(Parent::None), "{s:?}");
    }
}

#[test]
fn x11_takes_bare_hex_and_one_0x_prefix() {
    // GTK, then Chromium (S3, Q3).
    assert_eq!(xid("x11:400003"), 0x40_0003);
    assert_eq!(xid("x11:0x400003"), 0x40_0003);
    assert_eq!(xid("x11:0X4a"), 0x4a);
    assert_eq!(xid("x11:FFFFFFFF"), u32::MAX);
    assert_eq!(xid("x11:0x00000001"), 1);
    assert_eq!(xid("  x11:1c  "), 0x1c);
}

#[test]
fn x11_rejects_what_isnt_a_nonzero_id() {
    for s in [
        "x11:",
        "x11:0x",
        "x11:0x0x1",
        "x11:0",
        "x11:0x0",
        "x11:+1",
        "x11:-1",
        "x11:1g",
        "x11:1 2",
        "x11:123456789",
        "x11:0x123456789",
        "x11:٣",
    ] {
        assert_eq!(Parent::parse(s), Err(ParseError::BadXid), "{s:?}");
    }
}

#[test]
fn wayland_handles_stay_opaque() {
    assert_eq!(
        wayland("wayland:0123456789abcdef0123456789abcdef"),
        "0123456789abcdef0123456789abcdef"
    );
    // KWin's UUIDs, odd bytes and a colon are all kept as they are.
    assert_eq!(wayland("wayland:{6e1b…}:x y"), "{6e1b…}:x y");
    assert_eq!(wayland(" wayland:abc\n"), "abc");
}

#[test]
fn wayland_rejects_handles_no_message_can_carry() {
    assert_eq!(Parent::parse("wayland:"), Err(ParseError::EmptyHandle));
    assert_eq!(Parent::parse("wayland:a\0b"), Err(ParseError::BadHandle));
    let at_cap = format!("wayland:{}", "a".repeat(MAX_WAYLAND_HANDLE));
    assert!(matches!(Parent::parse(&at_cap), Ok(Parent::Wayland(_))));
    let over = format!("wayland:{}", "a".repeat(MAX_WAYLAND_HANDLE + 1));
    assert_eq!(
        Parent::parse(&over),
        Err(ParseError::TooLong(MAX_WAYLAND_HANDLE + 1))
    );
}

#[test]
fn other_kinds_and_shapes_are_errors() {
    assert_eq!(Parent::parse("400003"), Err(ParseError::Unrecognised));
    assert_eq!(
        Parent::parse("X11:400003"),
        Err(ParseError::UnknownKind("X11".into()))
    );
    assert_eq!(
        Parent::parse("Wayland:abc"),
        Err(ParseError::UnknownKind("Wayland".into()))
    );
    let long = format!("{}é:1", "k".repeat(31));
    let Err(ParseError::UnknownKind(kind)) = Parent::parse(&long) else {
        panic!("an unknown kind");
    };
    assert_eq!(kind, "k".repeat(31), "cut on a character boundary");
}

#[test]
fn a_parent_prints_as_the_portal_spells_it() {
    for s in ["", "wayland:abc", "x11:400003"] {
        assert_eq!(Parent::parse(s).unwrap().to_string(), s);
    }
    assert_eq!(Parent::parse("x11:0x00ABC").unwrap().to_string(), "x11:abc");
    assert_eq!("x11:5".parse::<Parent>(), Parent::parse("x11:5"));
}

#[test]
fn the_modal_state_is_added_or_removed_keeping_the_others() {
    let (above, max, modal) = (7, 8, 9);
    assert_eq!(
        with_state(&[above, max], modal, true),
        Some(vec![above, max, modal])
    );
    assert_eq!(
        with_state(&[above, modal, max], modal, false),
        Some(vec![above, max])
    );
    assert_eq!(with_state(&[modal], modal, true), None);
    assert_eq!(with_state(&[above], modal, false), None);
    assert_eq!(with_state(&[], modal, true), Some(vec![modal]));
}

#[test]
fn an_unreachable_display_follows_decision_1() {
    let parentless = Outcome::parentless(Reason::NoDisplay, Fallback::Compositor);
    assert_eq!(
        attach_failed(&Error::Unsupported(Unsupported::LibwaylandBelow1_23)),
        parentless
    );
    let status = Status {
        backend: "wayland",
        shared: false,
        own: true,
        data_control: Some("ext"),
        unsupported: Some(Unsupported::LibwaylandBelow1_23),
        x11: false,
    };
    assert_eq!(no_shared_conn(&status), parentless);
}

#[test]
fn other_attach_failures_are_reported_as_they_are() {
    assert_eq!(
        attach_failed(&Error::WindowClosed("gone".into())),
        Outcome::parentless(Reason::WindowClosed, Fallback::Compositor)
    );
    assert_eq!(
        attach_failed(&Error::Unsupported(Unsupported::NotLinux("Win32".into()))),
        Outcome::parentless(Reason::NotLinux("Win32".into()), Fallback::Compositor)
    );
    assert!(matches!(
        attach_failed(&Error::Connection("broken pipe".into())),
        Outcome::Parentless {
            reason: Reason::Failed(e),
            fallback: Fallback::Compositor
        } if e.contains("broken pipe")
    ));
}
