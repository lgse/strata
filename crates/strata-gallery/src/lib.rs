// SPDX-License-Identifier: MIT

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

pub type Scene = (&'static str, fn(&mut Gallery));

/// A page's view.
pub type View = fn(&Gallery, &mut Cx<Gallery>) -> El;

pub type KeyHook = fn(&mut Gallery, &KeyEvent, &mut Tx) -> bool;

pub type PollHook = fn(&mut Gallery, f64, &mut Tx) -> bool;

#[derive(Default)]
pub struct Gallery {
    pub dark: bool,
    pub look: Look,
    pub theme: Option<scenes::k1::Shown>,
    /// The open page's id, or `None` for the index.
    pub page: Option<&'static str>,
    pub waker: Option<Waker>,
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
    pub fn open(&mut self, page: &'static str, dark: bool) {
        self.page = Some(page);
        self.dark = dark;
        self.look = Look::fallback(dark);
        self.theme = None;
    }

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

pub fn route_key(page_key: KeyHook, g: &mut Gallery, key: &KeyEvent, tx: &mut Tx) -> bool {
    if page_key(g, key, tx) {
        return true;
    }
    if key.key == Key::Escape && g.page.is_some() {
        g.page = None;
        g.look = g.theme_look(&LookParams::default());
        g.dark = g.look.kit.dark;
        return true;
    }
    false
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SyntaxKey(SyntaxTheme, SyntaxExtra);

impl SyntaxKey {
    pub fn of(look: &Look) -> SyntaxKey {
        SyntaxKey(look.syntax, look.syntax_extra)
    }
}
