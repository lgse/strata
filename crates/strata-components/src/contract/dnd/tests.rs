use std::path::{Path, PathBuf};

use rune::{Cx, DragOperation, El, Env, Runtime, col, el, row};

use super::binder::drag_item;
use super::*;

const COPY: DropActions = DropActions::COPY;
const MOVE: DropActions = DropActions::MOVE;
const BOTH: DropActions = DropActions::COPY.union(DropActions::MOVE);

fn mods(ctrl: bool, shift: bool) -> Mods {
    Mods {
        ctrl,
        cmd: ctrl,
        shift,
        ..Mods::default()
    }
}

fn choose(
    m: Mods,
    offered: DropActions,
    volume: VolumeRelation,
    cross_device: CrossDevice,
) -> Option<DropAction> {
    choose_action(ActionInput::new(m, offered, volume, cross_device))
}

const PLAIN: (bool, bool) = (false, false);
const CTRL: (bool, bool) = (true, false);
const SHIFT: (bool, bool) = (false, true);
const CTRL_SHIFT: (bool, bool) = (true, true);

fn with((ctrl, shift): (bool, bool)) -> Mods {
    mods(ctrl, shift)
}

// operations/drag-and-drop/copy-or-move: "A plain drop onto a folder on the same filesystem
// moves the items."
#[test]
fn a_plain_drop_on_the_same_volume_moves() {
    for cross in [CrossDevice::Copy, CrossDevice::Move, CrossDevice::Ask] {
        let got = choose(with(PLAIN), BOTH, VolumeRelation::Same, cross);
        assert_eq!(got, Some(DropAction::Move), "{cross:?}");
    }
    // "When the dragging application does not offer move, the drop copies."
    let copy_only = choose(with(PLAIN), COPY, VolumeRelation::Same, CrossDevice::Ask);
    assert_eq!(
        copy_only,
        Some(DropAction::Copy),
        "a source that offers no move copies"
    );
}

// "Holding Ctrl at the drop copies and holding Shift moves, on the same or another device."
#[test]
fn ctrl_copies_and_shift_moves_on_any_volume() {
    for volume in [
        VolumeRelation::Same,
        VolumeRelation::Different,
        VolumeRelation::Unknown,
    ] {
        for cross in [CrossDevice::Copy, CrossDevice::Move, CrossDevice::Ask] {
            let at = format!("{volume:?} {cross:?}");
            assert_eq!(
                choose(with(CTRL), BOTH, volume, cross),
                Some(DropAction::Copy),
                "{at}"
            );
            assert_eq!(
                choose(with(SHIFT), BOTH, volume, cross),
                Some(DropAction::Move),
                "{at}"
            );
            let both = choose(with(CTRL_SHIFT), BOTH, volume, cross);
            assert_eq!(both, Some(DropAction::Copy), "Ctrl wins over Shift: {at}");
        }
    }
}

#[test]
fn a_modifier_for_an_action_not_offered_falls_back_to_the_plain_rule() {
    let same = VolumeRelation::Same;
    assert_eq!(
        choose(with(CTRL), MOVE, same, CrossDevice::Ask),
        Some(DropAction::Move)
    );
    assert_eq!(
        choose(with(SHIFT), COPY, same, CrossDevice::Ask),
        Some(DropAction::Copy)
    );
    let other = VolumeRelation::Different;
    assert_eq!(
        choose(with(CTRL_SHIFT), MOVE, other, CrossDevice::Copy),
        Some(DropAction::Move),
        "Ctrl without copy falls to the plain rule, not to Shift"
    );
}

// "Settings → … offers Always copy, Always move, and Always ask"; "With Always ask, a plain
// drop onto another device transfers nothing until the prompt is answered."
#[test]
fn a_plain_drop_onto_another_device_follows_the_setting() {
    let table = [
        (CrossDevice::Copy, BOTH, DropAction::Copy),
        (CrossDevice::Copy, MOVE, DropAction::Move),
        (CrossDevice::Move, BOTH, DropAction::Move),
        (CrossDevice::Move, COPY, DropAction::Copy),
        (CrossDevice::Ask, BOTH, DropAction::Ask),
        (CrossDevice::Ask, COPY, DropAction::Copy),
        (CrossDevice::Ask, MOVE, DropAction::Move),
    ];
    for (cross, offered, want) in table {
        for volume in [VolumeRelation::Different, VolumeRelation::Unknown] {
            let got = choose(with(PLAIN), offered, volume, cross);
            assert_eq!(got, Some(want), "{cross:?} {offered:?} {volume:?}");
        }
    }
}

// Strata's `drop_commit` forbids a drop offering neither copy nor move; no bullet names it.
#[test]
fn a_drop_offering_neither_copy_nor_move_is_forbidden() {
    for m in [PLAIN, CTRL, SHIFT] {
        for offered in [DropActions::NONE, DropActions::LINK] {
            let got = choose(with(m), offered, VolumeRelation::Same, CrossDevice::Ask);
            assert_eq!(got, None, "{m:?} {offered:?}");
        }
    }
}

#[test]
fn a_zone_without_an_action_hook_assumes_an_unknown_volume() {
    let zone: DropZone<()> = DropZone::new(|_, _, _| {});
    let files = DropOffer::new(vec![URI_LIST.into()], BOTH);
    assert_eq!(zone.choose(&(), &files, with(PLAIN)), Some(DropAction::Ask));
    assert_eq!(
        zone.choose(&(), &files, with(SHIFT)),
        Some(DropAction::Move)
    );
    let text = DropOffer::new(vec!["text/plain".into()], BOTH);
    assert_eq!(
        zone.choose(&(), &text, with(PLAIN)),
        None,
        "a refused offer has no action"
    );
    let hooked = zone.action(|_, _, _| Some(DropAction::Link));
    assert_eq!(
        hooked.choose(&(), &files, with(PLAIN)),
        Some(DropAction::Link)
    );
}

#[test]
fn a_zone_takes_local_drags_and_its_mime_types() {
    let zone: DropZone<()> = DropZone::new(|_, _, _| {});
    let local = DropOffer::new(Vec::new(), BOTH).local(true);
    assert!(zone.accepts(&local));
    assert!(!zone.clone().local(false).accepts(&local));
    let png = DropOffer::new(vec!["text/html".into(), "image/png".into()], COPY);
    assert!(!zone.accepts(&png));
    assert!(zone.mime_types(&["image/png", URI_LIST]).accepts(&png));
}

#[test]
fn file_uris_round_trip_through_a_uri_list() {
    let paths = [
        PathBuf::from("/home/me/a b.txt"),
        PathBuf::from("/srv/100%/Café déjà vu.md"),
        PathBuf::from("/tmp/#hash?q.txt"),
        PathBuf::from("relative/x.png"),
    ];
    let data = DropData::files(&paths);
    assert_eq!(
        data.uris(),
        [
            "file:///home/me/a%20b.txt",
            "file:///srv/100%25/Caf%C3%A9%20d%C3%A9j%C3%A0%20vu.md",
            "file:///tmp/%23hash%3Fq.txt",
            "file:relative/x.png",
        ]
    );
    assert_eq!(data.file_paths(), paths);
}

#[cfg(unix)]
#[test]
fn a_path_that_is_not_utf8_round_trips() {
    use std::os::unix::ffi::OsStrExt;
    let path = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff\xfe.bin"));
    let uri = file_uri(path);
    assert_eq!(uri, "file:///tmp/%FF%FE.bin");
    assert_eq!(uri_to_path(&uri).as_deref(), Some(path));
}

#[test]
fn a_uri_list_skips_comments_blank_lines_and_line_ends() {
    let list =
        b"# from a browser\r\nfile:///a\r\n\r\nhttps://example.com/x\nFILE://localhost/b%2Fc\n";
    let uris = parse_uri_list(list);
    assert_eq!(
        uris,
        [
            "file:///a",
            "https://example.com/x",
            "FILE://localhost/b%2Fc"
        ]
    );
    let data = DropData::new(vec![DropItem::new(URI_LIST, list.to_vec())]);
    assert_eq!(
        data.file_paths(),
        [PathBuf::from("/a"), PathBuf::from("/b/c")]
    );
}

#[test]
fn only_local_file_uris_name_paths() {
    assert_eq!(
        uri_to_path("file:/etc/hosts"),
        Some(PathBuf::from("/etc/hosts"))
    );
    for uri in [
        "https://example.com/a",
        "file://server/share/a",
        "file:///bad%zzescape",
        "file:///cut%2",
        "file:///sign%+1",
        "file:///nul%00byte",
        "file://",
        "file:?query",
        "fil",
    ] {
        assert_eq!(uri_to_path(uri), None, "{uri}");
    }
}

#[test]
fn a_query_or_fragment_is_no_part_of_the_path() {
    let path = |uri| uri_to_path(uri).expect("a local file");
    assert_eq!(path("file:///a/b.txt?x=1"), PathBuf::from("/a/b.txt"));
    assert_eq!(path("file:///a/b.txt#top"), PathBuf::from("/a/b.txt"));
    assert_eq!(
        path("file:///a/%3F%23.txt"),
        PathBuf::from("/a/?#.txt"),
        "escaped, they're the name"
    );
}

#[test]
fn a_payload_becomes_a_rune_drag_item() {
    let payload = DragPayload::new(
        vec![EntryId::new(1), EntryId::new(2)],
        vec!["file:///a/b.txt".into(), "https://example.com/".into()],
    );
    let item = drag_item(&payload).expect("a payload with files drags");
    assert_eq!(item.paths(), [PathBuf::from("/a/b.txt")]);
    assert_eq!(item.plain_text(), Some("https://example.com/"));
    assert!(item.offers(DragOperation::Copy) && item.allows_move());
    assert!(!item.offers(DragOperation::Link));

    let link = drag_item(&payload.clone().actions(DropActions::LINK)).expect("link only");
    assert!(link.offers(DragOperation::Link) && !link.offers(DragOperation::Copy));
    assert!(drag_item(&payload.clone().actions(DropActions::NONE)).is_none());
    assert!(drag_item(&DragPayload::new(vec![EntryId::new(1)], Vec::new())).is_none());
}

/// A pane background (`outer`, 400 by 300) around two rows of 100-point cells. The first: a
/// folder row that follows moves (`row`), a zone that takes only PNG and says so (`refuse`),
/// one that takes only PNG and passes other drags on (`pass`), and one attached through
/// [`Dnd::none`] (`none`). The second: zones whose `action` refuses every drag, one with an
/// `on_reject` (`veto`) and one without (`quiet-veto`). Below them, bare background.
#[derive(Default)]
struct Pane {
    log: Vec<String>,
    dropped: Vec<Dropped>,
    /// Every offer a hover or drop carried: the zone, the serial, and whether its URIs were read.
    offers: Vec<(&'static str, u64, bool)>,
}

fn logging(key: &'static str) -> DropZone<Pane> {
    DropZone::new(move |s: &mut Pane, d: Dropped, _| {
        s.log.push(format!("{key} drop"));
        s.offers.push((key, d.offer.serial, d.offer.uris.is_some()));
        s.dropped.push(d);
    })
    .on_hover(move |s: &mut Pane, h, offer: &DropOffer, _| {
        s.log.push(format!("{key} {h:?}"));
        s.offers.push((key, offer.serial, offer.uris.is_some()));
    })
}

fn view(_: &Pane, _: &mut Cx<Pane>) -> El {
    let dnd = Dnd::<Pane>::rune();
    let cell = |key: &'static str| el().key(key).size(100.0, 100.0);
    let follow = logging("row").on_move(|s: &mut Pane, p, _| {
        s.log.push(format!("row move {},{}", p.x, p.y));
    });
    let refuse =
        logging("refuse")
            .mime_types(&["image/png"])
            .on_reject(|s: &mut Pane, offer, _| {
                s.log
                    .push(format!("refuse rejected {:?}", offer.mime_types));
            });
    let pass = logging("pass").mime_types(&["image/png"]);
    let veto = logging("veto")
        .action(|_, _, _| None)
        .on_reject(|s: &mut Pane, offer, _| {
            s.log.push(format!("veto rejected {:?}", offer.uris));
        });
    let quiet_veto = logging("quiet-veto").action(|_, _, _| None);
    let cells = row()
        .child(dnd.drop_zone(cell("row"), follow))
        .child(dnd.drop_zone(cell("refuse"), refuse))
        .child(dnd.drop_zone(cell("pass"), pass))
        .child(Dnd::none().drop_zone(cell("none"), logging("none")));
    let vetoes = row()
        .child(dnd.drop_zone(cell("veto"), veto))
        .child(dnd.drop_zone(cell("quiet-veto"), quiet_veto));
    let pane = el()
        .key("outer")
        .size(400.0, 300.0)
        .child(col().child(cells).child(vetoes));
    dnd.drop_zone(pane, logging("outer"))
}

fn pane() -> Runtime<Pane> {
    let mut rt = Runtime::new(Pane::default(), view, (400.0, 300.0), Env::new(true, 1.0));
    rt.frame(0.0);
    rt
}

fn step(rt: &mut Runtime<Pane>, frame: &mut u32) {
    *frame += 1;
    rt.frame(f64::from(*frame) / 60.0);
}

fn files(names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(PathBuf::from).collect()
}

fn drop_at(rt: &mut Runtime<Pane>, frame: &mut u32, names: &[&str], at: (f32, f32)) {
    rt.drag_files(files(names), Some(at));
    step(rt, frame);
    rt.drop_files(files(names), Some(at));
    step(rt, frame);
}

#[test]
fn the_rune_binder_hovers_the_innermost_zone_and_delivers_a_uri_list() {
    let mut rt = pane();
    let mut f = 0;
    rt.drag_files(files(&["/home/me/a b.txt"]), Some((50.0, 50.0)));
    step(&mut rt, &mut f);
    rt.drag_files_to(Some((50.0, 250.0)));
    step(&mut rt, &mut f);
    rt.drop_files(files(&["/home/me/a b.txt", "rel.png"]), Some((50.0, 250.0)));
    step(&mut rt, &mut f);
    assert_eq!(
        rt.state.log,
        [
            "row Accepting",
            "row move 50,50",
            "row Left",
            "outer Accepting",
            "outer Left",
            "outer drop"
        ]
    );
    let dropped = &rt.state.dropped[0];
    assert_eq!(
        dropped.data.uris(),
        ["file:///home/me/a%20b.txt", "file:rel.png"]
    );
    assert_eq!(
        dropped.data.file_paths(),
        files(&["/home/me/a b.txt", "rel.png"])
    );
    let offer = &dropped.offer;
    assert_eq!(
        (offer.mime_types.as_ref(), offer.actions, offer.local),
        ([URI_LIST.into()].as_slice(), BOTH, false)
    );
    assert_eq!(
        offer.uris.as_deref(),
        Some(dropped.data.uris().as_slice()),
        "the drop's offer names its URIs"
    );
    assert!(dropped.data.local.is_none());
    let point = dropped.point.expect("a headless drop has a point");
    assert_eq!((point.x, point.y), (50.0, 250.0));
}

#[test]
fn a_moving_drag_reaches_a_zone_that_follows_it() {
    let mut rt = pane();
    let mut f = 0;
    rt.drag_files(files(&["/a"]), Some((10.0, 10.0)));
    step(&mut rt, &mut f);
    assert!(
        rt.drag_files_to(Some((60.0, 70.0))),
        "the row follows moves"
    );
    step(&mut rt, &mut f);
    assert_eq!(
        rt.state.log,
        ["row Accepting", "row move 10,10", "row move 60,70"]
    );
}

#[test]
fn a_zone_with_a_reject_hook_stops_a_drag_it_refuses() {
    let mut rt = pane();
    let mut f = 0;
    drop_at(&mut rt, &mut f, &["/a.txt"], (150.0, 50.0));
    assert_eq!(
        rt.state.log,
        [
            "refuse Rejecting",
            "refuse Left",
            r#"refuse rejected ["text/uri-list"]"#
        ]
    );
    assert!(rt.state.dropped.is_empty(), "nothing takes the drop");
}

#[test]
fn a_zone_that_refuses_quietly_passes_the_drag_to_the_zone_around_it() {
    let mut rt = pane();
    let mut f = 0;
    drop_at(&mut rt, &mut f, &["/a.txt"], (250.0, 50.0));
    assert_eq!(
        rt.state.log,
        ["outer Accepting", "outer Left", "outer drop"]
    );
}

#[test]
fn the_none_binder_attaches_nothing() {
    let mut rt = pane();
    let mut f = 0;
    drop_at(&mut rt, &mut f, &["/a.txt"], (350.0, 50.0));
    assert_eq!(
        rt.state.log,
        ["outer Accepting", "outer Left", "outer drop"]
    );
}

#[test]
fn a_zone_whose_action_refuses_never_gets_the_drop() {
    let mut rt = pane();
    let mut f = 0;
    drop_at(&mut rt, &mut f, &["/a.txt"], (50.0, 150.0));
    assert_eq!(
        rt.state.log,
        [
            "veto Accepting",
            "veto Left",
            r#"veto rejected Some(["file:///a.txt"])"#
        ],
        "Rune routes with no state, so the zone accepts until the drop, then refuses it"
    );
    rt.state.log.clear();
    drop_at(&mut rt, &mut f, &["/a.txt"], (150.0, 150.0));
    assert_eq!(
        rt.state.log,
        ["quiet-veto Accepting", "quiet-veto Left"],
        "and with no on_reject nothing hears it"
    );
    assert!(rt.state.dropped.is_empty());
}

#[test]
fn a_new_drag_gets_a_new_serial_and_its_drop_shares_the_hovers() {
    let mut rt = pane();
    let mut f = 0;
    drop_at(&mut rt, &mut f, &["/a"], (50.0, 50.0));
    drop_at(&mut rt, &mut f, &["/b"], (50.0, 50.0));
    let o = rt.state.offers.clone();
    // Each drag on the row: Accepting, Left, then the drop.
    let [a_in, a_out, a_drop, b_in, b_out, b_drop] = o.as_slice() else {
        panic!("two hovers and drops on the row: {o:?}");
    };
    assert!(o.iter().all(|(k, ..)| *k == "row"), "{o:?}");
    assert_eq!(
        (a_out.1, a_drop.1),
        (a_in.1, a_in.1),
        "one drag, one serial"
    );
    assert_eq!((b_out.1, b_drop.1), (b_in.1, b_in.1));
    assert_ne!(a_in.1, b_in.1, "the second drag has its own");
    assert_eq!(
        [a_in.2, a_out.2, a_drop.2],
        [false, false, true],
        "Rune hears the paths only at the drop"
    );

    rt.drop_files(files(&["/c"]), Some((50.0, 50.0)));
    step(&mut rt, &mut f);
    let c = rt.state.offers.last().expect("the drop was heard");
    assert!(
        c.1 != a_in.1 && c.1 != b_in.1,
        "a drop no hover came before gets a new serial"
    );
}

#[test]
fn a_drag_moving_to_another_zone_never_reuses_an_earlier_drags_serial() {
    let mut rt = pane();
    let mut f = 0;
    drop_at(&mut rt, &mut f, &["/a"], (50.0, 50.0));
    let first = rt.state.offers[0].1;
    rt.drag_files(files(&["/b"]), Some((50.0, 50.0)));
    step(&mut rt, &mut f);
    rt.drag_files_to(Some((50.0, 250.0)));
    step(&mut rt, &mut f);
    let outer: Vec<u64> = rt
        .state
        .offers
        .iter()
        .filter(|(k, ..)| *k == "outer")
        .map(|o| o.1)
        .collect();
    assert!(
        !outer.is_empty() && !outer.contains(&first),
        "{:?}",
        rt.state.offers
    );
}

// ---- Keyed binders (W1e) ----

thread_local! {
    /// What the keyed binder heard, in the order the view attached it: zone or source, the
    /// key and the scope.
    static HEARD: std::cell::RefCell<Vec<(&'static str, String, Option<String>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

fn heard() -> Vec<(&'static str, String, Option<String>)> {
    HEARD.with(|h| h.borrow().clone())
}

/// A binder that records each key it is given and attaches nothing.
fn recording() -> Dnd<Pane> {
    let note = |what: &'static str, key: &ElementKey| {
        HEARD.with(|h| {
            h.borrow_mut()
                .push((what, key.key().to_owned(), key.scope().map(str::to_owned)))
        });
    };
    Dnd::custom_keyed(
        move |el, key, _| {
            note("zone", key);
            el
        },
        move |el, key, _| {
            note("source", key);
            el
        },
    )
}

fn source() -> DragSource<Pane> {
    DragSource::new(|_: &Pane| None)
}

/// Elements given no key of their own, attached with keys: the zone binder, the source binder,
/// and both without a key (which a keyed binder hears nothing of).
fn keyed_view(_: &Pane, _: &mut Cx<Pane>) -> El {
    let dnd = recording();
    let cell = || el().size(100.0, 100.0);
    let rows = row()
        .key("list")
        .child(dnd.drop_zone_keyed(cell(), "list-bg", logging("bg")))
        .child(dnd.drop_zone_keyed(
            cell(),
            ElementKey::new("row-7").within("list"),
            logging("row"),
        ))
        .child(dnd.drag_source_keyed(cell(), "handle", source()))
        .child(dnd.drop_zone(cell().key("plain"), logging("plain")))
        .child(dnd.drag_source(cell().key("plain-source"), source()));
    col().child(rows)
}

#[test]
fn a_keyed_binder_hears_each_elements_key_and_scope_and_the_element_carries_it() {
    HEARD.with(|h| h.borrow_mut().clear());
    let mut rt = Runtime::new(
        Pane::default(),
        keyed_view,
        (500.0, 100.0),
        Env::new(true, 1.0),
    );
    rt.frame(0.0);
    let expected = [
        ("zone", "list-bg".to_owned(), None),
        ("zone", "row-7".to_owned(), Some("list".to_owned())),
        ("source", "handle".to_owned(), None),
    ];
    let all = heard();
    // The view may build more than once: each build hears the same three, in order.
    assert!(!all.is_empty() && all.len().is_multiple_of(3), "{all:?}");
    for build in all.chunks(3) {
        assert_eq!(
            build, expected,
            "nothing heard for the unkeyed zone and source"
        );
    }
    let keys: Vec<String> = rt.elements().into_iter().filter_map(|e| e.key).collect();
    for k in ["list-bg", "row-7", "handle", "plain", "plain-source"] {
        assert!(keys.iter().any(|e| e == k), "{k} is keyed: {keys:?}");
    }
    let row7 = rt
        .elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("row-7"))
        .expect("row-7");
    assert!(
        row7.path.contains("list/"),
        "inside its scope: {}",
        row7.path
    );
}

/// `pane()`'s view with every zone attached by key through Rune's binder.
fn keyed_rune_view(_: &Pane, _: &mut Cx<Pane>) -> El {
    let dnd = Dnd::<Pane>::rune();
    let follow = logging("row").on_move(|s: &mut Pane, p, _| {
        s.log.push(format!("row move {},{}", p.x, p.y));
    });
    let pane = el().size(400.0, 300.0).child(dnd.drop_zone_keyed(
        el().size(100.0, 100.0),
        ElementKey::new("row").within("outer"),
        follow,
    ));
    dnd.drop_zone_keyed(pane, "outer", logging("outer"))
}

#[test]
fn rune_and_unkeyed_custom_binders_take_keyed_zones_as_before() {
    let mut rt = Runtime::new(
        Pane::default(),
        keyed_rune_view,
        (400.0, 300.0),
        Env::new(true, 1.0),
    );
    rt.frame(0.0);
    let mut f = 0;
    drop_at(&mut rt, &mut f, &["/a"], (50.0, 50.0));
    drop_at(&mut rt, &mut f, &["/b"], (50.0, 250.0));
    assert_eq!(
        rt.state.log,
        [
            "row Accepting",
            "row move 50,50",
            "row Left",
            "row drop",
            "outer Accepting",
            "outer Left",
            "outer drop"
        ]
    );
    // A binder of the host's own without keys hears keyed zones and sources too.
    let count = std::rc::Rc::new(std::cell::Cell::new(0));
    let (c, d) = (count.clone(), count.clone());
    let dnd = Dnd::<Pane>::custom(
        move |el, _| {
            c.set(c.get() + 1);
            el
        },
        move |el, _| {
            d.set(d.get() + 10);
            el
        },
    );
    let _ = dnd.drop_zone_keyed(el(), "z", logging("z"));
    let _ = dnd.drag_source_keyed(el(), "s", source());
    assert_eq!(count.get(), 11);
}
