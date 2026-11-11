//! C3: a tile cuts its name again when the fonts move (a fallback font loading, the interface
//! font changing), not only when its name or width does. Alone in its own test binary: it moves
//! the process-wide fonts `rune::text::measure` reads (docs/testing.md, K6).

use rune::{Cx, El, Env, Runtime, col};
use strata_components::collection::element::{Collection, CollectionState};
use strata_components::contract::{EntryId, EntryKind, ListEntry, ListSource};
use strata_components::icons_grid::{IconsGrid, IconsGridState, IconsParts};
use strata_kit::look::Look;

struct Host {
    src: ListSource,
    coll: CollectionState,
    grid: IconsGridState,
    look: Look,
}

const NAME: &str = "a-long-name-whose-cut-depends-on-the-font-it-is-measured-in.tar.gz";

fn base(_: &Host) -> Collection<Host> {
    Collection::new(
        "grid",
        |s: &Host| (&s.src, &s.coll),
        |s: &mut Host| (&s.src, &mut s.coll),
        |s: &Host| &s.look,
        |_: &Host| 1,
    )
}

fn view(s: &Host, _: &mut Cx<Host>) -> El {
    let grid = IconsGrid::new(
        "grid",
        base,
        |s: &Host| IconsParts {
            source: &s.src,
            collection: &s.coll,
            grid: &s.grid,
            size: 64.0,
            listing: 1,
            look: &s.look,
            thumbs: None,
        },
        |s: &mut Host| &mut s.grid,
    );
    col().size(600.0, 400.0).child(grid.build(s))
}

fn label(rt: &Runtime<Host>) -> String {
    rt.elements()
        .into_iter()
        .find(|e| e.key.as_deref() == Some("label") && e.path.contains("row-1"))
        .and_then(|e| e.text)
        .expect("the name")
}

#[test]
fn a_tile_cuts_its_name_again_when_the_fonts_move() {
    let host = Host {
        src: ListSource::new(vec![ListEntry::new(
            EntryId::new(1),
            NAME.to_owned(),
            EntryKind::File,
            "",
        )]),
        coll: CollectionState::new(),
        grid: IconsGridState::new(),
        look: Look::fallback(false),
    };
    let mut rt = Runtime::new(host, view, (600.0, 400.0), Env::new(false, 1.0));
    for _ in 0..4 {
        rt.frame(1.0 / 60.0);
    }
    let before = label(&rt);
    assert!(before.contains('…'), "cut in the first font: {before}");
    // Another font set process-wide moves the fonts' epoch, as a font loading does.
    let epoch = rune::text::font_epoch();
    let _text = rune::text::TextSystem::new(strata_kit::fonts::FONTS, strata_kit::fonts::FAMILIES);
    assert_ne!(rune::text::font_epoch(), epoch);
    // Any commit builds the view again: the tile's versions see the new epoch.
    rt.update(|_, _| {});
    for _ in 0..4 {
        rt.frame(1.0 / 60.0);
    }
    let after = label(&rt);
    assert_ne!(after, before, "cut again in the new fonts");
}
