// SPDX-License-Identifier: MIT

//! Resolves a target to a node, its box on the screen and the point to aim at.

use std::collections::VecDeque;

use super::{
    fields::{field, focused_field},
    lookup::{
        find, first_rendered, missing, open_dialog, other_window, presented, rendered, result_rows,
        search_result, search_results, windows_titled,
    },
    scan::{PaneNodes, Scene, scan},
    wanted::{Wanted, pick},
};
use crate::{
    Aim, At, Axis, Backend, Boxes, EntryPart, FieldRef, LookupError, Mark, Memory, Rect, Target,
};

#[derive(Clone, Debug)]
pub struct Located<Node> {
    pub node: Node,
    pub rect: Rect,
    pub point: (f64, f64),
}

/// What earlier steps kept under a name: fields and other nodes, and boxes.
pub struct Kept<'a, B: Backend> {
    pub fields: &'a Memory<B>,
    pub nodes: &'a Memory<B>,
    pub boxes: &'a Boxes,
}

impl<B: Backend> Clone for Kept<'_, B> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<B: Backend> Copy for Kept<'_, B> {}

/// The node's box on the screen: its frame, moved by where its surface is.
pub fn screen_rect<B: Backend + ?Sized>(backend: &B, node: &B::Node) -> Result<Rect, LookupError> {
    let frame = backend
        .frame(node)
        .ok_or_else(|| missing("the target is not on screen"))?;
    let (x, y) = backend.origin(node).map_err(LookupError::Broken)?;
    Ok(Rect {
        x: x + frame.x,
        y: y + frame.y,
        width: frame.width,
        height: frame.height,
    })
}

fn icon<B: Backend>(backend: &B, node: &B::Node) -> Option<B::Node> {
    presented(backend, node)
        .into_iter()
        .find(|node| backend.role(node) == "image" && rendered(backend, node))
}

fn ancestors<'a, B: Backend>(backend: &'a B, node: &B::Node) -> impl Iterator<Item = B::Node> + 'a {
    std::iter::successors(backend.parent(node), |above| backend.parent(above))
}

/// The scroll pane that scrolls a pane's entries. A list that holds its own (a Rune
/// collection keeps its scroll viewport inside the node with the list's role) is scrolled by
/// the one inside it nearest it, even inside another that scrolls sideways (Rune's List
/// table); any other list by the nearest one around it, as GTK's is by its scrolled window. A
/// scroll pane inside an entry is never the pane's.
pub(crate) fn viewport<B: Backend>(backend: &B, container: &B::Node) -> Option<B::Node> {
    let scrolls = |node: &B::Node| backend.role(node) == "scroll pane";
    inside_list(backend, container, scrolls)
        .or_else(|| ancestors(backend, container).find(|node| scrolls(node)))
}

/// The node under `container` nearest it that `wanted` takes, the first in order among
/// those as near, without looking inside an entry.
fn inside_list<B: Backend>(
    backend: &B,
    container: &B::Node,
    wanted: impl Fn(&B::Node) -> bool,
) -> Option<B::Node> {
    let mut queue: VecDeque<B::Node> = backend.children(container).into();
    while let Some(node) = queue.pop_front() {
        if backend.mark(&node) == Some(Mark::Entry) {
            continue;
        }
        if wanted(&node) {
            return Some(node);
        }
        queue.extend(backend.children(&node));
    }
    None
}

pub fn locate<B: Backend>(
    backend: &B,
    window: &B::Node,
    kept: Kept<'_, B>,
    target: &Target,
    at: Option<&At>,
    aim: Aim,
) -> Result<Located<B::Node>, LookupError> {
    let scene = scan(backend, window);
    let root = window;
    let within = |target: &Target| locate(backend, window, kept, target, None, Aim::Point);
    let node = match target {
        Target::Entry(entry) => {
            let row = scene
                .entry(&entry.entry, entry.dir.as_deref())
                .cloned()
                .ok_or_else(|| {
                    missing(format!(
                        "no entry {:?}{} on screen",
                        entry.entry,
                        entry
                            .dir
                            .as_deref()
                            .map(|dir| format!(" in {dir:?}"))
                            .unwrap_or_default()
                    ))
                })?;
            match entry.part {
                Some(EntryPart::Label) => first_rendered(backend, &row, |node| {
                    backend.role(node) == "label" && backend.name(node) == entry.entry
                })
                .ok_or_else(|| missing(format!("no name label on {:?}", entry.entry)))?,
                Some(EntryPart::Icon) => icon(backend, &row)
                    .ok_or_else(|| missing(format!("no icon on {:?}", entry.entry)))?,
                Some(EntryPart::Metadata) => first_rendered(backend, &row, |node| {
                    backend.role(node) == "label" && {
                        let name = backend.name(node);
                        !name.is_empty() && name != entry.entry
                    }
                })
                .ok_or_else(|| missing(format!("no metadata label on {:?}", entry.entry)))?,
                None if aim == Aim::DragOrigin => return drag_origin(backend, row),
                None => row,
            }
        }
        Target::SearchResult(name) => {
            let everything = presented(backend, root);
            let label = backend.labels().search_results;
            let result = match search_results(backend, &everything, &label) {
                Some((results, row_role)) => find(backend, &results, row_role, name),
                None => find(backend, root, "list item", name),
            }?;
            if aim == Aim::DragOrigin {
                return drag_origin(backend, result);
            }
            result
        }
        Target::Role {
            within: scope,
            rendered,
            ..
        } => {
            let wanted = Wanted::of(target)?;
            let scope = match scope {
                None => root.clone(),
                Some(scope) => within(scope)?.node,
            };
            if *rendered {
                wanted.find(backend, &scope)?
            } else {
                wanted
                    .find_presented(backend, &scope)
                    .ok_or_else(|| missing(format!("no {} presented", wanted.role)))?
            }
        }
        Target::Row(name) => ["list item", "table cell"]
            .into_iter()
            .find_map(|role| find(backend, root, role, name).ok())
            .ok_or_else(|| missing(format!("no row named {name:?} on screen")))?,
        Target::FieldWithin(scope) => {
            let scope = within(scope)?.node;
            focused_field(backend, &presented(backend, &scope))
                .ok_or_else(|| missing("no editable field inside the target has focus"))?
        }
        Target::Toplevel(title) => windows_titled(backend, title)
            .into_iter()
            .next()
            .ok_or_else(|| missing(format!("no window titled {title:?} on screen")))?,
        Target::OtherWindow(title) => other_window(backend, window, title)
            .ok_or_else(|| missing(format!("no other window titled {title:?} on screen")))?,
        Target::Named {
            name,
            within: scope,
        } => {
            let scope = within(scope)?.node;
            first_rendered(backend, &scope, |node| backend.name(node) == *name)
                .ok_or_else(|| missing(format!("nothing named {name:?} on screen")))?
        }
        Target::Container { dir } => pane(&scene, dir.as_deref())?
            .container
            .clone()
            .ok_or_else(|| missing("the pane has no entry list"))?,
        Target::Dialog => open_dialog(backend, &presented(backend, root))
            .ok_or_else(|| missing("no dialog is open"))?,
        Target::Parent(of) => backend
            .parent(&within(of)?.node)
            .ok_or_else(|| missing("the target has no parent"))?,
        Target::Ancestor { role, of } => {
            let child = within(of)?.node;
            ancestors(backend, &child)
                .find(|node| backend.role(node) == role)
                .ok_or_else(|| missing(format!("the target has no {role} around it")))?
        }
        Target::Kept(name) => {
            let rect = kept.boxes.get(name).copied().ok_or_else(|| {
                LookupError::Broken(format!("no box or point was kept as {name:?}"))
            })?;
            return Ok(Located {
                node: root.clone(),
                rect,
                point: rect.point(at),
            });
        }
        Target::Point(axes) => {
            let [x, y] = &**axes;
            let rect = Rect {
                x: coordinate(backend, window, kept, x, Along::X)? as f64,
                y: coordinate(backend, window, kept, y, Along::Y)? as f64,
                width: 0.0,
                height: 0.0,
            };
            return Ok(Located {
                node: root.clone(),
                rect,
                point: rect.point(at),
            });
        }
        Target::Sidebar(name) | Target::Header(name) => find(backend, root, "button", name)?,
        Target::MenuItem(label) => {
            let menu = first_rendered(backend, root, |node| backend.role(node) == "menu")
                .ok_or_else(|| missing("no menu is open"))?;
            find(backend, &menu, "menu item", label)?
        }
        Target::DialogButton(label) => {
            let dialog = first_rendered(backend, root, |node| backend.role(node) == "dialog")
                .or_else(|| first_rendered(backend, root, |node| backend.role(node) == "alert"))
                .ok_or_else(|| missing("no dialog is open"))?;
            find(backend, &dialog, "button", label)?
        }
        Target::Field(remembered) => field(
            backend,
            window,
            kept.fields,
            &FieldRef {
                of: remembered.clone(),
                name: None,
            },
        )?
        .ok_or_else(|| {
            missing(match remembered {
                None => "no editable field has focus".to_owned(),
                Some(name) => format!("the field remembered as {name:?} is gone"),
            })
        })?,
        Target::Result(path) => search_result(backend, root, path)
            .ok_or_else(|| missing(format!("no result {path:?} on screen")))?,
        Target::ResultRow {
            index,
            containing,
            name: named,
        } => {
            let rows: Vec<B::Node> = result_rows(backend, root)
                .into_iter()
                .filter(|row| {
                    let name = backend.name(row);
                    containing.as_deref().is_none_or(|part| name.contains(part))
                        && named.as_deref().is_none_or(|named| name == named)
                })
                .collect();
            pick(rows, *index).ok_or_else(|| missing(format!("no result row {index}")))?
        }
        Target::FocusedWidget => {
            focused(backend, root).ok_or_else(|| missing("no widget has focus"))?
        }
        Target::Remembered(name) => {
            let remembered = kept.nodes.get(name).ok_or_else(|| {
                LookupError::Broken(format!("no widget was remembered as {name:?}"))
            })?;
            backend
                .recall(remembered)
                .ok_or_else(|| missing(format!("the widget remembered as {name:?} is gone")))?
        }
        Target::Pane { dir } => {
            let pane = pane(&scene, dir.as_deref())?;
            let rect = screen_rect(backend, &pane.pane)?;
            // `empty_point`: inside the pane, below its last entry.
            let point = at.map_or(
                (
                    rect.x + (rect.width / 2.0).floor(),
                    rect.y + rect.height - 14.0,
                ),
                |at| rect.point(Some(at)),
            );
            return Ok(Located {
                node: pane.pane.clone(),
                rect,
                point,
            });
        }
        Target::Background { dir } => return background(backend, &scene, dir.as_deref()),
        Target::Window { y_of } => {
            let rect = screen_rect(backend, root)?;
            let (x, mut y) = rect.point(at);
            if let Some(other) = y_of {
                y = within(other)?.rect.centre().1;
            }
            return Ok(Located {
                node: root.clone(),
                rect,
                point: (x, y),
            });
        }
        Target::Preview => {
            let label = backend.labels().preview;
            first_rendered(backend, root, |node| backend.name(node) == label)
                .ok_or_else(|| missing("the preview is not open"))?
        }
        Target::Viewport { dir } => {
            let container = pane(&scene, dir.as_deref())?
                .container
                .clone()
                .ok_or_else(|| missing("the pane has no entry list"))?;
            viewport(backend, &container)
                .ok_or_else(|| missing("the entry list has no scroll viewport"))?
        }
    };
    if aim == Aim::DragOrigin {
        return drag_origin(backend, node);
    }
    let rect = screen_rect(backend, &node)?;
    Ok(Located {
        rect,
        point: rect.point(at),
        node,
    })
}

/// The first drawn node, in tree order, that reports itself focused.
pub fn focused<B: Backend>(backend: &B, window: &B::Node) -> Option<B::Node> {
    presented(backend, window)
        .into_iter()
        .find(|node| backend.states(node).focused && rendered(backend, node))
}

/// A file drag starts on the row's icon, not on the inert space in a wide row.
fn drag_origin<B: Backend>(backend: &B, row: B::Node) -> Result<Located<B::Node>, LookupError> {
    let rect = screen_rect(backend, icon(backend, &row).as_ref().unwrap_or(&row))?;
    Ok(Located {
        node: row,
        rect,
        point: rect.centre(),
    })
}

#[derive(Clone, Copy)]
enum Along {
    X,
    Y,
}

/// Whole pixels, as AT-SPI reports a box, so the arithmetic is integer arithmetic.
fn coordinate<B: Backend>(
    backend: &B,
    window: &B::Node,
    kept: Kept<'_, B>,
    axis: &Axis,
    along: Along,
) -> Result<i64, LookupError> {
    let pixels = |target: &Target| {
        locate(backend, window, kept, target, None, Aim::Point).map(|located| located.rect.pixels())
    };
    Ok(match axis {
        Axis::Of { target, at, plus } => {
            let [x, y, width, height] = pixels(target)?;
            let (start, extent) = match along {
                Along::X => (x, width),
                Along::Y => (y, height),
            };
            let value = match at {
                None => start + extent.div_euclid(2),
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "an offset from whole pixels is a whole pixel"
                )]
                Some(at) => at.resolve(start as f64, extent as f64).floor() as i64,
            };
            value + i64::from(*plus)
        }
        Axis::Mid { pair, plus } => {
            let [first, second] = &**pair;
            (coordinate(backend, window, kept, first, along)?
                + coordinate(backend, window, kept, second, along)?)
            .div_euclid(2)
                + i64::from(*plus)
        }
        Axis::Extent { target, plus } => {
            let [_, _, width, height] = pixels(target)?;
            let extent = match along {
                Along::X => width,
                Along::Y => height,
            };
            extent + i64::from(*plus)
        }
        Axis::Max { pair, plus } => {
            let [first, second] = &**pair;
            coordinate(backend, window, kept, first, along)?
                .max(coordinate(backend, window, kept, second, along)?)
                + i64::from(*plus)
        }
    })
}

fn pane<'a, Node>(
    scene: &'a Scene<Node>,
    directory: Option<&str>,
) -> Result<&'a PaneNodes<Node>, LookupError> {
    scene.pane(directory).ok_or_else(|| {
        missing(format!(
            "no {} pane on screen",
            directory.map_or_else(|| "browser".to_owned(), |dir| format!("{dir:?}"))
        ))
    })
}

/// `background_point`: on the entry list itself, below the last entry.
fn background<B: Backend>(
    backend: &B,
    scene: &Scene<B::Node>,
    directory: Option<&str>,
) -> Result<Located<B::Node>, LookupError> {
    let nodes = pane(scene, directory)?;
    let Some(container) = nodes.container.as_ref() else {
        let rect = screen_rect(backend, &nodes.pane)?;
        return Ok(Located {
            node: nodes.pane.clone(),
            rect,
            point: rect.centre(),
        });
    };
    let rect = screen_rect(backend, container)?;
    let mut lowest = rect.y;
    let mut any = false;
    for entry in &nodes.entries {
        let entry = screen_rect(backend, entry)?;
        let bottom = entry.y.floor() + entry.height.floor();
        lowest = if any { lowest.max(bottom) } else { bottom };
        any = true;
    }
    let y = (rect.y + rect.height - 6.0).min((lowest + 12.0).max(rect.y + 6.0));
    Ok(Located {
        node: container.clone(),
        rect,
        point: (rect.x + (rect.width / 2.0).floor(), y),
    })
}

/// A role target that need not be drawn: the match its `nth` picks among presented nodes.
pub fn presented_target<B: Backend>(
    backend: &B,
    window: &B::Node,
    kept: Kept<'_, B>,
    target: &Target,
) -> Result<Option<B::Node>, LookupError> {
    match target {
        Target::Role { within, .. } => {
            let wanted = Wanted::of(target)?;
            let scope = match within {
                None => window.clone(),
                Some(within) => match locate(backend, window, kept, within, None, Aim::Point) {
                    Ok(located) => located.node,
                    Err(LookupError::Missing(_)) => return Ok(None),
                    Err(broken) => return Err(broken),
                },
            };
            Ok(wanted.find_presented(backend, &scope))
        }
        other => match locate(backend, window, kept, other, None, Aim::Point) {
            Ok(located) => Ok(Some(located.node)),
            Err(LookupError::Missing(_)) => Ok(None),
            Err(broken) => Err(broken),
        },
    }
}
