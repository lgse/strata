// SPDX-License-Identifier: MIT

use rune::{Cx, El, Env, Key, KeyEvent, Mods, Runtime, Tx};
use strata_kit::look::Look;

use super::super::{PdfDocument, PdfIntent, PdfState, PdfView};

struct Host {
    pdf: PdfState,
    look: Look,
    intents: Vec<PdfIntent>,
}

fn view(h: &Host, cx: &mut Cx<Host>) -> El {
    PdfView::new(|h: &Host| &h.pdf, |h: &mut Host| &mut h.pdf)
        .on_intent(|h: &mut Host, intent, _| h.intents.push(intent))
        .build(h, &h.look, cx)
}

fn poll(h: &mut Host, t: f64, tx: &mut Tx) -> bool {
    h.pdf.tick(t) | h.pdf.flush(tx)
}

fn runtime(pages: usize) -> Runtime<Host> {
    let mut pdf = PdfState::new("pdf");
    pdf.open(PdfDocument::new(pages, [600.0, 800.0]));
    let host = Host {
        pdf,
        look: Look::fallback(false),
        intents: Vec::new(),
    };
    let mut rt = Runtime::new(host, view, (500.0, 700.0), Env::new(false, 1.0));
    rt.poll = Some(poll);
    settle(&mut rt);
    rt
}

fn settle(rt: &mut Runtime<Host>) {
    for _ in 0..8 {
        rt.frame(0.016);
    }
}

fn focused_key(rt: &Runtime<Host>) -> Option<String> {
    rt.elements()
        .into_iter()
        .find(|el| !el.exiting && el.focused)
        .and_then(|el| el.key)
}

fn is_disabled(rt: &Runtime<Host>, key: &str) -> bool {
    !element_by_key(rt, key)
        .unwrap_or_else(|| panic!("element `{key}` should be mounted"))
        .focusable
}

fn element_by_key(rt: &Runtime<Host>, key: &str) -> Option<rune::ElementInfo> {
    rt.elements()
        .into_iter()
        .find(|el| !el.exiting && el.key.as_deref() == Some(key))
}

fn center_of(rt: &Runtime<Host>, key: &str) -> (f32, f32) {
    let el = element_by_key(rt, key).unwrap_or_else(|| panic!("element `{key}` should be mounted"));
    let [x, y, w, h] = el.visible;
    (x + w / 2.0, y + h / 2.0)
}

fn click_key(rt: &mut Runtime<Host>, key: &str) {
    let (x, y) = center_of(rt, key);
    rt.pointer_move(x, y);
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    settle(rt);
}

fn press_key(rt: &mut Runtime<Host>, key: Key) {
    rt.key(KeyEvent {
        key,
        mods: Mods::default(),
    });
    settle(rt);
}

fn type_str(rt: &mut Runtime<Host>, text: &str) {
    for ch in text.chars() {
        rt.key(KeyEvent {
            key: Key::Text(ch.to_string()),
            mods: Mods::default(),
        });
    }
    settle(rt);
}

#[test]
fn single_page_omits_page_nav_and_multipage_mounts_it_with_initial_page_and_total() {
    let single = runtime(1);
    assert!(
        element_by_key(&single, "pdf-page-nav").is_none(),
        "single-page PDF must omit the page navigation pill"
    );
    assert!(
        element_by_key(&single, "pdf-page-input").is_none(),
        "single-page PDF must omit the page input"
    );

    let multi = runtime(10);
    assert!(
        element_by_key(&multi, "pdf-page-nav").is_some(),
        "multi-page PDF must mount the page navigation pill"
    );
    assert!(
        element_by_key(&multi, "pdf-page-input").is_some(),
        "multi-page PDF must mount the page input"
    );
    assert_eq!(multi.state.pdf.page_input_text(), "1");
    let total = element_by_key(&multi, "pdf-page-total").expect("total page label mounted");
    assert_eq!(total.text.as_deref(), Some("/ 10"));
}

#[test]
fn typing_page_number_and_enter_jumps_clamps_and_returns_focus_to_document() {
    let mut rt = runtime(10);

    // Focus input, type a valid page number, press Enter.
    rt.update(|_, tx| tx.focus("pdf-page-input", true));
    settle(&mut rt);
    assert!(rt.state.pdf.is_page_input_focused());
    type_str(&mut rt, "6");
    press_key(&mut rt, Key::Enter);

    assert_eq!(rt.state.pdf.current_page(), 5);
    assert_eq!(rt.state.pdf.page_input_text(), "6");
    assert!(
        rt.state.pdf.visible().contains(&5),
        "target page 6 (index 5) scrolled into view: {:?}",
        rt.state.pdf.visible()
    );
    assert!(!rt.state.pdf.is_page_input_focused());
    assert_eq!(focused_key(&rt).as_deref(), Some("pdf"));

    // Clicking the unfocused input selects all so typing replaces in one shot.
    click_key(&mut rt, "pdf-page-input");
    assert!(rt.state.pdf.is_page_input_focused());
    type_str(&mut rt, "3");
    press_key(&mut rt, Key::Enter);
    assert_eq!(rt.state.pdf.current_page(), 2);
    assert_eq!(rt.state.pdf.page_input_text(), "3");
    assert_eq!(focused_key(&rt).as_deref(), Some("pdf"));

    // Out-of-range high clamps to last page.
    rt.update(|_, tx| tx.focus("pdf-page-input", true));
    settle(&mut rt);
    type_str(&mut rt, "999");
    press_key(&mut rt, Key::Enter);

    assert_eq!(rt.state.pdf.current_page(), 9);
    assert_eq!(rt.state.pdf.page_input_text(), "10");
    assert!(rt.state.pdf.visible().contains(&9));
    assert_eq!(focused_key(&rt).as_deref(), Some("pdf"));

    // Zero clamps to page 1.
    rt.update(|_, tx| tx.focus("pdf-page-input", true));
    settle(&mut rt);
    type_str(&mut rt, "0");
    press_key(&mut rt, Key::Enter);

    assert_eq!(rt.state.pdf.current_page(), 0);
    assert_eq!(rt.state.pdf.page_input_text(), "1");
    assert!(rt.state.pdf.visible().contains(&0));
    assert_eq!(focused_key(&rt).as_deref(), Some("pdf"));
}

#[test]
fn invalid_input_and_escape_restore_current_page_without_jumping_or_leaving() {
    let mut rt = runtime(10);
    rt.update(|h, tx| {
        h.pdf.jump_to_page(3, tx);
    });
    settle(&mut rt);
    assert_eq!(rt.state.pdf.current_page(), 3);
    assert_eq!(rt.state.pdf.page_input_text(), "4");

    // Invalid non-numeric input + Enter restores current page.
    rt.update(|_, tx| tx.focus("pdf-page-input", true));
    settle(&mut rt);
    type_str(&mut rt, "abc");
    press_key(&mut rt, Key::Enter);

    assert_eq!(rt.state.pdf.current_page(), 3);
    assert_eq!(rt.state.pdf.page_input_text(), "4");
    assert_eq!(focused_key(&rt).as_deref(), Some("pdf"));
    assert!(rt.state.intents.is_empty());

    // Empty input + Enter restores current page.
    rt.update(|_, tx| tx.focus("pdf-page-input", true));
    settle(&mut rt);
    press_key(&mut rt, Key::Backspace);
    press_key(&mut rt, Key::Enter);

    assert_eq!(rt.state.pdf.current_page(), 3);
    assert_eq!(rt.state.pdf.page_input_text(), "4");
    assert_eq!(focused_key(&rt).as_deref(), Some("pdf"));

    // Left arrow inside input must not emit PdfIntent::Leave; Escape cancels and returns focus.
    rt.update(|_, tx| tx.focus("pdf-page-input", true));
    settle(&mut rt);
    type_str(&mut rt, "9");
    press_key(&mut rt, Key::Left);
    press_key(&mut rt, Key::Left);
    assert!(
        rt.state.intents.is_empty(),
        "Left inside page input must not emit Leave"
    );

    press_key(&mut rt, Key::Escape);
    assert_eq!(rt.state.pdf.current_page(), 3);
    assert_eq!(rt.state.pdf.page_input_text(), "4");
    assert!(!rt.state.pdf.is_page_input_focused());
    assert_eq!(focused_key(&rt).as_deref(), Some("pdf"));
    assert!(
        rt.state.intents.is_empty(),
        "Escape inside page input must not emit Leave"
    );
}

#[test]
fn prev_and_next_buttons_step_pages_and_disable_at_bounds() {
    let mut rt = runtime(3);

    assert!(is_disabled(&rt, "pdf-page-prev"), "Prev disabled on page 1");
    assert!(!is_disabled(&rt, "pdf-page-next"), "Next enabled on page 1");

    // Clicking disabled Prev stays on page 1.
    click_key(&mut rt, "pdf-page-prev");
    assert_eq!(rt.state.pdf.current_page(), 0);
    assert_eq!(rt.state.pdf.page_input_text(), "1");

    // Clicking Next advances to page 2.
    click_key(&mut rt, "pdf-page-next");
    assert_eq!(rt.state.pdf.current_page(), 1);
    assert_eq!(rt.state.pdf.page_input_text(), "2");
    assert!(!is_disabled(&rt, "pdf-page-prev"));
    assert!(!is_disabled(&rt, "pdf-page-next"));

    // Clicking Next advances to page 3 (last page) and disables Next.
    click_key(&mut rt, "pdf-page-next");
    assert_eq!(rt.state.pdf.current_page(), 2);
    assert_eq!(rt.state.pdf.page_input_text(), "3");
    assert!(!is_disabled(&rt, "pdf-page-prev"));
    assert!(
        is_disabled(&rt, "pdf-page-next"),
        "Next disabled on last page"
    );

    // Clicking Prev steps back to page 2.
    click_key(&mut rt, "pdf-page-prev");
    assert_eq!(rt.state.pdf.current_page(), 1);
    assert_eq!(rt.state.pdf.page_input_text(), "2");
}

#[test]
fn scrolling_updates_unfocused_input_and_blur_or_open_resets_cleanly() {
    let mut rt = runtime(10);

    // Scrolling updates displayed page when unfocused.
    rt.update(|_, tx| tx.scroll_to_y("pdf-scroll", f32::MAX));
    settle(&mut rt);
    assert_eq!(rt.state.pdf.current_page(), 9);
    assert_eq!(rt.state.pdf.page_input_text(), "10");

    // While held (field menu open), blur preserves uncommitted text and regaining focus
    // preserves the user's partial selection instead of clobbering it with select-all.
    rt.update(|_, tx| tx.focus("pdf-page-input", true));
    settle(&mut rt);
    type_str(&mut rt, "12");
    assert_eq!(rt.state.pdf.page_input_text(), "12");

    rt.update(|h, tx| {
        h.pdf.page_nav.edit.set_selection(0, 1);
        h.pdf.hold_page_input(true);
        tx.focus("pdf", false);
    });
    settle(&mut rt);
    assert_eq!(
        rt.state.pdf.page_input_text(),
        "12",
        "held input keeps uncommitted text while field menu is open"
    );

    rt.update(|_, tx| tx.focus("pdf-page-input", false));
    settle(&mut rt);
    assert_eq!(
        rt.state.pdf.page_nav.edit.range(),
        (0, 1),
        "regaining focus while held preserves the user's selection"
    );

    rt.update(|h, tx| {
        h.pdf.hold_page_input(false);
        tx.focus("pdf", false);
    });
    settle(&mut rt);
    assert_eq!(
        rt.state.pdf.page_input_text(),
        "10",
        "blur restores current page once field menu hold releases"
    );

    // Reopening resets page input to "1".
    rt.update(|h, _| h.pdf.open(PdfDocument::new(5, [600.0, 800.0])));
    settle(&mut rt);
    assert_eq!(rt.state.pdf.current_page(), 0);
    assert_eq!(rt.state.pdf.page_input_text(), "1");
}

#[test]
fn page_pill_isolates_drags_and_right_clicks_while_preserving_canvas_and_menu_key() {
    let mut rt = runtime(10);
    rt.update(|_, tx| tx.scroll_to_y("pdf-scroll", f32::MAX));
    settle(&mut rt);

    // Dragging down on the page nav pill (a direction that CAN scroll up from the bottom)
    // must NOT pan the PDF, and right-clicking it must NOT open the PDF context menu.
    let offset_before = rt.state.pdf.scroll_offset();
    assert!(offset_before > 200.0, "scrolled near bottom");
    let (px, py) = center_of(&rt, "pdf-page-total");
    rt.pointer_move(px, py);
    rt.pointer_down(px, py);
    rt.pointer_move(px, py + 120.0);
    rt.pointer_up(px, py + 120.0);
    settle(&mut rt);
    assert!(
        (rt.state.pdf.scroll_offset() - offset_before).abs() < 0.5,
        "dragging down over the page pill must not pan the PDF"
    );

    rt.secondary_down(px, py);
    rt.secondary_up(px, py);
    settle(&mut rt);
    assert!(
        rt.state.intents.is_empty(),
        "right-clicking the page pill must not emit PdfIntent::ContextMenu"
    );

    // Dragging and right-clicking BESIDE the pill still reaches the PDF canvas underneath,
    // and Menu key on the focused PDF document still emits ContextMenu.
    let nav_rect = element_by_key(&rt, "pdf-page-nav").expect("pill").visible;
    let beside_x = (nav_rect[0] / 2.0).max(10.0);
    let beside_y = nav_rect[1] + nav_rect[3] / 2.0;
    rt.pointer_move(beside_x, beside_y);
    rt.pointer_down(beside_x, beside_y);
    rt.pointer_move(beside_x, beside_y + 120.0);
    rt.pointer_up(beside_x, beside_y + 120.0);
    settle(&mut rt);
    assert!(
        rt.state.pdf.scroll_offset() < offset_before - 50.0,
        "dragging beside the pill must still pan the PDF"
    );

    rt.update(|_, tx| tx.focus("pdf", true));
    settle(&mut rt);
    press_key(&mut rt, Key::ContextMenu);
    assert!(
        rt.state
            .intents
            .iter()
            .any(|i| matches!(i, PdfIntent::ContextMenu(_))),
        "Menu key on focused PDF document must still emit PdfIntent::ContextMenu"
    );
}

#[test]
fn jump_to_page_before_first_layout_preserves_reveal_until_viewport_mounts() {
    let mut pdf = PdfState::new("pdf");
    pdf.open(PdfDocument::new(10, [600.0, 800.0]));
    assert!(pdf.jump_to_page(6, &mut Tx::default()));
    assert_eq!(pdf.current_page(), 6);
    assert_eq!(pdf.page_input_text(), "7");

    let host = Host {
        pdf,
        look: Look::fallback(false),
        intents: Vec::new(),
    };
    let mut rt = Runtime::new(host, view, (500.0, 700.0), Env::new(false, 1.0));
    rt.poll = Some(poll);
    settle(&mut rt);

    assert!(
        rt.state.pdf.visible().contains(&6),
        "pre-layout jump_to_page must scroll page 7 into view on first layout: {:?}",
        rt.state.pdf.visible()
    );
    assert_eq!(rt.state.pdf.current_page(), 6);
    assert_eq!(rt.state.pdf.page_input_text(), "7");
}
