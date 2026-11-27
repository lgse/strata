// SPDX-License-Identifier: MIT

use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, col};
use strata_kit::look::{Look, LookParams};

use super::{Switch, radii};

struct Host {
    checked: bool,
    disabled: bool,
    toggles: usize,
    look: Look,
}

fn view(h: &Host, _: &mut Cx<Host>) -> El {
    col().child(
        Switch::new(h.checked)
            .label("Folder peeking")
            .description("Preview folders on hover.")
            .disabled(h.disabled)
            .on_toggle(|h: &mut Host| {
                h.checked = !h.checked;
                h.toggles += 1;
            })
            .build(&h.look)
            .key("sw"),
    )
}

fn runtime(checked: bool, disabled: bool) -> Runtime<Host> {
    let host = Host {
        checked,
        disabled,
        toggles: 0,
        look: Look::fallback(true),
    };
    let mut rt = Runtime::new(host, view, (200.0, 80.0), Env::new(true, 1.0));
    rt.frame(0.0);
    for _ in 0..3 {
        rt.frame(1.0 / 60.0);
    }
    rt
}

fn step(rt: &mut Runtime<Host>) {
    for _ in 0..3 {
        rt.frame(1.0 / 60.0);
    }
}

#[test]
fn radii_square_off_at_zero_and_stay_concentric_when_rounded() {
    let sharp = Look::fallback(true).with_params(&LookParams {
        roundness: 0,
        ..LookParams::default()
    });
    assert_eq!(radii(&sharp), (0.0, 0.0));

    let gentle = Look::fallback(true).with_params(&LookParams {
        roundness: 20,
        ..LookParams::default()
    });
    let (track, knob) = radii(&gentle);
    assert!(track > 0.0 && track < 12.0);
    assert!(knob > 0.0 && knob < 10.0);
    assert!(track > knob);

    let default = Look::fallback(true).with_params(&LookParams {
        roundness: 60,
        ..LookParams::default()
    });
    assert_eq!(radii(&default), (12.0, 10.0));

    let max = Look::fallback(true).with_params(&LookParams {
        roundness: 200,
        ..LookParams::default()
    });
    assert_eq!(radii(&max), (12.0, 10.0));
}

#[test]
fn pointer_and_space_toggle_switch_and_preserve_role() {
    let mut rt = runtime(false, false);
    let info = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("sw"))
        .expect("switch element");
    assert_eq!(info.role, "switch");
    assert!(info.focusable);
    assert!(!rt.state.checked);

    let [x, y, w, h] = info.rect;
    rt.pointer_down(x + w / 2.0, y + h / 2.0);
    rt.pointer_up(x + w / 2.0, y + h / 2.0);
    step(&mut rt);
    assert!(rt.state.checked);
    assert_eq!(rt.state.toggles, 1);

    rt.update(|_: &mut Host, tx| tx.focus("sw", true));
    step(&mut rt);
    rt.key(KeyEvent {
        key: Key::Text(" ".into()),
        mods: Mods::default(),
    });
    step(&mut rt);
    assert!(!rt.state.checked);
    assert_eq!(rt.state.toggles, 2);
}

#[test]
fn disabled_switch_ignores_pointer_and_is_not_focusable() {
    let mut rt = runtime(false, true);
    let info = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("sw"))
        .expect("switch element");
    assert_eq!(info.role, "switch");
    assert!(!info.focusable);

    let [x, y, w, h] = info.rect;
    rt.pointer_down(x + w / 2.0, y + h / 2.0);
    rt.pointer_up(x + w / 2.0, y + h / 2.0);
    step(&mut rt);
    assert!(!rt.state.checked);
    assert_eq!(rt.state.toggles, 0);
}
