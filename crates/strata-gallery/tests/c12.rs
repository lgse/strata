//! C12's page in process: what scripts can't reach (the requests the view asks, their widths
//! and order, stale and cancelled replies, the zoom's rest from wake to wake, the budgets, a
//! pane wider than 1280 points, where a zoom lands far down a mixed document, the cursor, a
//! rebound reset key). The scripts in `tests/scripts/c12/` check the rest. Each test names the
//! bullets it covers.

mod c12 {
    use rune::text::TextSystem;
    use rune::{Cursor, Cx, El, ElementInfo, Env, Key, KeyEvent, Mods, Runtime, WheelEvent};
    use strata_components::pdf_view::{Landed, PdfDocument, PdfState, PdfView, RenderCaps};
    use strata_components::preview_drawer::{DrawerRules, HostedPreview};
    use strata_gallery::scenes::c12::{fake_doc, prose, render};
    use strata_gallery::{Gallery, scenes};
    use strata_kit::fonts;
    use strata_kit::look::Look;

    const FRAME: f64 = 1.0 / 60.0;

    fn scene(name: &str) -> fn(&mut Gallery) {
        scenes::all_scenes()
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, f)| f)
            .unwrap_or_else(|| panic!("no scene {name}"))
    }

    fn mount_with(name: &str, scale: f32, change: impl FnOnce(&mut Gallery)) -> Runtime<Gallery> {
        let mut g = Gallery::default();
        scene(name)(&mut g);
        change(&mut g);
        let env = Env::new(g.dark, scale);
        let mut rt = strata_gallery::runtime(g, env);
        rt.frame(0.0);
        run(&mut rt, 1.0);
        rt
    }

    fn mount(name: &str) -> Runtime<Gallery> {
        mount_with(name, 1.0, |_| {})
    }

    fn run<S: 'static>(rt: &mut Runtime<S>, seconds: f64) {
        for _ in 0..(seconds / FRAME).round() as usize {
            rt.frame(FRAME);
        }
    }

    /// Steps a resting window: a frame only at each requested wake, none in between, for at
    /// most `limit` seconds. Returns the clocks it stepped to.
    fn rest(rt: &mut Runtime<Gallery>, limit: f64) -> Vec<f64> {
        let start = rt.state.c12.now;
        let mut stops = Vec::new();
        for _ in 0..400 {
            let Some(deadline) = rt.wake_deadline() else {
                break;
            };
            if deadline - start > limit {
                break;
            }
            let now = rt.state.c12.now;
            rt.frame((deadline - now).max(0.001));
            stops.push(rt.state.c12.now);
        }
        stops
    }

    fn ctrl() -> Mods {
        Mods {
            ctrl: true,
            cmd: true,
            ..Mods::default()
        }
    }

    fn page(rt: &Runtime<Gallery>, i: usize) -> Option<ElementInfo> {
        let tail = format!("-p{i}/page");
        rt.elements()
            .into_iter()
            .find(|e| e.path.ends_with(&tail) && !e.exiting)
    }

    fn centre(e: &ElementInfo) -> (f32, f32) {
        (e.rect[0] + e.rect[2] / 2.0, e.rect[1] + e.rect[3] / 2.0)
    }

    fn viewport(rt: &Runtime<Gallery>) -> [f32; 4] {
        rt.elements()
            .into_iter()
            .find(|e| e.key.as_deref() == Some("pdf-scroll"))
            .expect("the viewport")
            .rect
    }

    /// The page under the viewport's centre, and how far down it the centre is.
    fn at_centre(rt: &Runtime<Gallery>) -> (usize, f32) {
        let vp = viewport(rt);
        let cy = vp[1] + vp[3] / 2.0;
        for e in rt.elements() {
            if e.key.as_deref() != Some("page") || e.exiting {
                continue;
            }
            let Some(i) = e
                .path
                .rsplit_once("-p")
                .and_then(|(_, t)| t.split('/').next())
                .and_then(|n| n.parse::<usize>().ok())
            else {
                continue;
            };
            if e.rect[1] <= cy && cy < e.rect[1] + e.rect[3] {
                return (i, (cy - e.rect[1]) / e.rect[3]);
            }
        }
        panic!("no page at the centre");
    }

    fn zoom_wheel(rt: &mut Runtime<Gallery>, dy: f32) {
        let (x, y) = centre(&page(rt, rt.state.c12.pdf.current_page()).expect("a page"));
        rt.wheel_event(WheelEvent::new(0.0, dy).at(x, y).mods(ctrl()));
    }

    // preview/preview-panel/pdf: "PDF pages render at the preview pane's width, at most 1280
    // px". preview/preview-panel/layout: "In a preview wider than 1280 px, PDF pages … stop at
    // 1280 px, centered; Ctrl+wheel PDF zoom scales from that width."
    #[test]
    fn pages_render_at_the_panes_width_up_to_1280_points() {
        let rt = mount("c12-pages-light");
        let d = &rt.state.c12;
        let pane = viewport(&rt)[2];
        let wanted = (pane - 12.0).round() as u32;
        assert!(!d.renderer.rendered.is_empty());
        assert!(
            d.renderer.rendered.iter().all(|&(_, w)| w == wanted),
            "{:?}",
            d.renderer.rendered
        );

        // A window wide enough for a 1500-point drawer.
        let mut g = Gallery::default();
        scene("c12-pages-light")(&mut g);
        g.c12
            .drawer
            .set_manual_width(&DrawerRules::default(), 1500.0);
        let text = TextSystem::new(fonts::FONTS, fonts::FAMILIES);
        let mut rt = Runtime::with_text(
            g,
            strata_gallery::view,
            (2400.0, 900.0),
            Env::new(false, 1.0),
            text,
        );
        rt.poll = Some(strata_gallery::poll);
        rt.on_key = Some(strata_gallery::on_key);
        rt.frame(0.0);
        run(&mut rt, 1.0);
        let pane = viewport(&rt);
        assert!(pane[2] > 1400.0, "the pane is wider than 1280: {pane:?}");
        let p0 = page(&rt, 0).expect("page 1");
        assert_eq!(p0.rect[2], 1268.0, "1280 less the gaps");
        assert!(
            (p0.rect[0] + p0.rect[2] / 2.0 - (pane[0] + pane[2] / 2.0)).abs() < 1.0,
            "centred"
        );
        assert!(
            rt.state
                .c12
                .renderer
                .rendered
                .iter()
                .all(|&(_, w)| w == 1268)
        );
        // Zoom scales from that width: two notches is e^0.28 of it.
        zoom_wheel(&mut rt, 40.0);
        zoom_wheel(&mut rt, 40.0);
        rt.frame(FRAME);
        let p0 = page(&rt, 0).expect("page 1");
        assert_eq!(p0.rect[2], (1268.0 * 0.28f32.exp()).round());
    }

    // The work package: "zoom triggers re-render requests for visible pages only". With the zoom
    // resting 0.15 s first, in a window at rest: the wake the zoom asked for is the one that
    // starts the renders, with no frame between.
    #[test]
    fn a_zoom_asks_for_the_visible_pages_once_it_rests() {
        let mut rt = mount("c12-pages-light");
        assert!(rest(&mut rt, 1.0).is_empty(), "nothing left to do at rest");
        let before = rt.state.c12.renderer.rendered.len();
        let visible = rt.state.c12.pdf.visible();
        // A gesture of notches a frame apart: the zoom keeps moving, and nothing renders.
        for _ in 0..4 {
            zoom_wheel(&mut rt, 40.0);
            rt.frame(FRAME);
            assert!(
                rt.state.c12.renderer.busy.is_none(),
                "no render while the zoom moves"
            );
        }
        assert_eq!(rt.state.c12.renderer.rendered.len(), before);
        let t0 = rt.state.c12.now;
        // From wake to wake: the first render starts at the wake that ends the rest. (The bars'
        // fade asks for frames of its own in between; none of them starts one.)
        let mut started = None;
        for _ in 0..100 {
            let Some(deadline) = rt.wake_deadline() else {
                break;
            };
            let now = rt.state.c12.now;
            rt.frame((deadline - now).max(0.001));
            if rt.state.c12.renderer.busy.is_some() {
                started = Some(rt.state.c12.now);
                break;
            }
        }
        let started = started.expect("a render after the rest");
        assert!(
            started >= t0 + 0.15 - 1e-6 && started < t0 + 0.15 + FRAME,
            "the render starts as the rest ends: {started} from {t0}"
        );
        rest(&mut rt, 2.0);
        let d = &rt.state.c12;
        let again: Vec<usize> = d.renderer.rendered[before..].iter().map(|r| r.0).collect();
        assert!(!again.is_empty());
        let visible_now = d.pdf.visible();
        assert!(
            again
                .iter()
                .all(|p| visible.contains(p) || visible_now.contains(p) || *p == visible_now.end),
            "only pages on screen (or the next one): {again:?}, {visible:?} then {visible_now:?}"
        );
        assert!(d.renderer.rendered[before..].iter().all(|r| r.1 > 500));
    }

    // preview/preview-panel/pdf: "Queued PDF pages that leave the view are cancelled before
    // rendering." Ctrl+End while the first pages render.
    #[test]
    fn a_jump_cancels_what_was_asked_for_the_pages_left_behind() {
        let mut g = Gallery::default();
        scene("c12-long-light")(&mut g);
        let mut rt = strata_gallery::runtime(g, Env::new(false, 1.0));
        rt.frame(0.0);
        run(&mut rt, 0.1);
        let (x, y) = centre(&page(&rt, 0).expect("page 1"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
        // Jump just after a render starts, so the cancel can reach it before it's done.
        let started = rt.state.c12.renderer.busy.map(|(r, _)| r.ticket);
        for _ in 0..30 {
            rt.frame(FRAME);
            if rt.state.c12.renderer.busy.map(|(r, _)| r.ticket) != started {
                break;
            }
        }
        let busy = rt.state.c12.renderer.busy.map(|(r, _)| r.page);
        let busy = busy.expect("a render in flight when the jump comes");
        rt.key(KeyEvent {
            key: Key::End,
            mods: ctrl(),
        });
        rt.frame(FRAME);
        rt.frame(FRAME);
        rt.frame(FRAME);
        assert!(
            rt.state.c12.renderer.cancelled.contains(&busy),
            "the render in flight for page {} is cancelled as the jump lands",
            busy + 1
        );
        run(&mut rt, 1.0);
        let d = &rt.state.c12;
        assert!(d.pdf.visible().contains(&499), "{:?}", d.pdf.visible());
        let late: Vec<usize> = d
            .renderer
            .rendered
            .iter()
            .map(|r| r.0)
            .skip_while(|&p| p < 100)
            .collect();
        assert!(
            late.iter().all(|&p| p > 490),
            "only the end renders after the jump: {late:?}"
        );
        assert!(
            d.renderer.rendered.iter().all(|r| r.0 < 3 || r.0 > 490),
            "nothing between"
        );
    }

    #[test]
    fn a_reply_for_the_last_document_is_dropped() {
        let mut rt = mount("c12-loading-light");
        let d = &mut rt.state.c12;
        let req = d.pdf.pop().expect("a request");
        d.pdf.open(PdfDocument::new(12, [612.0, 792.0]));
        assert_eq!(d.pdf.take_cancelled(), vec![req.ticket]);
        let reply = render(&d.doc[req.page], req.box_px, false);
        assert_eq!(d.pdf.complete(req.ticket, reply), Landed::Dropped);
        assert!(d.pdf.held(req.page).is_none());
    }

    /// The 500-page scene with every third page landscape, sizes the host knows up front.
    fn mixed_known(g: &mut Gallery) {
        g.c12.doc = (0..500)
            .map(|p| {
                let mut page = fake_doc(1, false).remove(0);
                if p % 3 == 1 {
                    page.size = [792.0, 612.0];
                }
                page
            })
            .collect();
        let mut doc = PdfDocument::new(500, [612.0, 792.0]);
        for p in (0..500).filter(|p| p % 3 == 1) {
            doc = doc.with_size(p, [792.0, 612.0]);
        }
        g.c12.pdf.open(doc);
    }

    /// Zooms `notches` notches at page `target`, `gap` frames apart: where the centre was, where
    /// it lands, and the farthest any frame drew it from there, in pages.
    fn zoom_mixed(target: usize, notches: usize, gap: usize) -> ((usize, f32), (usize, f32), f32) {
        let mut rt = mount_with("c12-long-light", 1.0, mixed_known);
        rt.state.c12.pdf.show_page(target, 0.4);
        run(&mut rt, 2.0);
        let before = at_centre(&rt);
        let (x, y) = centre(&page(&rt, before.0).expect("the centre page"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
        assert_eq!(at_centre(&rt), before, "a click doesn't move the view");
        let far = |at: (usize, f32)| (at.0 as f32 + at.1 - before.0 as f32 - before.1).abs();
        let mut worst: f32 = 0.0;
        for _ in 0..notches {
            zoom_wheel(&mut rt, 40.0);
            rt.frame(FRAME);
            worst = worst.max(far(at_centre(&rt)));
            for _ in 0..gap {
                rt.frame(FRAME);
                worst = worst.max(far(at_centre(&rt)));
            }
        }
        for _ in 0..12 {
            rt.frame(FRAME);
            worst = worst.max(far(at_centre(&rt)));
        }
        run(&mut rt, 0.5);
        (before, at_centre(&rt), worst)
    }

    // D6: a zoom far down a document of mixed page sizes keeps the centre once its page lands:
    // one notch, notches on consecutive frames (review 1's B2: each took the centre from pages
    // the last notch's estimate had put on screen, and landed 3 pages off), and notches apart.
    #[test]
    fn a_zoom_far_down_a_mixed_document_keeps_the_centre() {
        for (target, notches, gap) in [(300, 1, 0), (300, 4, 0), (300, 4, 20), (30, 4, 0)] {
            let (before, after, worst) = zoom_mixed(target, notches, gap);
            eprintln!(
                "page {target}, {notches} notches {gap} frames apart: {before:?} then {after:?}, \
                 at most {worst:.2} pages off on the way"
            );
            assert_eq!(after.0, before.0, "the same page");
            assert!(
                (after.1 - before.1).abs() < 0.02,
                "the same point: {before:?} then {after:?}"
            );
            assert!(worst < 1.0, "no frame more than a page off: {worst}");
        }
        // Ctrl+0 back to fitted width lands on the same page.
        let mut rt = mount_with("c12-long-light", 1.0, mixed_known);
        rt.state.c12.pdf.show_page(300, 0.4);
        run(&mut rt, 2.0);
        let (p0, f0) = at_centre(&rt);
        let (x, y) = centre(&page(&rt, p0).expect("the centre page"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
        for _ in 0..4 {
            zoom_wheel(&mut rt, 40.0);
            rt.frame(FRAME);
        }
        run(&mut rt, 0.3);
        rt.key(KeyEvent {
            key: Key::Command('0'),
            mods: ctrl(),
        });
        run(&mut rt, 0.3);
        assert_eq!(rt.state.c12.pdf.level(), 1.0);
        let (p2, f2) = at_centre(&rt);
        assert_eq!(p2, p0, "Ctrl+0 lands on the same page");
        assert!((f2 - f0).abs() < 0.02, "{f0} then {f2}");
    }

    // A Ctrl+wheel in the frame after a plain wheel moved the pages: input comes after the
    // pages report where the scroll put them, so the zoom keeps the point under the centre.
    #[test]
    fn a_ctrl_wheel_right_after_a_scroll_keeps_the_centre() {
        let mut rt = mount("c12-pages-light");
        let vp = viewport(&rt);
        rt.wheel(vp[0] + 50.0, vp[1] + 50.0, -150.0);
        rt.frame(FRAME);
        let before = at_centre(&rt);
        zoom_wheel(&mut rt, 40.0);
        rt.frame(FRAME);
        run(&mut rt, 0.3);
        let after = at_centre(&rt);
        assert_eq!(after.0, before.0);
        assert!(
            (after.1 - before.1).abs() < 0.02,
            "{before:?} then {after:?}"
        );
    }

    // A zoom asked as the pages first lay out (the zoomed scene zooms from its first poll after
    // them) keeps the centre too: a row of the last zoom that reports its layout late doesn't
    // place the anchor.
    #[test]
    fn a_zoom_right_after_the_first_layout_keeps_the_centre() {
        let at_fit = at_centre(&mount("c12-pages-light"));
        let zoomed = mount("c12-zoomed-light");
        assert_eq!(zoomed.state.c12.pdf.level(), 2.0);
        let at_two = at_centre(&zoomed);
        assert_eq!(at_two.0, at_fit.0);
        assert!(
            (at_two.1 - at_fit.1).abs() < 0.02,
            "{at_fit:?} then {at_two:?}"
        );
    }

    // A zoom from the host's poll in the frame after a scroll moved the rows.
    #[test]
    fn a_zoom_right_after_a_scroll_keeps_the_centre() {
        let mut rt = mount("c12-pages-light");
        let vp = viewport(&rt);
        rt.wheel(vp[0] + 50.0, vp[1] + 50.0, -150.0);
        rt.frame(FRAME);
        let before = at_centre(&rt);
        rt.state.c12.zoom_once = Some(2.0);
        rt.frame(FRAME);
        run(&mut rt, 0.3);
        let after = at_centre(&rt);
        assert_eq!(after.0, before.0);
        assert!(
            (after.1 - before.1).abs() < 0.02,
            "{before:?} then {after:?}"
        );
    }

    // A window moved to a 2× display: no input and no layout change in points, yet the visible
    // pages ask again at twice the pixels (C14a's rescale element).
    #[test]
    fn a_new_display_scale_renders_the_visible_pages_again() {
        let mut rt = mount("c12-pages-light");
        rest(&mut rt, 1.0);
        let before = rt.state.c12.renderer.rendered.len();
        let w = rt.state.c12.renderer.rendered[0].1;
        rt.set_viewport(strata_gallery::SIZE, 2.0);
        run(&mut rt, 1.0);
        let d = &rt.state.c12;
        let again = &d.renderer.rendered[before..];
        assert!(!again.is_empty(), "renders again");
        assert!(again.iter().all(|r| r.1 == 2 * w), "{again:?} after {w}");
        assert!(
            again
                .iter()
                .all(|r| d.pdf.visible().contains(&r.0) || r.0 == d.pdf.visible().end)
        );
    }

    // Review 1's B1: a page whose text layer the budget dropped while it kept its image gets
    // its text back once it's in view again.
    #[test]
    fn a_page_that_lost_its_text_gets_it_back_in_view() {
        let mut rt = mount_with("c12-long-light", 1.0, |g| {
            g.c12.pdf.set_budgets(1 << 30, 300 << 10);
        });
        assert!(text_of(&rt, 0), "page 1 has its text at first");
        for _ in 0..30 {
            let vp = viewport(&rt);
            rt.wheel(vp[0] + 50.0, vp[1] + 50.0, -300.0);
            rt.frame(1.0 / 10.0);
        }
        run(&mut rt, 1.0);
        for _ in 0..40 {
            let vp = viewport(&rt);
            rt.wheel(vp[0] + 50.0, vp[1] + 50.0, 300.0);
            rt.frame(1.0 / 10.0);
        }
        run(&mut rt, 2.0);
        let d = &rt.state.c12.pdf;
        assert!(d.visible().contains(&0) && d.held(0).is_some());
        assert!(text_of(&rt, 0), "page 1 is back in view with its text");
        assert!(d.held_bytes().1 <= 300 << 10);
    }

    fn text_of(rt: &Runtime<Gallery>, i: usize) -> bool {
        let tail = format!("-p{i}/page/text");
        rt.elements()
            .into_iter()
            .any(|e| e.path.ends_with(&tail) && !e.exiting)
    }

    // The text budget holds while find is open: pages whose layers it drops stay counted, and
    // aren't asked for their text again while the query runs.
    #[test]
    fn find_holds_the_text_budget_and_keeps_its_count() {
        let count = |budget: usize| {
            let mut rt = mount_with("c12-long-light", 1.0, |g| {
                g.c12.pdf.set_searchable(true);
                g.c12.pdf.set_budgets(64 << 20, budget);
            });
            rt.state.c12.pdf.find("strata");
            rt.frame(FRAME);
            run(&mut rt, 40.0);
            let d = &rt.state.c12;
            let total = d.pdf.find_status().map(|s| s.total);
            (total, d.pdf.held_bytes().1, d.renderer.rendered.len())
        };
        let (all, _, _) = count(usize::MAX);
        let (capped, text, renders) = count(4 << 20);
        assert_eq!(capped, all, "the same count");
        assert!(text <= 4 << 20, "{text} bytes of text held");
        assert!(renders <= 502, "each page asked once, not again: {renders}");
    }

    // Review 1's S3: when the zoom rests, a request still out for a visible page at the width
    // before it is cancelled at once.
    #[test]
    fn the_rest_cancels_a_request_at_the_old_width() {
        let mut rt = mount_with("c12-pages-light", 1.0, |g| g.c12.renderer.paused = true);
        let req = rt.state.c12.pdf.pop().expect("a request");
        assert_eq!(req.page, 0);
        for _ in 0..3 {
            zoom_wheel(&mut rt, 40.0);
            rt.frame(FRAME);
        }
        assert!(
            rt.state.c12.renderer.cancelled.is_empty(),
            "kept while the zoom moves"
        );
        run(&mut rt, 0.2);
        assert_eq!(
            rt.state.c12.renderer.cancelled,
            [0],
            "cancelled when it rests"
        );
    }

    // Strata's `select_all` returns false when no page has text: the key goes on.
    #[test]
    fn ctrl_a_with_no_text_goes_on() {
        let mut rt = mount("c12-loading-light");
        let (x, y) = centre(&page(&rt, 0).expect("page 1"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
        rt.key(KeyEvent {
            key: Key::Command('a'),
            mods: ctrl(),
        });
        rt.frame(FRAME);
        assert_eq!(rt.state.c12.last, "Ctrl+A passed on");
    }

    // The I-beam follows text that scrolls under a still pointer.
    #[test]
    fn the_cursor_follows_text_scrolled_under_a_still_pointer() {
        let mut rt = mount("c12-pages-light");
        let p0 = page(&rt, 0).expect("page 1");
        let k = p0.rect[2] / 612.0;
        // Over the last line of page 1 with text (the fake page's lines are 16 points apart from
        // 72, their boxes 13 points tall).
        let last = prose(0, 38)
            .iter()
            .rposition(|l| l.len() > 20)
            .expect("a line of text") as f32;
        let (x, y) = (
            p0.rect[0] + (54.0 + 6.6 * 5.0) * k,
            p0.rect[1] + (72.0 + last * 16.0 + 6.0) * k,
        );
        rt.pointer_move(x, y);
        rt.frame(FRAME);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(FRAME);
        assert_eq!(rt.cursor(), Cursor::Text);
        // Down scrolls 40 points: the bottom margin comes under the pointer.
        rt.key(KeyEvent {
            key: Key::Down,
            mods: Mods::default(),
        });
        run(&mut rt, 0.3);
        assert_eq!(rt.cursor(), Cursor::Grab);
    }

    // A drag from text that ends over a margin shows the grab once released, not the I-beam
    // it started with.
    #[test]
    fn the_cursor_is_worked_out_again_on_release() {
        let mut rt = mount("c12-pages-light");
        let p0 = page(&rt, 0).expect("page 1");
        let (x, y) = centre(&p0);
        rt.pointer_move(x, y);
        rt.frame(FRAME);
        assert_eq!(rt.cursor(), Cursor::Text);
        rt.pointer_down(x, y);
        rt.frame(FRAME);
        let margin = (p0.rect[0] + 8.0, y);
        for i in 1..=8 {
            rt.pointer_move(x + (margin.0 - x) * i as f32 / 8.0, y);
            rt.frame(FRAME);
        }
        rt.pointer_up(margin.0, margin.1);
        rt.frame(FRAME);
        assert_eq!(rt.cursor(), Cursor::Grab);
    }

    #[test]
    fn scrolling_through_holds_images_within_the_budget() {
        let mut rt = mount_with("c12-long-light", 1.0, |g| {
            g.c12.pdf.set_budgets(8 << 20, 1 << 20);
        });
        for _ in 0..200 {
            let vp = viewport(&rt);
            rt.wheel(vp[0] + 50.0, vp[1] + 50.0, -300.0);
            rt.frame(1.0 / 20.0);
        }
        run(&mut rt, 1.0);
        let d = &rt.state.c12.pdf;
        let (images, text) = d.held_bytes();
        assert!(images <= 8 << 20, "{images}");
        assert!(text <= 1 << 20, "{text}");
        for p in d.visible() {
            assert!(d.held(p).is_some(), "a visible page keeps its image: {p}");
        }
    }

    // preview/preview-panel/pdf: "Dragging on a page margin … pans the view instead of
    // selecting." With the cursor Strata shows: text over text, grab elsewhere, grabbing while
    // panning.
    #[test]
    fn the_cursor_shows_which_a_drag_does_and_a_margin_drag_pans() {
        let mut rt = mount("c12-pages-light");
        let p0 = page(&rt, 0).expect("page 1");
        let (x, y) = centre(&p0);
        rt.pointer_move(x, y);
        rt.frame(FRAME);
        assert_eq!(rt.cursor(), Cursor::Text);
        let margin = (p0.rect[0] + 8.0, y);
        rt.pointer_move(margin.0, margin.1);
        rt.frame(FRAME);
        assert_eq!(rt.cursor(), Cursor::Grab);
        let top = p0.rect[1];
        rt.pointer_down(margin.0, margin.1);
        rt.frame(FRAME);
        for i in 1..=10 {
            rt.pointer_move(margin.0, margin.1 - i as f32 * 20.0);
            rt.frame(FRAME);
        }
        assert_eq!(rt.cursor(), Cursor::Grabbing);
        rt.pointer_up(margin.0, margin.1 - 200.0);
        rt.frame(FRAME);
        let moved = top - page(&rt, 0).expect("page 1").rect[1];
        assert!((moved - 200.0).abs() < 1.0, "panned 200 points: {moved}");
        assert!(!rt.state.c12.pdf.has_selection());
    }

    /// A host of the test's own, for a rebound reset key: Strata's `pdf_zoom_reset` with
    /// Ctrl+E.
    struct Host {
        pdf: PdfState,
        look: Look,
    }

    fn host_view(h: &Host, cx: &mut Cx<Host>) -> El {
        PdfView::new(|h: &Host| &h.pdf, |h: &mut Host| &mut h.pdf)
            .reset_key(|e| e.is_command('e'))
            .build(h, &h.look, cx)
    }

    // app/window/key-dispatch: "With `text_reset = ["<Control>9"]` and `pdf_zoom_reset =
    // ["<Control>0", "<Control>e"]`, Ctrl+0 and Ctrl+E reset a focused document's zoom" (the
    // key map is Strata's; the view takes the keys it's given).
    #[test]
    fn a_rebound_reset_key_resets_the_zoom() {
        let mut pdf = PdfState::new("pdf");
        let doc = fake_doc(3, false);
        pdf.set_caps(RenderCaps::default());
        pdf.open_with(
            PdfDocument::new(3, [612.0, 792.0]),
            0,
            render(&doc[0], [400, 1800], false),
        );
        let h = Host {
            pdf,
            look: Look::fallback(false),
        };
        let text = TextSystem::new(fonts::FONTS, fonts::FAMILIES);
        let mut rt = Runtime::with_text(h, host_view, (600.0, 600.0), Env::new(false, 1.0), text);
        rt.frame(0.0);
        run(&mut rt, 0.2);
        let e = rt
            .elements()
            .into_iter()
            .find(|e| e.path.ends_with("-p0/page"))
            .expect("page 1");
        let (x, y) = (e.rect[0] + 20.0, e.rect[1] + 20.0);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.wheel_event(WheelEvent::new(0.0, 80.0).at(x, y).mods(ctrl()));
        rt.frame(FRAME);
        assert!(rt.state.pdf.level() > 1.0);
        rt.key(KeyEvent {
            key: Key::Command('0'),
            mods: ctrl(),
        });
        rt.frame(FRAME);
        assert!(rt.state.pdf.level() > 1.0, "Ctrl+0 isn't the key now");
        rt.key(KeyEvent {
            key: Key::Command('e'),
            mods: ctrl(),
        });
        rt.frame(FRAME);
        assert_eq!(rt.state.pdf.level(), 1.0);
    }
}
