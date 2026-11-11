use super::*;
use crate::theme::color::Rgba8;
use crate::theme::colors::{DEFAULT_DANGER, ErrorKind, Location};

const OCEAN: &str = r##"name = "Ocean Blue"
background = "#0c1a2b"
surface = "#122438"
text = "#c9deed"
accent = "#4fd6ff"
danger = "#ff6b7a"
muted = "#1e3a52"
highlight = "#244d68"
border = "#315b75"
dim_text = "#6f8da3"
"##;

fn hex(s: &str) -> Rgba8 {
    Rgba8::parse(s).expect("parses")
}

fn with_line(src: &str, key: &str, value: &str) -> String {
    src.lines()
        .map(|l| {
            if l.starts_with(&format!("{key} =")) {
                format!("{key} = {value}")
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_custom_file_loads_as_written() {
    let t = parse_custom("ocean-blue", OCEAN).expect("loads");
    assert_eq!(t.id, "ocean-blue");
    assert_eq!(t.name, "Ocean Blue");
    assert_eq!(t.source, Source::Custom);
    assert_eq!(t.colors.accent, hex("#4fd6ff"));
    assert_eq!(t.colors.syntax.keyword, None);
}

/// settings/themes: "A custom theme file keeps its dim text as written, even below those
/// floors."
#[test]
fn a_custom_file_keeps_dim_text_below_the_floors() {
    let src = with_line(OCEAN, "dim_text", "\"#1a2a3a\"");
    let t = parse_custom("dim", &src).expect("loads");
    assert_eq!(t.colors.dim_text, hex("#1a2a3a"));
}

/// The WP's test: "A custom theme with a bad value reports its line."
#[test]
fn a_bad_value_reports_its_line_and_key() {
    let src = with_line(OCEAN, "accent", "\"#zz\"");
    let e = parse_custom("bad", &src).expect_err("rejects");
    assert_eq!(
        e.at,
        Some(Location {
            line: 5,
            column: 10
        })
    );
    assert_eq!(
        e.kind,
        ErrorKind::BadColor {
            key: "accent",
            value: "#zz".into()
        }
    );
    assert_eq!(
        e.to_string(),
        "line 5, column 10: `accent` is not a colour: \"#zz\""
    );

    let src = with_line(OCEAN, "border", "3");
    let e = parse_custom("bad", &src).expect_err("rejects");
    assert_eq!(e.at.map(|a| a.line), Some(9));
    assert_eq!(e.kind, ErrorKind::NotAString("border"));
}

#[test]
fn broken_toml_reports_its_line() {
    let src = with_line(OCEAN, "surface", "\"#122438");
    let e = parse_custom("broken", &src).expect_err("rejects");
    assert!(matches!(e.kind, ErrorKind::Syntax(_)), "{e}");
    assert_eq!(e.at.map(|a| a.line), Some(3), "{e}");
}

/// settings/themes: "Saving with an empty or whitespace-only name shows 'Enter a theme name'":
/// such a file doesn't load either.
#[test]
fn a_missing_key_or_an_empty_name_is_reported() {
    let src: String = OCEAN
        .lines()
        .filter(|l| !l.starts_with("muted"))
        .collect::<Vec<_>>()
        .join("\n");
    let e = parse_custom("x", &src).expect_err("rejects");
    assert_eq!((e.at, e.kind), (None, ErrorKind::Missing("muted")));

    let src = with_line(OCEAN, "name", "\"  \"");
    let e = parse_custom("x", &src).expect_err("rejects");
    assert_eq!(e.kind, ErrorKind::EmptyName);
    assert_eq!(e.at.map(|a| a.line), Some(1));
}

#[test]
fn danger_and_the_syntax_keys_are_optional_and_unknown_keys_are_ignored() {
    let src: String = OCEAN
        .lines()
        .filter(|l| !l.starts_with("danger"))
        .chain(["syntax_keyword = \"#ff7b72\"", "glow = \"#ffffff\""])
        .collect::<Vec<_>>()
        .join("\n");
    let t = parse_custom("x", &src).expect("loads");
    assert_eq!(t.colors.danger, DEFAULT_DANGER);
    assert_eq!(t.colors.syntax.keyword, Some(hex("#ff7b72")));
    assert_eq!(t.colors.syntax.string, None);
}

/// settings/themes: "A custom file with a GTK-only color form, such as 9-digit hex, loads with
/// colors as `#rrggbb`".
#[test]
fn gtk_only_forms_load_canonical_in_memory() {
    let src = with_line(OCEAN, "accent", "\"#7a7a2af7f\"");
    let t = parse_custom("x", &src).expect("loads");
    assert_eq!(t.colors.accent.hex(), "#7aa2f7");
}

/// settings/themes: "A custom file whose stem matches a bundled theme id replaces that bundled
/// theme", and custom themes "appear after the bundled themes".
#[test]
fn a_custom_theme_replaces_the_bundled_one_with_its_id() {
    let dracula = parse_custom("dracula", OCEAN).expect("loads");
    let extra = parse_custom("ocean", &with_line(OCEAN, "name", "\"Abyss\"")).expect("loads");
    let lib = Library::new(vec![dracula, extra]);
    assert_eq!(lib.themes().len(), 96);
    let found = lib.get("dracula").expect("listed");
    assert_eq!(
        (found.source, found.name.as_str()),
        (Source::Custom, "Ocean Blue")
    );
    assert_eq!(lib.themes().iter().filter(|t| t.id == "dracula").count(), 1);
    let tail: Vec<&str> = lib.themes()[94..].iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        tail,
        ["Abyss", "Ocean Blue"],
        "custom themes come last, by name"
    );
}

/// settings/themes: "A fresh install without Omarchy starts with Tokyo Night", and "A saved
/// theme id that matches no bundled or custom theme falls back to Azure Glow at startup."
#[test]
fn a_selection_resolves_to_the_default_or_the_fallback() {
    let lib = Library::default();
    assert_eq!(lib.resolve(None).id, "tokyo-night");
    assert_eq!(lib.resolve(Some("no-such-theme")).id, "azure-glow");
    assert_eq!(lib.resolve(Some("nord")).id, "nord");
}
