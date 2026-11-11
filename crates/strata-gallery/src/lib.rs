//! The gallery's state, pages and view, shared by the binary and the golden tests.
//!
//! The gallery has one page per component (and per contract or look work package that shows
//! something). Each page lives in `scenes/<id>.rs` with its scenes, its demo state and its
//! view; `docs/testing.md` describes the layout and the goldens built from it.

use rune::text::TextSystem;
use rune::{AppSpec, Cx, El, Env, Key, KeyEvent, Runtime, Tx, Waker, col};
use rune_kit::Kit;
use rune_kit::syntax::SyntaxTheme;
use strata_kit::fonts;
use strata_kit::look::{Look, SyntaxExtra};
use strata_kit::shape::LookParams;

pub mod scenes;

/// The window size every scene renders at, headless and in a window.
pub const SIZE: (f32, f32) = (960.0, 640.0);

/// A named state the gallery opens in with `--scene <name>`. Each golden scene also has an
/// accessibility and a PNG golden.
pub type Scene = (&'static str, fn(&mut Gallery));

/// A page's view.
pub type View = fn(&Gallery, &mut Cx<Gallery>) -> El;

/// A page's key hook, `AppSpec::on_key`'s shape: keys no focused element took. True consumes
/// the key.
pub type KeyHook = fn(&mut Gallery, &KeyEvent, &mut Tx) -> bool;

/// A page's poll hook, `AppSpec::poll`'s shape: runs every frame with the clock in seconds.
/// True when state changed.
pub type PollHook = fn(&mut Gallery, f64, &mut Tx) -> bool;

/// The gallery's state: the open page, light or dark, the look pages draw with, the app's
/// waker, and one demo state per page.
#[derive(Default)]
pub struct Gallery {
    pub dark: bool,
    /// The look every page draws with. [`Gallery::open`] rebuilds it as the fallback for
    /// `dark`; a look page (K1, K2, K4) then changes it from its own `scenes/<id>.rs`.
    pub look: Look,
    /// The theme picked on K1's list, which every page wears until another is picked; `None`
    /// is rune-kit's neutral kit, the fallback. [`Gallery::open`] clears it, so a scene
    /// starts on the fallback.
    pub theme: Option<scenes::k1::Shown>,
    /// The open page's id, or `None` for the index.
    pub page: Option<&'static str>,
    /// Wakes the app loop from another thread, so the open page's `poll` runs: a fake worker
    /// calls it after it hands over a result. Set by [`setup`]; a no-op in tests.
    pub waker: Option<Waker>,
    /// The index's state. `index.markers` (default true) marks each page built or not built;
    /// the index's golden scenes turn it off, so their goldens don't change as pages land.
    pub index: scenes::gallery::Demo,
    pub w1: scenes::w1::Demo,
    pub w2: scenes::w2::Demo,
    pub k1: scenes::k1::Demo,
    pub k2: scenes::k2::Demo,
    pub k3: scenes::k3::Demo,
    pub k4: scenes::k4::Demo,
    pub c1: scenes::c1::Demo,
    pub c2: scenes::c2::Demo,
    pub c3: scenes::c3::Demo,
    pub c4: scenes::c4::Demo,
    pub c5: scenes::c5::Demo,
    pub c6: scenes::c6::Demo,
    pub c7: scenes::c7::Demo,
    pub c8: scenes::c8::Demo,
    pub c9: scenes::c9::Demo,
    pub c10: scenes::c10::Demo,
    pub c10a: scenes::c10a::Demo,
    pub c11: scenes::c11::Demo,
    pub c11b: scenes::c11b::Demo,
    pub c12: scenes::c12::Demo,
    pub c13: scenes::c13::Demo,
    pub c14a: scenes::c14a::Demo,
    pub c14b: scenes::c14b::Demo,
    pub c15: scenes::c15::Demo,
    pub c16: scenes::c16::Demo,
    pub c17: scenes::c17::Demo,
    pub c18: scenes::c18::Demo,
    pub c19: scenes::c19::Demo,
    pub c20a: scenes::c20a::Demo,
    pub c20b: scenes::c20b::Demo,
    pub c20c: scenes::c20c::Demo,
    pub c20d: scenes::c20d::Demo,
    pub c21: scenes::c21::Demo,
    pub c22: scenes::c22::Demo,
    pub c23: scenes::c23::Demo,
    pub c24: scenes::c24::Demo,
    pub c25: scenes::c25::Demo,
    pub c26: scenes::c26::Demo,
    pub x1: scenes::x1::Demo,
}

impl Gallery {
    /// Opens the page `page` in light or dark, on the fallback look. Every scene except the
    /// index's calls this.
    pub fn open(&mut self, page: &'static str, dark: bool) {
        self.page = Some(page);
        self.dark = dark;
        self.look = Look::fallback(dark);
        self.theme = None;
    }

    /// The picked theme's look at `params`, or the fallback's for `dark` when none is
    /// picked: what a page that rebuilds the look (K2) starts from, and what the index goes
    /// back to.
    pub fn theme_look(&self, params: &LookParams) -> Look {
        match self.theme {
            Some(shown) => scenes::k1::theme_of(shown).look().with_params(params),
            None => Look::fallback(self.dark).with_params(params),
        }
    }

    /// The look pages draw with: the accessor components take, `fn(&Gallery) -> &Look`.
    pub fn look(&self) -> &Look {
        &self.look
    }

    /// The look's kit.
    pub fn kit(&self) -> &Kit {
        &self.look.kit
    }
}

/// The open page on the gallery's background.
pub fn view(g: &Gallery, cx: &mut Cx<Gallery>) -> El {
    let page = scenes::open_page(g);
    col()
        .key("gallery")
        .w_full()
        .h_full()
        .fill(g.kit().c.bg_base)
        .child((page.view)(g, cx))
}

/// The app: every scene, and the hooks that route keys and polls to the open page. The binary
/// runs it, so `--snap` and `--script` runs go through the same hooks as a window. Text draws
/// in the bundled JetBrains Mono, in both slots, as Strata's does (K6).
pub fn spec(scenes: &'static [Scene]) -> AppSpec<Gallery> {
    AppSpec {
        scenes,
        fonts: fonts::FONTS,
        families: fonts::FAMILIES,
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
        ..AppSpec::new("Strata components", SIZE, Gallery::default, view)
    }
}

/// A headless runtime on `g`, wired as [`spec`] wires the app: the open page's `poll` runs on
/// every `frame`, keys go through [`on_key`], the waker is a no-op, and text shapes in
/// [`spec`]'s fonts (`Runtime::new` would shape in Rune's built-in Inter), so accessibility
/// goldens and page tests lay out as `--snap` and `--script` do.
pub fn runtime(g: Gallery, env: Env) -> Runtime<Gallery> {
    let text = TextSystem::new(fonts::FONTS, fonts::FAMILIES);
    let mut rt = Runtime::with_text(g, view, SIZE, env, text);
    rt.poll = Some(poll);
    rt.on_key = Some(on_key);
    setup(&mut rt.state, std::sync::Arc::new(|| {}));
    rt
}

/// Keeps the app's waker for pages that deliver results from another thread.
pub fn setup(g: &mut Gallery, waker: Waker) {
    g.waker = Some(waker);
}

/// Runs the open page's poll.
pub fn poll(g: &mut Gallery, t: f64, tx: &mut Tx) -> bool {
    (scenes::open_page(g).poll)(g, t, tx)
}

/// Gives the key to the open page first; a key it leaves goes to [`route_key`]'s fallback.
pub fn on_key(g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    route_key(scenes::open_page(g).on_key, g, key, tx)
}

/// `page_key` takes the key first. If it leaves Escape and a page is open, Escape goes back
/// to the index. Public so tests can route through a page hook of their own.
pub fn route_key(page_key: KeyHook, g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    if page_key(g, key, tx) {
        return true;
    }
    if key.key == Key::Escape && g.page.is_some() {
        g.page = None;
        // A look page (K2, K4, X1) may leave its own look behind; the index wears the theme.
        g.look = g.theme_look(&LookParams::default());
        g.dark = g.look.kit.dark;
        return true;
    }
    false
}

/// What a page's cached syntect theme was built from. A page that keeps a highlighter
/// rebuilds its theme when this changes: a theme picked on K1, or light to dark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyntaxKey(SyntaxTheme, SyntaxExtra);

impl SyntaxKey {
    pub fn of(look: &Look) -> SyntaxKey {
        SyntaxKey(look.syntax, look.syntax_extra)
    }
}
