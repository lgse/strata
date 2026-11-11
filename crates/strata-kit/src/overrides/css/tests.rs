use super::*;
use crate::overrides::Overrides;
use crate::shape::LookParams;
use crate::slots;
use crate::theme::bundled_theme;
use crate::tokens::Kind;

/// A user's stylesheet with rules that convert and rules that don't.
const STYLE: &str = "/* My Strata tweaks */
.file-list row {
  margin: 1px 8px;
  border-radius: 3px;
}

.sidebar-row:hover,
.sidebar-device:hover {
  background: #ff000020;
}

.sidebar-row.active { background-color: rgba(0, 128, 255, 0.3); color: @strata_text; }
.sidebar-row.active image { color: hsl(30, 100%, 50%); }

.sidebar-shell {
  border-radius: 14px;
  background: alpha(@strata_surface, 0.6);
}

.sidebar-row.active image { color: lighter(red); }
.sidebar-row.active { background: alpha(@strata_accent, 0.25); }

@define-color strata_accent #ff0000;

@media (min-width: 600px) { .file-list row { margin: 0; } }
";

fn listed(m: &Migration) -> Vec<String> {
    m.unconverted.iter().map(ToString::to_string).collect()
}

fn theme_look(id: &str, params: &LookParams) -> crate::look::Look {
    bundled_theme(id)
        .expect("bundled")
        .look()
        .with_params(params)
}

/// The WP's test: a CSS migration fixture with converted and unconverted rules.
#[test]
fn a_stylesheet_converts_what_a_token_stands_for_and_lists_the_rest() {
    let m = migrate_css(STYLE);
    assert_eq!(
        m.toml,
        "# Converted from a user stylesheet (style.css): each row names the line it came from.

[sidebar.selected]
fill = \"alpha(@color.bg_accent, 0.25)\" # style.css line 21
"
    );
    assert_eq!(m.converted, 1);
    assert_eq!(
        listed(&m),
        [
            "line 3, column 3: `.file-list row` { margin: 1px 8px }: its token also styles elements this selector doesn't",
            "line 4, column 3: `.file-list row` { border-radius: 3px }: its token also styles elements this selector doesn't",
            "line 8, column 1: `.sidebar-device:hover`: no token stands for this selector",
            "line 9, column 3: `.sidebar-row:hover` { background: #ff000020 }: its token also styles elements this selector doesn't",
            "line 12, column 65: `.sidebar-row.active` { color: @strata_text }: no token stands for this property here",
            "line 13, column 29: `.sidebar-row.active image` { color: hsl(30, 100%, 50%) }: a later declaration for the same token didn't convert, and the later one wins",
            "line 15, column 1: `.sidebar-shell`: no token stands for this selector",
            "line 20, column 29: `.sidebar-row.active image` { color: lighter(red) }: the value isn't a plain colour",
            "line 23, column 1: `@define-color strata_accent #ff0000`: it redefines a theme colour: make a custom theme instead",
            "line 25, column 1: `@media (min-width: 600px)`: at-rules don't convert",
        ]
    );

    // What it wrote is an override file that applies with no errors.
    let o = Overrides::parse(&m.toml);
    assert_eq!(o.errors(), &[]);
    let base = theme_look("tokyo-night", &LookParams::default());
    let look = o.applied(base.clone());
    let accent = base.kit.c.bg_accent;
    assert_eq!(
        slots::sidebar::selected_fill(&look),
        base.kit.alpha(accent, accent.a * 0.25)
    );
    assert_eq!(
        slots::sidebar::selected_icon(&look),
        slots::sidebar::selected_icon(&base)
    );
}

#[test]
fn a_later_declaration_for_the_same_token_wins() {
    let m = migrate_css(
        ".sidebar-row.active { background: red; }\n.sidebar-row.active { background: #00ff00; }\n",
    );
    assert_eq!(m.converted, 1);
    assert_eq!(m.unconverted, []);
    assert!(
        m.toml.contains("fill = \"#00ff00\" # style.css line 2\n"),
        "{}",
        m.toml
    );
}

#[test]
fn values_that_arent_plain_colours_dont_convert() {
    for value in [
        "shade(@strata_bg, 0.9)",
        "mix(red, blue, 0.5)",
        "none",
        "red !important",
        "alpha(@strata_nope, 0.5)",
        "alpha(red, 2)",
        "url(\"x.png\")",
    ] {
        let m = migrate_css(&format!(".sidebar-row.active {{ background: {value}; }}"));
        assert_eq!(m.converted, 0, "{value}");
        assert_eq!(m.unconverted[0].reason, Reason::Value, "{value}");
    }
}

#[test]
fn broken_css_is_listed_and_the_rest_converts() {
    let m = migrate_css(
        "}\n.sidebar-row.active { background red; }\n.sidebar-row.active image { color: #123456; }\n.sidebar-row.active { background: red;",
    );
    assert_eq!(m.converted, 1, "{:?}", listed(&m));
    let reasons: Vec<_> = m
        .unconverted
        .iter()
        .map(|u| (u.at.line, u.reason))
        .collect();
    assert_eq!(
        reasons,
        [
            (1, Reason::Syntax),
            (2, Reason::Syntax),
            (4, Reason::Syntax)
        ]
    );
}

#[test]
fn a_stylesheet_with_nothing_to_convert_writes_no_file() {
    let m = migrate_css("label { color: red; }\n/* .sidebar-row.active { background: red; } */\n");
    assert_eq!(m.toml, "");
    assert_eq!(m.converted, 0);
    assert_eq!(m.unconverted.len(), 1);
}

#[test]
fn every_rule_names_a_registry_token_of_its_kind() {
    for &(selector, prop, name) in RULES.iter().chain(WIDER) {
        let token = tokens::find(name).unwrap_or_else(|| panic!("{name} is a token"));
        let kind = match prop {
            Prop::Background | Prop::Color => Kind::Color,
            Prop::Radius | Prop::Margin => Kind::Length,
        };
        assert_eq!(token.kind(), kind, "{selector} → {name}");
    }
}

/// Strata's own rules for these selectors (`src/style.css` at lgse/strata 35ee677ab, lines
/// 5061–5079 and 2373–2389). The two that convert set their slots to the slots' defaults, so
/// those rows are one-to-one; the rest are listed.
const STRATA_RULES: &str = ".file-list row,
.file-list-mode row,
.preview-archive-list row {
  background: transparent;
  color: @strata_text;
  border-radius: 6px;
  transition: color 160ms ease-out;
}

.file-list row,
.file-list-mode row {
  margin: 1px 8px;
  padding: 0;
}

.file-list row {
  margin-left: 5px;
  margin-right: 5px;
}

.sidebar-row:hover,
.sidebar-device:hover {
  background: alpha(@strata_text, 0.06);
}

.sidebar-row.active,
.sidebar-row.active:hover,
.sidebar-device.active,
.sidebar-device.active:hover {
  background: alpha(@strata_accent, 0.18);
  color: @strata_text;
}

.sidebar-row.active image,
.sidebar-device.active .sidebar-row image {
  color: @strata_accent;
}
";

#[test]
fn stratas_own_rules_migrate_to_the_slots_defaults() {
    let m = migrate_css(STRATA_RULES);
    assert_eq!(
        m.toml,
        "# Converted from a user stylesheet (style.css): each row names the line it came from.

[sidebar.selected]
fill = \"alpha(@color.bg_accent, 0.18)\" # style.css line 30
icon = \"@color.bg_accent\" # style.css line 36
"
    );
    let wider: Vec<_> = m
        .unconverted
        .iter()
        .filter(|u| u.reason == Reason::Wider)
        .map(|u| u.at.line)
        .collect();
    assert_eq!(wider, [6, 12, 17, 18, 23]);
    let o = Overrides::parse(&m.toml);
    assert_eq!(o.errors(), &[]);
    for id in ["tokyo-night", "omarchy-light"] {
        let base = theme_look(id, &LookParams::default());
        let look = o.applied(base.clone());
        type Reader = fn(&crate::look::Look) -> rune::Color;
        let colours: [(&str, Reader); 2] = [
            ("selected fill", slots::sidebar::selected_fill),
            ("selected icon", slots::sidebar::selected_icon),
        ];
        for (what, read) in colours {
            assert_eq!(read(&look), read(&base), "{id}: {what}");
        }
    }
}

#[test]
fn every_strata_colour_maps_onto_a_colour_token() {
    for (strata, name) in STRATA_COLORS {
        let token = tokens::find(name).unwrap_or_else(|| panic!("{name} is a token"));
        assert_eq!(token.kind(), Kind::Color, "@{strata} → {name}");
    }
}

/// Ruling B4: a token that would style more than the selector did doesn't take it.
#[test]
fn rules_whose_token_reaches_further_than_the_selector_dont_convert() {
    let m = migrate_css(
        ".file-list row { margin: 0 4px; border-radius: 3px; }\n.sidebar-row:hover { background: red; }\n",
    );
    assert_eq!(m.toml, "");
    let reasons: Vec<_> = m
        .unconverted
        .iter()
        .map(|u| (u.at.line, u.reason))
        .collect();
    assert_eq!(
        reasons,
        [(1, Reason::Wider), (1, Reason::Wider), (2, Reason::Wider)]
    );
}

/// Should-fix 5: in CSS the later declaration wins, so when it doesn't convert, the earlier
/// one mustn't either; both are listed.
#[test]
fn a_later_declaration_that_doesnt_convert_drops_the_earlier_row() {
    for css in [
        ".sidebar-row.active { background: red; }\n.sidebar-row.active { background: shade(red, 1.1); }\n",
        ".sidebar-row.active { background: red;\n  background: 1em; }\n",
    ] {
        let m = migrate_css(css);
        assert_eq!(m.toml, "", "{css}");
        assert_eq!(m.converted, 0, "{css}");
        let reasons: Vec<_> = m
            .unconverted
            .iter()
            .map(|u| (u.at.line, u.reason))
            .collect();
        assert_eq!(
            reasons,
            [(1, Reason::Superseded), (2, Reason::Value)],
            "{css}"
        );
    }
}

/// Should-fix 4: locations are found incrementally. Strata's own 7,639-line stylesheet took
/// 0.71 s; this is a generous bound for a debug build.
#[test]
fn a_large_stylesheet_migrates_quickly() {
    let css: String = (0..20_000)
        .map(|i| {
            format!(
                ".c{i} {{ color: red; }}\n.sidebar-row.active {{ background: #{:06x}; }}\n",
                i
            )
        })
        .collect();
    let t = std::time::Instant::now();
    let m = migrate_css(&css);
    let took = t.elapsed();
    assert_eq!(m.converted, 1);
    assert_eq!(m.unconverted.len(), 20_000);
    assert!(took.as_secs_f64() < 5.0, "{took:?}");
}
