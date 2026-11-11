//! [`Describe`]: what assistive technology reads after a row's name.

use rune::{Cx, El, Env, Runtime};
use strata_kit::look::Look;

use super::a11y_node;
use super::fixture::FastSource;
use crate::collection::element::{Collection, CollectionState, Describe};
use crate::contract::EntrySource;

struct Host {
    src: FastSource,
    coll: CollectionState,
    look: Look,
    /// What every file row is described with, in kB.
    size: u64,
}

fn view(s: &Host, _: &mut Cx<Host>) -> El {
    Collection::new(
        "list",
        |s: &Host| (&s.src as &dyn EntrySource, &s.coll),
        |s: &mut Host| (&s.src as &dyn EntrySource, &mut s.coll),
        |s: &Host| &s.look,
        |_: &Host| 1,
    )
    .describe(Describe::new(
        |s: &Host, _| s.size,
        |s: &Host, rcx| (!rcx.entry.kind.is_folder_like()).then(|| format!("{} kB", s.size)),
    ))
    .build(s)
}

/// The row's description, from its node's JSON text.
fn description(rt: &Runtime<Host>, key: &str) -> Option<String> {
    let node = a11y_node(rt, key).unwrap_or_else(|| panic!("{key}"));
    let (_, rest) = node.split_once(r#""description":""#)?;
    rest.split_once('"').map(|(d, _)| d.to_owned())
}

/// A file row is described after its name and a folder (entry 0) isn't; a new description
/// reaches the tree when its version changes, though the entries stay the same.
#[test]
fn a_row_reads_its_description_and_follows_its_version() {
    let host = Host {
        src: FastSource::new(5),
        coll: CollectionState::new(),
        look: Look::fallback(false),
        size: 12,
    };
    let mut rt = Runtime::new(host, view, (400.0, 300.0), Env::new(false, 1.0));
    for _ in 0..3 {
        rt.frame(1.0 / 60.0);
    }
    assert_eq!(description(&rt, "row-1").as_deref(), Some("12 kB"));
    assert_eq!(description(&rt, "row-0"), None, "a folder has none");
    rt.update(|s, _| s.size = 13);
    rt.frame(1.0 / 60.0);
    assert_eq!(description(&rt, "row-1").as_deref(), Some("13 kB"));
}
