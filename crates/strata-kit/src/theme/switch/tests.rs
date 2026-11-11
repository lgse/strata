use rune::{Cx, El, Env, Runtime, el};

use super::switch;
use crate::look::Look;
use crate::theme::bundled_theme;

const FRAME: f64 = 1.0 / 60.0;

fn view(look: &Look, _: &mut Cx<Look>) -> El {
    el().key("card")
        .size(40.0, 40.0)
        .shadow(0.0, 8.0, look.kit.c.bg_accent)
}

fn shown(rt: &Runtime<Look>) -> rune::Color {
    let pose = rt.presented_pose("card").expect("the card");
    pose.shadows().first().expect("its shadow").color
}

/// The accent one frame after switching from Nord to Gruvbox, with animations on or off.
fn one_frame_in(animations: bool) -> (rune::Color, rune::Color) {
    let from = bundled_theme("nord").expect("bundled").look();
    let mut to = bundled_theme("gruvbox-dark-hard").expect("bundled").look();
    to.animations = animations;
    let target = to.kit.c.bg_accent;
    let mut rt = Runtime::new(from, view, (100.0, 100.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt.frame(FRAME);
    rt.update(move |look, tx| switch(look, to, tx));
    rt.frame(FRAME);
    rt.frame(FRAME);
    (shown(&rt), target)
}

/// Strata's animations switch: with animations off a theme switch lands at once; with them on
/// the same frame is still mid-switch.
#[test]
fn with_animations_off_a_switch_lands_at_once() {
    let (on, target) = one_frame_in(true);
    assert_ne!(on, target, "animations on: still moving");
    let (off, target) = one_frame_in(false);
    assert_eq!(off, target, "animations off: landed");
}
