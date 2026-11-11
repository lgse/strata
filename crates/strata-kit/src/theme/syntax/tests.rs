use rune_kit::syntax::Syntax;

use crate::theme::color::Rgba8;
use crate::theme::colors::SourceColors;
use crate::theme::{bundled_theme, parse_custom};

const NO_SYNTAX: &str = r##"name = "Plain"
background = "#0c1a2b"
surface = "#122438"
text = "#c9deed"
accent = "#4fd6ff"
muted = "#1e3a52"
highlight = "#244d68"
border = "#315b75"
dim_text = "#6f8da3"
"##;

fn kinds(look: &crate::look::Look) -> [rune::Color; 5] {
    [
        look.syntax.color(Syntax::Keyword),
        look.syntax.color(Syntax::String),
        look.syntax.color(Syntax::Number),
        look.syntax.color(Syntax::Type),
        look.syntax_extra.preprocessor,
    ]
}

/// settings/themes: "In a custom file, each `syntax_*` key overrides one role; an omitted key
/// keeps its color derived from accent and text."
#[test]
fn an_omitted_syntax_colour_follows_the_accent_and_a_set_one_holds() {
    let plain = parse_custom("plain", NO_SYNTAX).expect("loads");
    assert_eq!(
        plain.look().syntax.color(Syntax::Keyword),
        plain.colors.accent.color(),
        "keywords take the accent itself"
    );

    let mut other_accent = plain.clone();
    other_accent.colors.accent = Rgba8::rgb(0xff, 0x7b, 0x72);
    let (a, b) = (kinds(&plain.look()), kinds(&other_accent.look()));
    assert!(
        a.iter().zip(&b).all(|(x, y)| x != y),
        "every derived colour follows the accent"
    );

    let src = format!("{NO_SYNTAX}syntax_string = \"#7ee787\"\n");
    let one = parse_custom("one", &src).expect("loads");
    let (base, set) = (kinds(&plain.look()), kinds(&one.look()));
    assert_eq!(set[1], Rgba8::rgb(0x7e, 0xe7, 0x87).color());
    for i in [0, 2, 3, 4] {
        assert_eq!(base[i], set[i], "only the string colour moves");
    }
}

/// settings/themes: "Comments use the dim text color"; functions and the rest draw as text,
/// as GtkSourceView's identifier style does.
#[test]
fn comments_take_dim_text_and_unstyled_kinds_take_text() {
    let t = bundled_theme("gruvbox-dark-hard").expect("bundled");
    let look = t.look();
    assert_eq!(
        look.syntax.color(Syntax::Comment),
        t.colors.dim_text.color()
    );
    for kind in [
        Syntax::Function,
        Syntax::Property,
        Syntax::Parameter,
        Syntax::Punctuation,
        Syntax::Variable,
    ] {
        assert_eq!(look.syntax.color(kind), t.colors.text.color(), "{kind:?}");
    }
    assert_eq!(look.syntax.color(Syntax::Deleted), t.colors.danger.color());
}

#[test]
fn a_theme_with_every_syntax_colour_uses_them_as_written() {
    let t = bundled_theme("catppuccin-mocha").expect("bundled");
    let SourceColors {
        keyword,
        string,
        constant,
        type_,
        preprocessor,
    } = t.colors.syntax;
    let written = [keyword, string, constant, type_, preprocessor].map(|c| c.expect("set").color());
    assert_eq!(kinds(&t.look()), written);
}

/// A user's `syntax.*` override, applied after the theme, reaches the scope roles; the
/// syntect theme built from them (K1b) then colours code with it.
#[test]
fn scope_roles_follow_the_finished_look() {
    use crate::theme::syntax::{Role, SCOPES};
    use crate::tokens::{self, Value};

    let mut look = bundled_theme("nord").expect("bundled").look();
    let pink = Rgba8::rgb(0xff, 0x00, 0x80).color();
    for (name, role) in [
        ("syntax.keyword", Role::Keyword),
        ("syntax.comment", Role::Comment),
        ("syntax.preprocessor", Role::Preprocessor),
        ("color.text_accent", Role::Heading),
        ("color.danger.fg", Role::Error),
    ] {
        assert_ne!(role.color(&look), pink);
        tokens::set(&mut look, name, Value::Color(pink)).expect("a token");
        assert_eq!(role.color(&look), pink, "{name}");
    }
    // Every role but plain text has a scope, and no selector is listed twice.
    for role in Role::ALL.into_iter().filter(|&r| r != Role::Text) {
        assert!(SCOPES.iter().any(|&(_, r)| r == role), "{role:?}");
    }
    let mut selectors: Vec<&str> = SCOPES.iter().map(|&(s, _)| s).collect();
    selectors.sort_unstable();
    selectors.dedup();
    assert_eq!(selectors.len(), SCOPES.len());
}

mod syntect_theme {
    use syntect::highlighting::{Color, Highlighter};
    use syntect::parsing::Scope;

    use crate::look::Look;
    use crate::theme::bundled_theme;
    use crate::theme::syntax::{Role, SCOPES, syntect_theme, to_syntect};
    use crate::tokens::{self, Value};

    /// The foreground `look`'s syntect theme gives a token under `stack`.
    fn fg(look: &Look, stack: &[&str]) -> Color {
        let theme = syntect_theme(look);
        let stack: Vec<Scope> = stack
            .iter()
            .map(|s| Scope::new(s).expect("a scope"))
            .collect();
        Highlighter::new(&theme).style_for_stack(&stack).foreground
    }

    fn role(look: &Look, r: Role) -> Color {
        to_syntect(r.color(look))
    }

    /// The most specific selector wins, along the scope and down the stack, as syntect
    /// matches; a scope no selector names keeps the default foreground, the look's text, so
    /// C10a gives it no span.
    #[test]
    fn the_most_specific_selector_wins_and_unlisted_scopes_draw_as_text() {
        let look = bundled_theme("catppuccin-mocha").expect("bundled").look();
        let roles = [
            Role::Text,
            Role::String,
            Role::Constant,
            Role::Keyword,
            Role::Type,
            Role::Preprocessor,
            Role::Inserted,
        ];
        for (i, a) in roles.iter().enumerate() {
            for b in &roles[i + 1..] {
                assert_ne!(role(&look, *a), role(&look, *b), "{a:?} and {b:?} differ");
            }
        }
        let cases = [
            (&["source.rust", "storage.type.rust"][..], Role::Type),
            (
                &["source.rust", "storage.type.function.rust"],
                Role::Keyword,
            ),
            (
                &[
                    "source.python",
                    "string.quoted.single.python",
                    "storage.type.string.python",
                ],
                Role::String,
            ),
            (
                &[
                    "source.c",
                    "string.quoted.double.c",
                    "constant.character.escape.c",
                ],
                Role::Constant,
            ),
            (
                &[
                    "source.c",
                    "meta.preprocessor.include.c",
                    "keyword.control.import.include.c",
                ],
                Role::Preprocessor,
            ),
            (&["source.diff", "markup.inserted.diff"], Role::Inserted),
            (&["source.rust", "entity.name.function.rust"], Role::Text),
            // css.lang: a class selector is an identifier, an id selector a statement, and
            // an attribute selector's name a type; html.lang's attribute names stay types.
            (
                &[
                    "source.css",
                    "meta.selector.css",
                    "entity.other.attribute-name.class.css",
                    "punctuation.definition.entity.css",
                ],
                Role::Text,
            ),
            (
                &[
                    "source.css",
                    "meta.selector.css",
                    "entity.other.attribute-name.class.css",
                ],
                Role::Text,
            ),
            (
                &[
                    "source.css",
                    "meta.selector.css",
                    "entity.other.attribute-name.id.css",
                ],
                Role::Keyword,
            ),
            (
                &[
                    "source.css",
                    "meta.selector.css",
                    "meta.attribute-selector.css",
                    "entity.other.attribute-name.css",
                ],
                Role::Type,
            ),
            (
                &[
                    "text.html.basic",
                    "meta.tag.inline.a.html",
                    "meta.attribute-with-value.class.html",
                    "entity.other.attribute-name.class.html",
                ],
                Role::Type,
            ),
            // go.lang lists `func`, `type`, `struct`, `var`, `const` and `interface` as
            // keywords, and `map` and `chan` as types.
            (
                &["source.go", "storage.type.keyword.function.go"],
                Role::Keyword,
            ),
            (
                &["source.go", "meta.type.go", "storage.type.keyword.map.go"],
                Role::Type,
            ),
            (
                &["source.go", "meta.type.go", "storage.type.keyword.chan.go"],
                Role::Type,
            ),
        ];
        // Every mismatch at once, each with the role it drew in.
        let wrong: Vec<String> = cases
            .iter()
            .filter_map(|&(stack, want)| {
                let got = fg(&look, stack);
                (got != role(&look, want)).then(|| {
                    let drew = roles.iter().find(|&&r| role(&look, r) == got);
                    format!("{stack:?}: want {want:?}, drew {drew:?}")
                })
            })
            .collect();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// Every [`SCOPES`] row parses into a theme item: `syntect_theme` skips a selector that
    /// doesn't parse rather than failing.
    #[test]
    fn every_scope_row_becomes_a_theme_item() {
        let look = bundled_theme("nord").expect("bundled").look();
        assert_eq!(syntect_theme(&look).scopes.len(), SCOPES.len());
    }

    /// A user's `syntax.*` override, applied after the theme, reaches the syntect theme.
    #[test]
    fn an_override_reaches_the_syntect_theme() {
        let mut look = bundled_theme("nord").expect("bundled").look();
        let keyword = ["source.rust", "keyword.control.rust"];
        let pink = rune::color::hex("#ff0080").expect("a colour");
        assert_ne!(fg(&look, &keyword), to_syntect(pink));
        tokens::set(&mut look, "syntax.keyword", Value::Color(pink)).expect("a token");
        assert_eq!(fg(&look, &keyword), to_syntect(pink));
    }
}
