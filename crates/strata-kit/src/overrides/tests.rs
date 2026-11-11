use super::*;
use crate::look::Look;
use crate::shape::LookParams;
use crate::slots;
use crate::theme::syntax::Role;
use crate::theme::{Rgba8, bundled_theme};
use crate::tokens::{self, Value};

/// A bundled theme's look at Strata's default settings: Tokyo Night when `dark`, Omarchy
/// Light otherwise.
fn theme_look(dark: bool) -> Look {
    themed(dark, &LookParams::default())
}

fn themed(dark: bool, params: &LookParams) -> Look {
    let id = if dark { "tokyo-night" } else { "omarchy-light" };
    let look = bundled_theme(id)
        .expect("bundled")
        .look()
        .with_params(params);
    assert_eq!(look.kit.dark, dark, "{id}");
    look
}

fn color(hex: &str) -> rune::Color {
    Rgba8::parse(hex).expect("a colour").color()
}

fn get(look: &Look, name: &str) -> Value {
    tokens::get(look, name).expect("a token")
}

fn at(line: usize, column: usize) -> Location {
    Location { line, column }
}

/// The file's problems as a user reads them.
fn messages(o: &Overrides) -> Vec<String> {
    o.errors().iter().map(ToString::to_string).collect()
}

/// Every registry token whose value differs between two looks.
fn changed(a: &Look, b: &Look) -> Vec<&'static str> {
    tokens::all()
        .filter(|t| t.get(a) != t.get(b))
        .map(|t| t.name())
        .collect()
}

#[test]
fn an_override_reaches_its_token_and_the_reader_its_component_calls() {
    let o = Overrides::parse(
        r##"
syntax.keyword = "#00ff00"

[sidebar.row]
height = 40

[sidebar.selected]
fill = "rgba(255, 0, 0, 0.5)"
"##,
    );
    assert_eq!(messages(&o), Vec::<String>::new());
    let look = o.applied(theme_look(true));
    assert_eq!(slots::sidebar::row_height(&look), 40.0);
    assert_eq!(
        slots::sidebar::selected_fill(&look),
        color("rgba(255, 0, 0, 0.5)")
    );
    // The syntect theme (K1b) reads its colours through the roles.
    assert_eq!(Role::Keyword.color(&look), color("#00ff00"));
    assert_eq!(o.len(), 3);
}

#[test]
fn a_core_override_moves_the_slot_tokens_that_inherit_it() {
    let look = Overrides::parse("row.list = 33").applied(theme_look(false));
    assert_eq!(slots::list::row_height(&look), 33.0);
    assert_eq!(get(&look, "list.row.height"), Value::Length(33.0));
}

/// The WP's test: a scoped override.
#[test]
fn a_scoped_override_applies_only_to_its_appearance_and_beats_an_unscoped_one() {
    // The scoped rows come first in the file, and still win.
    let o = Overrides::parse(
        r##"
dark.color.bg_base = "#111111"
color.bg_base = "#222222"
row.list = 30

[light.list.row]
height = 44
"##,
    );
    assert_eq!(messages(&o), Vec::<String>::new());
    let dark = o.applied(theme_look(true));
    let light = o.applied(theme_look(false));
    assert_eq!(dark.kit.c.bg_base, color("#111111"));
    assert_eq!(light.kit.c.bg_base, color("#222222"));
    assert_eq!(slots::list::row_height(&dark), 30.0);
    assert_eq!(slots::list::row_height(&light), 44.0);
}

/// The WP's test: an unknown token.
#[test]
fn an_unknown_token_is_reported_once_with_its_line_and_column() {
    let o = Overrides::parse(
        "row.list = 30\n  list.row.length = 30\n\n[list.roww]\nheight = 1\nwidth = 2\n",
    );
    assert_eq!(
        o.errors(),
        &[
            OverrideError {
                at: at(2, 3),
                scope: Scope::All,
                problem: Problem::Unknown("list.row.length".into()),
            },
            OverrideError {
                at: at(4, 1),
                scope: Scope::All,
                problem: Problem::Unknown("list.roww".into()),
            },
        ]
    );
    assert_eq!(
        messages(&o)[0],
        "line 2, column 3: unknown token `list.row.length`"
    );
    // The good row still applies.
    assert_eq!(o.applied(theme_look(true)).row.list, 30.0);
}

#[test]
fn a_scoped_problem_names_its_scope() {
    let o = Overrides::parse("[dark]\ncolor.bg_basee = \"#000\"\n");
    assert_eq!(
        messages(&o),
        ["line 2, column 1: unknown token `color.bg_basee` (in [dark])"]
    );
}

#[test]
fn a_value_on_a_group_of_tokens_names_one_of_them() {
    let o = Overrides::parse("list.row = 30");
    assert_eq!(o.errors()[0].problem, Problem::Group("list.row".into()));
    assert_eq!(
        messages(&o),
        [
            "line 1, column 1: `list.row` names a group of tokens: set one of them, such as `list.row.height`"
        ]
    );
}

/// The WP's test: a bad value. settings/themes: "A rule GTK cannot parse, such as
/// `label { color: nope; }`, is skipped with an "invalid rule in user stylesheet" warning."
#[test]
fn a_bad_value_reports_where_it_is_and_keeps_the_themes_value() {
    let cases: &[(&str, &str, Problem)] = &[
        (
            "row.columns",
            "\"28px\"",
            Problem::Kind {
                name: "row.columns",
                expected: tokens::Kind::Length,
                found: "string",
            },
        ),
        (
            "collection.cut.opacity",
            "1.5",
            Problem::Range {
                name: "collection.cut.opacity",
                min: 0.0,
                max: 1.0,
            },
        ),
        (
            "color.text_base",
            "\"nope\"",
            Problem::BadColor {
                name: "color.text_base",
                value: "nope".into(),
            },
        ),
        (
            "glow.on",
            "1",
            Problem::Kind {
                name: "glow.on",
                expected: tokens::Kind::Flag,
                found: "integer",
            },
        ),
        (
            "color.bg_base",
            "0xff0000",
            Problem::Kind {
                name: "color.bg_base",
                expected: tokens::Kind::Color,
                found: "integer",
            },
        ),
        (
            "row.list",
            "-1",
            Problem::Range {
                name: "row.list",
                min: 0.0,
                max: f32::INFINITY,
            },
        ),
        (
            "row.list",
            "nan",
            Problem::Range {
                name: "row.list",
                min: 0.0,
                max: f32::INFINITY,
            },
        ),
        (
            "radius.md",
            "1e40",
            Problem::Range {
                name: "radius.md",
                min: 0.0,
                max: f32::INFINITY,
            },
        ),
    ];
    let base = theme_look(true);
    for (name, value, problem) in cases {
        let src = format!("space.md = 9\n{name} = {value}\n");
        let o = Overrides::parse(&src);
        let column = name.chars().count() + 4;
        assert_eq!(
            o.errors(),
            &[OverrideError {
                at: at(2, column),
                scope: Scope::All,
                problem: problem.clone(),
            }],
            "{src}"
        );
        let look = o.applied(base.clone());
        assert_eq!(get(&look, name), get(&base, name), "{src}");
        assert_eq!(look.space.md, 9.0, "{src}");
    }
    assert_eq!(
        messages(&Overrides::parse("row.columns = \"28px\"")),
        [
            "line 1, column 15: `row.columns` takes a length, not a string: write the number without quotes or units"
        ]
    );
}

/// The WP's test: partial apply. settings/themes: "Valid rules in the same file still
/// apply."
#[test]
fn rows_that_parse_still_apply_around_syntax_errors() {
    // Each file sets row.list = 30, row.columns = 28 and list.row.height = 40 around one
    // break, on the lines given.
    let deep = format!(
        "row.list = 30\nrow.sidebar = {}1\nrow.columns = 28\n[list.row]\nheight = 40\n",
        "[".repeat(5000)
    );
    let mut cases: Vec<(&str, &str, &[usize])> = vec![
        (
            "an unquoted value",
            "row.list = 30\ncolor.bg_base = #fff\nrow.columns = 28\n[list.row]\nheight = 40\n",
            &[2],
        ),
        (
            "a line of garbage",
            "row.list = 30\nthis is not toml\nrow.columns = 28\n[list.row]\nheight = 40\n",
            &[2],
        ),
        (
            "an unterminated string",
            "row.list = 30\ncolor.bg_base = \"#fff\nrow.columns = 28\n[list.row]\nheight = 40\n",
            &[2],
        ),
        (
            "a key set twice",
            "row.list = 30\nrow.list = 31\nrow.columns = 28\n[list.row]\nheight = 40\n",
            &[2],
        ),
        (
            "a table defined twice",
            "row.list = 30\nrow.columns = 28\n[list.row]\nheight = 40\n[list.row]\nheight = 41\n",
            &[5],
        ),
        (
            "a broken table header",
            "row.list = 30\nrow.columns = 28\n[sidebar.row\nheight = 99\n[list.row]\nheight = 40\n",
            &[3],
        ),
        (
            "an unclosed inline table",
            "row.list = 30\nlist.row = { height = 30\nrow.columns = 28\n[list.row]\nheight = 40\n",
            &[2],
        ),
        (
            "an unclosed array",
            "row.list = 30\nrow.sidebar = [1,\nrow.columns = 28\n[list.row]\nheight = 40\n",
            &[2],
        ),
        (
            "an unclosed multi-line array",
            "row.list = 30\nrow.sidebar = [\n  1,\nrow.columns = 28\n[list.row]\nheight = 40\n",
            // The array's element left on its own line is reported too.
            &[2, 3],
        ),
        (
            "a form feed",
            "row.list = 30\n\x0c\nrow.columns = 28\n[list.row]\nheight = 40\n",
            &[2],
        ),
        (
            "a lone carriage return",
            "row.list = 30\nrow.sidebar = 3\rx\nrow.columns = 28\n[list.row]\nheight = 40\n",
            &[2],
        ),
        (
            "a bad value in a CRLF file",
            "row.list = 30\r\nrow.sidebar = x\r\nrow.columns = 28\r\n[list.row]\r\nheight = 40\r\n",
            &[2],
        ),
    ];
    cases.push(("an unclosed array too deep to parse", &deep, &[2]));
    for (what, src, lines) in cases {
        let o = Overrides::parse(src);
        let got: Vec<usize> = o.errors().iter().map(|e| e.at.line).collect();
        assert_eq!(&got, lines, "{what}: {:?}", messages(&o));
        assert!(
            o.errors()
                .iter()
                .all(|e| matches!(e.problem, Problem::Syntax(_))),
            "{what}: {:?}",
            messages(&o)
        );
        let look = o.applied(theme_look(false));
        assert_eq!(look.row.list, 30.0, "{what}");
        assert_eq!(look.row.columns, 28.0, "{what}");
        assert_eq!(slots::list::row_height(&look), 40.0, "{what}");
        assert_eq!(o.len(), 3, "{what}");
    }
}

#[test]
fn a_broken_table_header_skips_its_rows_and_says_so() {
    let base = theme_look(false);
    let o = Overrides::parse("[sidebar.row\nheight = 99\n");
    assert!(o.is_empty());
    assert!(
        messages(&o)[0].ends_with("; the rows under it don't apply"),
        "{:?}",
        messages(&o)
    );
    let look = o.applied(base.clone());
    assert_eq!(
        slots::sidebar::row_height(&look),
        slots::sidebar::row_height(&base)
    );
}

#[test]
fn a_string_left_open_is_reported_where_it_opens_and_the_rest_applies() {
    let o = Overrides::parse("row.list = 30\nrow.columns = \"\"\"\nrow.palette = 50\n");
    let at: Vec<_> = o.errors().iter().map(|e| e.at).collect();
    assert_eq!(
        at,
        [Location {
            line: 2,
            column: 15
        }],
        "{:?}",
        messages(&o)
    );
    let look = o.applied(theme_look(true));
    assert_eq!(look.row.list, 30.0);
    assert_eq!(look.row.palette, 50.0);
}

#[test]
fn a_row_whose_value_runs_over_a_broken_line_doesnt_apply() {
    let src = "color.scrim = \"\"\"red\n\\q\n\"\"\"\nrow.list = 30\n";
    let o = Overrides::parse(src);
    let lines: Vec<_> = o.errors().iter().map(|e| e.at.line).collect();
    assert_eq!(lines, [2], "{:?}", messages(&o));
    let base = theme_look(true);
    let look = o.applied(base.clone());
    assert_eq!(look.kit.c.scrim, base.kit.c.scrim);
    assert_eq!(look.row.list, 30.0);
}

/// Should-fix 4: the salvage loop and the walk stay near linear. The bound is generous for a
/// debug build; quadratic bookkeeping took over 15 s here.
#[test]
fn a_large_broken_file_parses_quickly() {
    // Keys set again, unknown tokens and lines of garbage: every line but the first is an
    // error.
    let src: String = (0..20_000)
        .map(|i| match i % 3 {
            0 => "row.list = 30\n".to_owned(),
            1 => format!("x{i} = 1\n"),
            _ => format!("garbage line {i}\n"),
        })
        .collect();
    let t = std::time::Instant::now();
    let o = Overrides::parse(&src);
    let took = t.elapsed();
    assert_eq!(o.len(), 1);
    assert_eq!(o.errors().len(), 19_999);
    assert!(took.as_secs_f64() < 5.0, "{took:?}");
}

/// Ruling B1: one row per token wins (the scoped one for this appearance), whichever of the
/// two is a reference, wherever each sits in the file; a reference reads only winners.
#[test]
fn a_scoped_row_beats_an_unscoped_one_whatever_their_kinds() {
    let accent = |look: &Look| look.kit.c.bg_accent;
    for (src, scoped_is_ref) in [
        (
            "color.scrim = \"@color.bg_accent\"\n[dark]\ncolor.scrim = \"#123456\"\n",
            false,
        ),
        (
            "dark.color.scrim = \"#123456\"\ncolor.scrim = \"@color.bg_accent\"\n",
            false,
        ),
        (
            "color.scrim = \"#123456\"\n[dark]\ncolor.scrim = \"@color.bg_accent\"\n",
            true,
        ),
        (
            "dark.color.scrim = \"@color.bg_accent\"\ncolor.scrim = \"#123456\"\n",
            true,
        ),
    ] {
        let src = format!("fill.muted = \"@color.scrim\"\n{src}");
        let o = Overrides::parse(&src);
        assert_eq!(messages(&o), Vec::<String>::new(), "{src}");
        for dark in [false, true] {
            let base = theme_look(dark);
            let look = o.applied(base.clone());
            let wins_ref = scoped_is_ref == dark;
            let want = if wins_ref {
                accent(&base)
            } else {
                color("#123456")
            };
            assert_eq!(look.kit.c.scrim, want, "{src} dark={dark}");
            // A reference to it reads the winning literal, or the theme's value when the
            // winner is itself a reference: never the losing row's.
            let read = if wins_ref {
                base.kit.c.scrim
            } else {
                color("#123456")
            };
            assert_eq!(look.fill.muted, read, "{src} dark={dark}");
        }
    }
}

#[test]
fn a_quoted_key_with_dots_is_one_key_not_a_token() {
    let o = Overrides::parse("\"list.row.height\" = 30\n");
    assert!(o.is_empty());
    assert_eq!(
        messages(&o),
        ["line 1, column 1: unknown token `\"list.row.height\"`"]
    );
}

#[test]
fn alpha_of_minus_zero_is_zero() {
    let o = Overrides::parse("color.scrim = \"alpha(red, -0)\"");
    match o.rows[0].value {
        Setting::Derived { alpha: Some(a), .. } => assert!(a == 0.0 && a.is_sign_positive()),
        other => panic!("{other:?}"),
    }
}

/// Done when: "The token names in error messages match the registry."
#[test]
fn token_names_in_messages_match_the_registry() {
    for token in tokens::all() {
        let name = token.name();
        // An array is the wrong kind for every token.
        let o = Overrides::parse(&format!("{name} = [1]"));
        let m = messages(&o);
        assert_eq!(m.len(), 1, "{name}: {m:?}");
        assert!(m[0].contains(&format!("`{name}` takes ")), "{m:?}");
    }
}

/// settings/themes: "Without the file, Strata's appearance is unchanged." and "Deleting the
/// file, or making it unreadable, restores Strata's default appearance." The host hands in
/// empty bytes for a missing or unreadable file.
#[test]
fn an_empty_or_missing_file_leaves_the_look_as_the_theme_made_it() {
    for src in ["", "# only a comment\n\n", "[dark]\n[light]\n"] {
        let o = Overrides::from_bytes(src.as_bytes());
        assert!(o.is_empty() && o.errors().is_empty(), "{src:?}");
        assert_eq!(o, Overrides::default(), "{src:?}");
        for dark in [false, true] {
            let base = theme_look(dark);
            assert_eq!(changed(&o.applied(base.clone()), &base), Vec::<&str>::new());
        }
    }
}

/// settings/themes: "With `~/.config/strata/style.css` present at startup, its rules … override
/// Strata's own rules and the active theme." and "Corner roundness does not scale radii set in
/// the user stylesheet."
#[test]
fn an_override_is_final_over_the_theme_and_the_settings() {
    let o = Overrides::parse("radius.md = 4\nrow.list = 50\nglow.on = false\n");
    for roundness in [0, 60, 200] {
        let params = LookParams {
            roundness,
            text_size: 48,
            content_density: 16,
            element_glow: true,
            ..LookParams::default()
        };
        let base = themed(true, &params);
        let look = o.applied(base.clone());
        assert_eq!(look.kit.radius.md, 4.0, "{roundness}");
        assert_eq!(look.row.list, 50.0);
        // The Element glow setting is on; the override wins (ruling Q6).
        assert!(base.glow.on && !look.glow.on);
        // Only what the file names moves.
        let mut moved = changed(&look, &base);
        moved.sort_unstable();
        // C4's heading strip follows `row.list` (slots inherit).
        let mut expected = vec![
            "glow.on",
            "list.heading.height",
            "list.row.height",
            "radius.md",
            "row.list",
        ];
        if base.kit.radius.md != 4.0 {
            // The collection rows' radius follows `radius.md` (slots inherit).
            expected.insert(0, "collection.row.radius");
        } else {
            expected.retain(|n| *n != "radius.md");
        }
        assert_eq!(moved, expected, "{roundness}");
    }
}

#[test]
fn light_and_dark_are_scopes_and_no_token_starts_with_them() {
    for token in tokens::all() {
        let first = token.name().split('.').next().unwrap_or_default();
        assert!(!matches!(first, "light" | "dark"), "{}", token.name());
    }
    let o = Overrides::parse("dark = 3\n[light.dark]\nrow.list = 30\n");
    let problems: Vec<_> = o.errors().iter().map(|e| e.problem.clone()).collect();
    assert_eq!(
        problems,
        [Problem::Scope("dark".into()), Problem::Scope("dark".into())]
    );
    assert_eq!(o.errors()[1].scope, Scope::Light);
    assert!(o.is_empty());
}

#[test]
fn a_file_that_is_not_utf8_applies_nothing() {
    let o = Overrides::from_bytes(b"row.list = 30\nrow.columns = \xff\n");
    assert!(o.is_empty());
    assert_eq!(
        o.errors(),
        &[OverrideError {
            at: at(2, 15),
            scope: Scope::All,
            problem: Problem::NotUtf8,
        }]
    );
}

#[test]
fn a_rewrite_that_changes_nothing_parses_equal() {
    let a = "row.list = 30\n[dark]\ncolor.bg_base = \"#111\"\n";
    assert_eq!(Overrides::parse(a), Overrides::parse(a));
    assert_ne!(
        Overrides::parse(a),
        Overrides::parse(&a.replace("30", "31"))
    );
}

/// Ruling Q2. settings/themes: "Rules in the file may use the active theme's `@strata_*`
/// colors; switching themes keeps the override and updates those colors."
#[test]
fn a_colour_taken_from_a_token_follows_the_theme() {
    let o = Overrides::parse(
        "sidebar.hover.fill = \"alpha(@color.text_base, 0.5)\"\ncolor.scrim = \"@color.bg_accent\"\n",
    );
    assert_eq!(messages(&o), Vec::<String>::new());
    let mut seen = Vec::new();
    for dark in [false, true] {
        let base = theme_look(dark);
        let look = o.applied(base.clone());
        let text = base.kit.c.text_base;
        assert_eq!(
            slots::sidebar::hover_fill(&look),
            base.kit.alpha(text, text.a * 0.5)
        );
        assert_eq!(look.kit.c.scrim, base.kit.c.bg_accent);
        seen.push(look.kit.c.scrim);
    }
    assert_ne!(seen[0], seen[1], "each theme's own accent");
}

#[test]
fn references_read_the_literal_overrides_and_never_chain() {
    let o = Overrides::parse(
        "color.text_accent = \"@color.bg_accent\"\ncolor.bg_accent = \"@color.border_focus\"\ncolor.border_focus = \"#ff9e64\"\n",
    );
    let base = theme_look(true);
    let look = o.applied(base.clone());
    // A reference sees the literal override of its target...
    assert_eq!(look.kit.c.bg_accent, color("#ff9e64"));
    // ...but not another reference's result.
    assert_eq!(look.kit.c.text_accent, base.kit.c.bg_accent);
}

#[test]
fn alpha_scales_the_colours_own_opacity() {
    let o = Overrides::parse("fill.muted = \"alpha(rgba(255, 0, 0, 0.5), 0.5)\"");
    assert_eq!(messages(&o), Vec::<String>::new());
    let base = theme_look(false);
    let look = o.applied(base.clone());
    let red = color("rgba(255, 0, 0, 0.5)");
    assert_eq!(look.fill.muted, base.kit.alpha(red, red.a * 0.5));
    assert!((red.a * 0.5 - 0.25).abs() < 0.01);
}

#[test]
fn a_bad_reference_is_reported_and_keeps_the_themes_value() {
    let reference = |target: &str| Problem::Reference {
        name: "fill.muted",
        target: target.into(),
    };
    let bad = |value: &str| Problem::BadColor {
        name: "fill.muted",
        value: value.into(),
    };
    for (value, problem) in [
        ("@color.bg_basee", reference("color.bg_basee")),
        ("@row.list", reference("row.list")),
        ("alpha(@nope, 0.5)", reference("nope")),
        ("alpha(@color.text_base)", bad("alpha(@color.text_base)")),
        ("alpha(red, 2)", bad("alpha(red, 2)")),
        (
            "alpha(alpha(red, 0.5), 0.5)",
            bad("alpha(alpha(red, 0.5), 0.5)"),
        ),
    ] {
        let o = Overrides::parse(&format!("fill.muted = \"{value}\""));
        let problems: Vec<_> = o.errors().iter().map(|e| e.problem.clone()).collect();
        assert_eq!(problems, [problem], "{value}");
        let base = theme_look(true);
        assert_eq!(o.applied(base.clone()).fill.muted, base.fill.muted);
    }
    assert_eq!(
        messages(&Overrides::parse("fill.muted = \"@row.list\"")),
        ["line 1, column 14: `fill.muted` refers to `@row.list`, which isn't a colour token"]
    );
}
