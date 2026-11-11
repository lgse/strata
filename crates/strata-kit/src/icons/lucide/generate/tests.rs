//! What the generator accepts from a source, what it refuses (and so stops the build), and
//! what it makes of an accepted one.

use super::*;

fn svg(root: &str, body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"{root}>{body}</svg>"#
    )
}

const LUCIDE_ROOT: &str = r##" fill="none" stroke="#8bc9eb" stroke-width="2""##;

/// The generated source for `sources`.
fn generated(sources: Vec<Source>) -> String {
    catalog(sources).expect("a catalog")
}

fn refused(name: &str, svg: &str, kind: Kind) -> String {
    match icon(name, svg, kind) {
        Ok(_) => panic!("{name} was accepted"),
        Err(why) => why,
    }
}

#[test]
fn a_lucide_icon_takes_the_tint_and_keeps_its_root_paint_in_a_group() {
    let source = svg(LUCIDE_ROOT, r#"<path d="M1 1h2"/>"#);
    let out = generated(vec![
        icon("strata-x", &source, Kind::Lucide).expect("accepted"),
    ]);
    let body =
        r#"<g fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"><path d=\"M1 1h2\"/></g>"#;
    assert!(out.contains(body), "{out}");
    assert!(!out.contains("8bc9eb"));
}

#[test]
fn only_the_check_on_accent_may_be_white() {
    let white = svg(r##" stroke="#ffffff""##, r#"<path d="M1 1h2"/>"#);
    let out = generated(vec![
        icon("strata-check-on-primary", &white, Kind::Lucide).expect("ok"),
    ]);
    assert!(out.contains(r#"stroke=\"currentColor\""#), "{out}");
    let why = refused("strata-check", &white, Kind::Lucide);
    assert!(why.contains("#ffffff"), "{why}");
}

#[test]
fn a_catppuccin_icon_resolves_in_each_flavour_and_fills_faintly_under_theme_colours() {
    let source = svg(
        "",
        r#"<path fill="none" stroke="var(--vscode-ctp-peach)" d="M1 1h2"/>"#,
    );
    let out = generated(vec![
        icon("strata-ctp-rust", &source, Kind::Catppuccin).expect("ok"),
    ]);
    for want in [
        r##"stroke=\"#fab387\""##,
        r##"stroke=\"#fe640b\""##,
        r#"fill=\"currentColor\" fill-opacity=\"0.15\" stroke=\"currentColor\""#,
    ] {
        assert!(out.contains(want), "{want} in {out}");
    }
}

#[test]
fn sources_with_unexpected_content_are_refused() {
    let lucide = |body: &str| svg(LUCIDE_ROOT, body);
    let ctp = |body: &str| svg("", body);
    let cases = [
        (
            lucide(r#"<path style="fill:red" d="M1 1"/>"#),
            Kind::Lucide,
            "`style`",
        ),
        (
            lucide("<style>path{fill:red}</style>"),
            Kind::Lucide,
            "<style>",
        ),
        (lucide(r##"<use href="#a"/>"##), Kind::Lucide, "<use>"),
        (lucide(r#"<image href="x.png"/>"#), Kind::Lucide, "<image>"),
        (
            lucide("<script>alert(1)</script>"),
            Kind::Lucide,
            "<script>",
        ),
        (lucide(r#"<svg viewBox="0 0 1 1"/>"#), Kind::Lucide, "<svg>"),
        (
            lucide(r#"<path class="a" d="M1 1"/>"#),
            Kind::Lucide,
            "`class`",
        ),
        (
            lucide(r#"<path stroke="red" d="M1 1"/>"#),
            Kind::Lucide,
            "`red`",
        ),
        (
            lucide(r##"<path stroke="#123456" d="M1 1"/>"##),
            Kind::Lucide,
            "#123456",
        ),
        (
            lucide(r#"<path fill="url(https://x/y.svg#a)" d="M1 1"/>"#),
            Kind::Lucide,
            "outside",
        ),
        (
            ctp(r#"<path clip-path="url(other.svg#a)" d="M1 1"/>"#),
            Kind::Catppuccin,
            "outside",
        ),
        (
            ctp(r##"<path stroke="#fab387" d="M1 1"/>"##),
            Kind::Catppuccin,
            "#fab387",
        ),
        (
            ctp(r#"<path stroke="white" d="M1 1"/>"#),
            Kind::Catppuccin,
            "`white`",
        ),
        (
            ctp(r#"<path stroke="var(--vscode-ctp-crust)" d="M1 1"/>"#),
            Kind::Catppuccin,
            "crust",
        ),
        (
            ctp(r#"<path stroke="var(--other)" d="M1 1"/>"#),
            Kind::Catppuccin,
            "Catppuccin's",
        ),
    ];
    for (source, kind, says) in cases {
        let why = refused("strata-ctp-x", &source, kind);
        assert!(why.contains(says), "{says}: {why}");
    }
}

#[test]
fn text_entities_and_markup_outside_the_root_are_refused() {
    let lucide = |body: &str| svg(LUCIDE_ROOT, body);
    let path = r#"<path d="M1 1h2"/>"#;
    let cases = [
        (lucide(&format!("{path}hello")), "stray text `hello`"),
        (lucide(&format!("hello{path}")), "stray text `hello`"),
        (lucide(&format!("<g>{path}</g>bye")), "stray text `bye`"),
        (lucide(&format!("{path}&#60;script&#62;")), "`&`"),
        (lucide(r#"<path d="M1&#x20;1h2"/>"#), "`&`"),
        (
            format!("<?xml version=\"1.0\"?>{}", lucide(path)),
            "outside the <svg>",
        ),
        (
            format!(
                r#"<!DOCTYPE svg [<!ENTITY a "b">]>{}"#,
                lucide(r#"<path d="M1 1"/>"#)
            ),
            "outside the <svg>",
        ),
        (format!("{}<path/>", lucide(path)), "outside the <svg>"),
    ];
    for (source, says) in cases {
        let why = refused("strata-x", &source, Kind::Lucide);
        assert!(why.contains(says), "{says}: {why}");
    }
    // Whitespace and comments around and inside the root stay fine.
    let fine = format!(
        "<!-- a note -->\n{}\n",
        lucide(&format!(" {path}\n  {path} "))
    );
    assert!(icon("strata-x", &fine, Kind::Lucide).is_ok());
}

#[test]
fn a_root_or_viewbox_rune_cant_draw_is_refused() {
    let body = r#"<path d="M1 1"/>"#;
    let off = svg("", body).replace("0 0 24 24", "2 2 24 24");
    assert!(refused("strata-a", &off, Kind::Lucide).contains("viewBox"));
    let wide = svg("", body).replace("0 0 24 24", "0 0 24 16");
    assert!(refused("strata-a", &wide, Kind::Lucide).contains("viewBox"));
    let class = svg(r#" class="icon""#, body);
    assert!(refused("strata-a", &class, Kind::Lucide).contains("root attribute"));
}

#[test]
fn a_name_must_start_with_a_letter_and_stay_lowercase() {
    let source = svg("", r#"<path d="M1 1"/>"#);
    assert!(refused("3d", &source, Kind::Lucide).contains("starts with a lowercase letter"));
    assert!(refused("strata-Folder", &source, Kind::Lucide).contains("lowercase"));
}

#[test]
fn what_runes_single_colour_rasterizer_ignores_is_allowed() {
    let source = svg(
        "",
        r##"<defs><clipPath id="a"><path fill="#fff" d="M0 0h16v16H0z"/></clipPath></defs><path clip-path="url(#a)" stroke-dasharray="1 1.5" stroke-miterlimit="4" stroke="var(--vscode-ctp-sky)" d="M1 1"/>"##,
    );
    assert!(icon("strata-ctp-humans", &source, Kind::Catppuccin).is_ok());
}

#[test]
fn two_sources_with_one_name_or_identifier_are_refused() {
    let source = svg("", r#"<path d="M1 1"/>"#);
    let one = || icon("strata-a-b", &source, Kind::Lucide).expect("ok");
    let why = catalog(vec![one(), one()]).expect_err("refused");
    assert!(why.contains("two folders"), "{why}");
    let other = icon("strata-a_b", &source, Kind::Lucide).expect("ok");
    let why = catalog(vec![one(), other]).expect_err("refused");
    assert!(why.contains("identifier"), "{why}");
}

#[test]
fn the_output_follows_the_names_not_the_reading_order() {
    let source = svg("", r#"<path d="M1 1"/>"#);
    let make = |names: [&str; 3]| {
        let sources = names.map(|n| icon(n, &source, Kind::Lucide).expect("ok"));
        generated(sources.into())
    };
    assert_eq!(
        make(["strata-b", "strata-a", "strata-c"]),
        make(["strata-c", "strata-b", "strata-a"])
    );
}
