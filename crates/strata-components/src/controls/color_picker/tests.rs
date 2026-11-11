//! Unit tests: hex text, `rune::Color` conversions, HSV, recent colours and the state model.

use rune::Color;

use super::state::Part;
use super::*;

fn hex(s: &str) -> Option<Srgba> {
    Srgba::parse_hex(s)
}

#[test]
fn hex_reads_every_form_in_either_case_with_or_without_the_hash() {
    let c = Srgba::rgb(0xaa, 0xbb, 0xcc);
    assert_eq!(hex("#abc"), Some(c));
    assert_eq!(hex("#ABC"), Some(c));
    assert_eq!(hex("abc"), Some(c));
    assert_eq!(hex("#aabbcc"), Some(c));
    assert_eq!(hex("AaBbCc"), Some(c));
    assert_eq!(hex("  #aabbcc \t"), Some(c));
    assert_eq!(hex("#abcd"), Some(Srgba::rgba(0xaa, 0xbb, 0xcc, 0xdd)));
    assert_eq!(hex("#aabbcc80"), Some(Srgba::rgba(0xaa, 0xbb, 0xcc, 0x80)));
    assert_eq!(hex("#000"), Some(Srgba::BLACK));
    assert_eq!(hex("#fff"), Some(Srgba::WHITE));
    assert_eq!(
        "#12345678".parse::<Srgba>(),
        Ok(Srgba::rgba(0x12, 0x34, 0x56, 0x78))
    );
}

#[test]
fn hex_refuses_anything_else() {
    for bad in [
        "",
        "#",
        "#1",
        "#12",
        "#12345",
        "#1234567",
        "#123456789",
        "##fff",
        "#ggg",
        "#12 34 56",
        "#+12",
        "#-12",
        "#éé",
        "rgb(1,2,3)",
        "red",
        "#fff#",
    ] {
        assert_eq!(hex(bad), None, "{bad:?}");
    }
    assert_eq!("#12".parse::<Srgba>(), Err(ParseHexError));
}

#[test]
fn hex_agrees_with_rune_on_the_forms_both_read() {
    for text in [
        "#abc",
        "#1e1e2e",
        "#89b4fa",
        "#4fd6ff80",
        "#00000000",
        "#ffffffff",
    ] {
        let ours: Color = hex(text).unwrap().into();
        let rune = rune::color::hex(text).unwrap();
        for (a, b) in [
            (ours.r, rune.r),
            (ours.g, rune.g),
            (ours.b, rune.b),
            (ours.a, rune.a),
        ] {
            assert!((a - b).abs() < 1e-6, "{text}: {ours:?} vs {rune:?}");
        }
    }
}

#[test]
fn hex_is_lowercase_and_six_digits_when_opaque() {
    assert_eq!(Srgba::rgb(0xAB, 0x01, 0xEF).hex(), "#ab01ef");
    assert_eq!(Srgba::rgba(0xAB, 0x01, 0xEF, 0x80).hex(), "#ab01ef80");
    assert_eq!(Srgba::rgba(0xAB, 0x01, 0xEF, 0x80).hex_rgb(), "#ab01ef");
    assert_eq!(Srgba::rgba(1, 2, 3, 0).to_string(), "#01020300");
}

#[test]
fn every_byte_survives_a_trip_through_rune_color() {
    for v in 0..=255u8 {
        for c in [
            Srgba::rgba(v, 0, 0, 255),
            Srgba::rgba(0, v, 0, 255),
            Srgba::rgba(0, 0, v, v),
        ] {
            let back = Srgba::from(Color::from(c));
            assert_eq!(back, c);
            assert_eq!(back.hex(), c.hex());
        }
    }
}

#[test]
fn a_rune_color_rounds_as_its_own_hex_does_and_clamps() {
    for c in [
        rune::oklcha(0.6, 0.1, 250.0, 0.5),
        rune::oklch(0.9, 0.05, 30.0),
        Color {
            r: 0.2140,
            g: 0.5,
            b: 0.0031,
            a: 0.777,
        },
    ] {
        let ours = Srgba::from(c);
        assert_eq!(format!("{}{:02x}", ours.hex_rgb(), ours.a), c.hex());
    }
    let wild = Color {
        r: 2.0,
        g: -1.0,
        b: f32::NAN,
        a: 1.5,
    };
    assert_eq!(Srgba::from(wild), Srgba::rgba(255, 0, 0, 255));
}

#[test]
fn every_colour_survives_a_trip_through_hsv() {
    // Every grey, then a lattice through the cube that meets every channel value.
    for v in 0..=255u8 {
        let c = Srgba::rgba(v, v, v, v);
        assert_eq!(Hsv::of(c).to_srgba(), c);
    }
    for r in (0..=255u8).step_by(3) {
        for g in (0..=255u8).step_by(5) {
            for b in (0..=255u8).step_by(7) {
                let c = Srgba::rgb(r, g, b);
                assert_eq!(Hsv::of(c).to_srgba(), c, "{c}");
                let c = Srgba::rgb(b, r, g);
                assert_eq!(Hsv::of(c).to_srgba(), c, "{c}");
            }
        }
    }
}

#[test]
fn hsv_reads_as_gtk_and_css_do() {
    let h = Hsv::of(Srgba::rgb(0, 0x80, 0xff));
    assert!((h.h - 209.88).abs() < 0.01, "{h:?}");
    assert!((h.s - 1.0).abs() < 1e-6 && (h.v - 1.0).abs() < 1e-6);
    let red = Hsv {
        h: 360.0,
        s: 1.0,
        v: 1.0,
        a: 1.0,
    };
    assert_eq!(red.to_srgba(), Srgba::rgb(255, 0, 0), "360° is red again");
    let half = Hsv {
        h: 120.0,
        s: 0.5,
        v: 0.5,
        a: 0.5,
    };
    assert_eq!(half.to_srgba(), Srgba::rgba(64, 128, 64, 128));
}

#[test]
fn a_grey_keeps_the_hue_and_black_the_saturation() {
    let before = Hsv::of(Srgba::rgb(0x34, 0xd3, 0x99));
    let grey = Hsv::of_keeping(Srgba::rgb(0x80, 0x80, 0x80), before);
    assert_eq!(grey.h, before.h);
    assert_eq!(grey.s, 0.0);
    let black = Hsv::of_keeping(Srgba::BLACK, before);
    assert_eq!((black.h, black.s, black.v), (before.h, before.s, 0.0));
    assert_eq!(
        Hsv::of(Srgba::rgb(9, 9, 9)).h,
        0.0,
        "with nothing to keep, grey's hue is 0"
    );
}

#[test]
fn recent_colours_are_newest_first_without_copies_up_to_the_cap() {
    let c = |v| Srgba::rgb(v, v, v);
    let mut recent = Vec::new();
    for v in 0..10 {
        remember(&mut recent, c(v), RECENT_MAX);
    }
    assert_eq!(recent, (2..10).rev().map(c).collect::<Vec<_>>());
    remember(&mut recent, c(5), RECENT_MAX);
    assert_eq!(recent, [5, 9, 8, 7, 6, 4, 3, 2].map(c));
}

/// A picker laid out with a 216 by 136 plane area and 216-wide strip tracks, inside an
/// 8-point margin.
fn laid_out(c: Srgba, alpha: bool) -> ColorPickerState {
    let mut st = ColorPickerState::new(Srgba::WHITE)
        .alpha(alpha)
        .with_value(c);
    st.set_size(Part::Plane, 232.0, 152.0);
    st.set_size(Part::Hue, 232.0, 24.0);
    st.set_size(Part::Alpha, 232.0, 24.0);
    st
}

const M: f32 = 8.0;

#[test]
fn without_alpha_the_colour_stays_opaque() {
    let mut st = ColorPickerState::new(Srgba::rgba(1, 2, 3, 4));
    assert_eq!(st.value(), Srgba::rgb(1, 2, 3));
    st.set_value(Srgba::rgba(9, 9, 9, 9));
    assert_eq!(st.value(), Srgba::rgb(9, 9, 9));
    st.edit_mut().replace_text("#11223344");
    st.edited();
    assert!(!st.invalid(), "eight digits read, their alpha dropped");
    assert!(st.commit_text());
    assert_eq!(st.value(), Srgba::rgb(0x11, 0x22, 0x33));
    assert_eq!(st.edit().text, "#112233");
    let glass = ColorPickerState::new(Srgba::WHITE)
        .alpha(true)
        .with_value(Srgba::rgba(1, 2, 3, 4));
    assert_eq!(glass.value(), Srgba::rgba(1, 2, 3, 4));
    assert_eq!(glass.edit().text, "#01020304");
}

#[test]
fn a_drag_through_black_keeps_the_hue() {
    let mut st = laid_out(Srgba::rgb(0x34, 0xd3, 0x99), false);
    let hue = st.hsv().h;
    assert!(st.press(Part::Plane, 200.0, 40.0, M));
    assert!(st.drag(200.0, 400.0, M), "to the bottom edge: black");
    assert_eq!(st.value(), Srgba::BLACK);
    assert!(st.drag(200.0, 40.0, M));
    assert_eq!(st.hsv().h, hue);
    assert!(st.value().g > st.value().r, "still green");
    assert_eq!(st.release(false), Some(ColorChange::Commit));
}

#[test]
fn a_cancelled_drag_puts_the_colour_back_and_an_unmoved_one_reports_nothing() {
    let start = Srgba::rgb(0x89, 0xb4, 0xfa);
    let mut st = laid_out(start, false);
    st.press(Part::Hue, 20.0, 12.0, M);
    st.drag(30.0, 12.0, M);
    assert_ne!(st.value(), start);
    assert_eq!(st.release(true), Some(ColorChange::Cancel));
    assert_eq!(st.value(), start);
    assert_eq!(st.edit().text, start.hex());
    // A press and release where the cursor is already: nothing moved.
    let x = M + st.hsv().s * 216.0;
    let y = M + (1.0 - st.hsv().v) * 136.0;
    st.press(Part::Plane, x, y, M);
    assert_eq!(st.release(false), None);
}

#[test]
fn the_plane_maps_its_corners_to_white_hue_and_black() {
    let mut st = laid_out(Srgba::rgb(255, 0, 0), false);
    st.press(Part::Plane, 0.0, 0.0, M);
    assert_eq!(st.value(), Srgba::WHITE, "top left");
    st.drag(500.0, -50.0, M);
    assert_eq!(st.value(), Srgba::rgb(255, 0, 0), "top right, clamped");
    st.drag(120.0, 152.0, M);
    assert_eq!(st.value(), Srgba::BLACK, "bottom");
}

#[test]
fn strips_land_on_whole_degrees_and_percents_and_hold_at_their_ends() {
    let mut st = laid_out(Srgba::rgb(255, 0, 0), true);
    st.press(Part::Hue, M + 216.0 / 3.0 + 0.1, 12.0, M);
    assert_eq!(st.hsv().h, 120.0);
    assert_eq!(st.value(), Srgba::rgb(0, 255, 0));
    st.release(false);
    assert!(st.strip_step(Part::Hue, 1));
    assert_eq!(st.hsv().h, 121.0);
    assert!(st.strip_end(Part::Hue, true));
    assert!(!st.strip_step(Part::Hue, 1), "360 holds");
    assert!(st.strip_end(Part::Alpha, false));
    assert_eq!(st.value().a, 0);
    assert!(!st.strip_step(Part::Alpha, -10));
    assert!(st.strip_step(Part::Alpha, 10));
    assert_eq!(st.value().a, 26, "10%");
}

#[test]
fn plane_keys_step_whole_percents_from_the_nearest() {
    let mut st = laid_out(Srgba::rgb(0x34, 0xd3, 0x99), false);
    let (s, v) = (st.hsv().s, st.hsv().v);
    // 75.4% and 82.7%: each steps from its nearest whole percent.
    assert!(st.plane_step(1, 0));
    assert_eq!(st.hsv().s, ((s * 100.0).round() + 1.0) / 100.0);
    assert_eq!(st.hsv().v, v, "Right leaves the value");
    assert!(st.plane_step(0, 1));
    assert!((st.hsv().v - ((v * 100.0).round() + 1.0) / 100.0).abs() < 1e-6);
    assert!(st.plane_step(0, -10));
    assert!((st.hsv().v - ((v * 100.0).round() - 9.0) / 100.0).abs() < 1e-6);
    st.set_value(Srgba::WHITE);
    assert!(!st.plane_step(0, 1), "full value holds");
}

#[test]
fn typed_hex_commits_reverts_and_waits_for_the_blur() {
    let start = Srgba::rgb(0x34, 0xd3, 0x99);
    let mut st = laid_out(start, false);
    st.edit_mut().replace_text("#12");
    st.edited();
    assert!(st.invalid());
    assert!(st.revert_text(), "Escape reverts typed text");
    assert!(!st.revert_text(), "then goes on");
    assert_eq!(st.edit().text, "#34d399");
    // A blur commits only after the field has had the keyboard, and not to its menu.
    st.edit_mut().replace_text("abc");
    assert_eq!(st.blurred(), None, "never focused");
    st.edit.focused = true;
    assert_eq!(st.blurred(), None);
    st.hold(true);
    st.edit.focused = false;
    assert_eq!(st.blurred(), None, "the menu has it");
    st.hold(false);
    assert_eq!(st.blurred(), Some(true));
    assert_eq!(st.value(), Srgba::rgb(0xaa, 0xbb, 0xcc));
    assert_eq!(st.edit().text, "#aabbcc");
    assert_eq!(st.blurred(), None, "once");
}

#[test]
fn typed_text_commits_before_another_part_moves_the_colour() {
    let mut st = laid_out(Srgba::rgb(255, 0, 0), false);
    st.edit_mut().replace_text("#00f");
    assert!(st.commit_pending());
    assert_eq!(st.hsv().h, 240.0, "the plane then works in the typed hue");
    assert!(!st.commit_pending(), "nothing left");
    st.edit_mut().replace_text("nope");
    assert!(!st.commit_pending());
    assert_eq!(st.edit().text, "#0000ff", "bad text reverts");
}

#[test]
fn the_popover_cancels_to_its_opening_colour_and_keeps_otherwise() {
    let start = Srgba::rgb(0x89, 0xb4, 0xfa);
    let mut st = laid_out(start, false);
    st.open();
    assert!(st.is_open());
    st.pick(Srgba::rgb(1, 2, 3));
    st.open();
    assert_eq!(
        st.close(true),
        (false, true),
        "back to the first opening colour"
    );
    assert_eq!(st.value(), start);
    assert!(!st.is_open());
    assert_eq!(st.close(true), (false, false), "closed already");
    st.open();
    st.pick(Srgba::rgb(1, 2, 3));
    st.edit_mut().replace_text("#456");
    assert_eq!(
        st.close(false),
        (true, true),
        "typed text commits on a press outside"
    );
    assert_eq!(st.value(), Srgba::rgb(0x44, 0x55, 0x66));
    st.open();
    assert_eq!(st.close(false), (false, false));
}

#[test]
fn a_swatch_or_the_host_keeps_the_hue_for_a_grey() {
    let mut st = laid_out(Srgba::rgb(0, 0, 255), false);
    assert!(st.pick(Srgba::rgb(0x80, 0x80, 0x80)));
    assert_eq!(st.hsv().h, 240.0);
    assert!(!st.pick(Srgba::rgb(0x80, 0x80, 0x80)));
    st.set_value(Srgba::BLACK);
    assert_eq!(st.hsv().h, 240.0);
}
