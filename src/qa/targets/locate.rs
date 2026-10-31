// SPDX-License-Identifier: MIT

use gtk::prelude::*;

use super::{
    super::{
        input::Injector,
        protocol::{At, Axis, EntryPart, FieldRef, Target},
        state::{self, Scene},
    },
    Aim, Kept, Located, LookupError, Rect, Wanted, accessible_name, field, focused_field,
    lookup::{find, first_rendered, other_window, search_result, toplevel},
    missing, open_dialog, presented, result_rows, role_name, screen_rect, search_results, states,
    wanted::pick,
};

fn icon(widget: &gtk::Widget) -> Option<gtk::Widget> {
    presented(widget)
        .into_iter()
        .find(|widget| role_name(widget) == "image" && state::rendered_bounds(widget).is_some())
}

pub(crate) fn locate(
    window: &gtk::Window,
    injector: &Injector,
    kept: Kept<'_>,
    target: &Target,
    at: Option<&At>,
    aim: Aim,
) -> Result<Located, LookupError> {
    let memory = kept.memory;
    let scene = state::scan(window);
    let root = window.upcast_ref::<gtk::Widget>();
    let widget = match target {
        Target::Entry(entry) => {
            let row = scene
                .entry_widget(&entry.entry, entry.dir.as_deref())
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
                Some(EntryPart::Label) => first_rendered(&row, |widget| {
                    role_name(widget) == "label" && accessible_name(widget) == entry.entry
                })
                .ok_or_else(|| missing(format!("no name label on {:?}", entry.entry)))?,
                Some(EntryPart::Icon) => {
                    icon(&row).ok_or_else(|| missing(format!("no icon on {:?}", entry.entry)))?
                }
                Some(EntryPart::Metadata) => first_rendered(&row, |widget| {
                    role_name(widget) == "label" && {
                        let name = accessible_name(widget);
                        !name.is_empty() && name != entry.entry
                    }
                })
                .ok_or_else(|| missing(format!("no metadata label on {:?}", entry.entry)))?,
                None if aim == Aim::DragOrigin => return drag_origin(row, injector),
                None => row,
            }
        }
        Target::SearchResult(name) => {
            let everything = presented(root);
            let result = match search_results(&everything) {
                Some((results, row_role)) => find(&results, row_role, name),
                None => find(root, "list item", name),
            }?;
            if aim == Aim::DragOrigin {
                return drag_origin(result, injector);
            }
            result
        }
        Target::Role {
            within, rendered, ..
        } => {
            let wanted = Wanted::of(target)?;
            let scope = match within {
                None => root.clone(),
                Some(within) => locate(window, injector, kept, within, None, Aim::Point)?.widget,
            };
            if *rendered {
                wanted.find(&scope)?
            } else {
                wanted
                    .find_presented(&scope)
                    .ok_or_else(|| missing(format!("no {} presented", wanted.role)))?
            }
        }
        Target::Row(name) => ["list item", "table cell"]
            .into_iter()
            .find_map(|role| find(root, role, name).ok())
            .ok_or_else(|| missing(format!("no row named {name:?} on screen")))?,
        Target::FieldWithin(within) => {
            let scope = locate(window, injector, kept, within, None, Aim::Point)?.widget;
            focused_field(&presented(&scope))
                .ok_or_else(|| missing("no editable field inside the target has focus"))?
        }
        Target::Toplevel(title) => toplevel(title)
            .ok_or_else(|| missing(format!("no window titled {title:?} on screen")))?,
        Target::OtherWindow(title) => other_window(window, title)
            .ok_or_else(|| missing(format!("no other window titled {title:?} on screen")))?,
        Target::Named { name, within } => {
            let scope = locate(window, injector, kept, within, None, Aim::Point)?.widget;
            first_rendered(&scope, |widget| accessible_name(widget) == *name)
                .ok_or_else(|| missing(format!("nothing named {name:?} on screen")))?
        }
        Target::Container { dir } => pane(&scene, dir.as_deref())?
            .container
            .clone()
            .ok_or_else(|| missing("the pane has no entry list"))?,
        Target::Dialog => {
            open_dialog(&presented(root)).ok_or_else(|| missing("no dialog is open"))?
        }
        Target::Parent(of) => locate(window, injector, kept, of, None, Aim::Point)?
            .widget
            .parent()
            .ok_or_else(|| missing("the target has no parent"))?,
        Target::Ancestor { role, of } => {
            let child = locate(window, injector, kept, of, None, Aim::Point)?.widget;
            std::iter::successors(child.parent(), WidgetExt::parent)
                .find(|widget| role_name(widget) == role)
                .ok_or_else(|| missing(format!("the target has no {role} around it")))?
        }
        Target::Kept(name) => {
            let rect = kept.boxes.get(name).copied().ok_or_else(|| {
                LookupError::Broken(format!("no box or point was kept as {name:?}"))
            })?;
            return Ok(Located {
                widget: root.clone(),
                rect,
                point: rect.point(at),
            });
        }
        Target::Point(axes) => {
            let [x, y] = &**axes;
            let rect = Rect {
                x: coordinate(window, injector, kept, x, Along::X)? as f64,
                y: coordinate(window, injector, kept, y, Along::Y)? as f64,
                width: 0.0,
                height: 0.0,
            };
            return Ok(Located {
                widget: root.clone(),
                rect,
                point: rect.point(at),
            });
        }
        Target::Sidebar(name) | Target::Header(name) => find(root, "button", name)?,
        Target::MenuItem(label) => {
            let menu = first_rendered(root, |widget| role_name(widget) == "menu")
                .ok_or_else(|| missing("no menu is open"))?;
            find(&menu, "menu item", label)?
        }
        Target::DialogButton(label) => {
            let dialog = first_rendered(root, |widget| role_name(widget) == "dialog")
                .or_else(|| first_rendered(root, |widget| role_name(widget) == "alert"))
                .ok_or_else(|| missing("no dialog is open"))?;
            find(&dialog, "button", label)?
        }
        Target::Field(remembered) => field(
            window,
            memory,
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
        Target::Result(path) => search_result(root, path)
            .ok_or_else(|| missing(format!("no result {path:?} on screen")))?,
        Target::ResultRow {
            index,
            containing,
            name: named,
        } => {
            let rows: Vec<gtk::Widget> = result_rows(root)
                .into_iter()
                .filter(|row| {
                    let name = accessible_name(row);
                    containing.as_deref().is_none_or(|part| name.contains(part))
                        && named.as_deref().is_none_or(|named| name == named)
                })
                .collect();
            pick(rows, *index).ok_or_else(|| missing(format!("no result row {index}")))?
        }
        Target::FocusedWidget => presented(root)
            .into_iter()
            .find(|widget| states(widget).focused && state::rendered_bounds(widget).is_some())
            .ok_or_else(|| missing("no widget has focus"))?,
        Target::Remembered(name) => kept
            .widgets
            .get(name)
            .ok_or_else(|| LookupError::Broken(format!("no widget was remembered as {name:?}")))?
            .upgrade()
            .ok_or_else(|| missing(format!("the widget remembered as {name:?} is gone")))?,
        Target::Pane { dir } => {
            let pane = pane(&scene, dir.as_deref())?;
            let rect = screen_rect(&pane.pane, injector)?;
            // `empty_point`: inside the pane, below its last entry.
            let point = at.map_or(
                (
                    rect.x + (rect.width / 2.0).floor(),
                    rect.y + rect.height - 14.0,
                ),
                |at| rect.point(Some(at)),
            );
            return Ok(Located {
                widget: pane.pane.clone(),
                rect,
                point,
            });
        }
        Target::Background { dir } => return background(&scene, injector, dir.as_deref()),
        Target::Window { y_of } => {
            let rect = screen_rect(root, injector)?;
            let (x, mut y) = rect.point(at);
            if let Some(other) = y_of {
                y = locate(window, injector, kept, other, None, Aim::Point)?
                    .rect
                    .centre()
                    .1;
            }
            return Ok(Located {
                widget: root.clone(),
                rect,
                point: (x, y),
            });
        }
        Target::Preview => {
            let label = crate::i18n::tr(crate::ui::PREVIEW_LABEL);
            first_rendered(root, |widget| accessible_name(widget) == label)
                .ok_or_else(|| missing("the preview is not open"))?
        }
        Target::Viewport { dir } => {
            let container = pane(&scene, dir.as_deref())?
                .container
                .clone()
                .ok_or_else(|| missing("the pane has no entry list"))?;
            std::iter::successors(container.parent(), WidgetExt::parent)
                .find(|widget| widget.is::<gtk::ScrolledWindow>())
                .ok_or_else(|| missing("the entry list has no scroll viewport"))?
        }
    };
    if aim == Aim::DragOrigin {
        return drag_origin(widget, injector);
    }
    let rect = screen_rect(&widget, injector)?;
    Ok(Located {
        rect,
        point: rect.point(at),
        widget,
    })
}

/// A file drag starts on the row's icon, not on the inert space in a wide row.
fn drag_origin(row: gtk::Widget, injector: &Injector) -> Result<Located, LookupError> {
    let rect = screen_rect(icon(&row).as_ref().unwrap_or(&row), injector)?;
    Ok(Located {
        widget: row,
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
fn coordinate(
    window: &gtk::Window,
    injector: &Injector,
    kept: Kept<'_>,
    axis: &Axis,
    along: Along,
) -> Result<i64, LookupError> {
    Ok(match axis {
        Axis::Of { target, at, plus } => {
            let [x, y, width, height] = locate(window, injector, kept, target, None, Aim::Point)?
                .rect
                .pixels();
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
            (coordinate(window, injector, kept, first, along)?
                + coordinate(window, injector, kept, second, along)?)
            .div_euclid(2)
                + i64::from(*plus)
        }
        Axis::Extent { target, plus } => {
            let [_, _, width, height] = locate(window, injector, kept, target, None, Aim::Point)?
                .rect
                .pixels();
            let extent = match along {
                Along::X => width,
                Along::Y => height,
            };
            extent + i64::from(*plus)
        }
        Axis::Max { pair, plus } => {
            let [first, second] = &**pair;
            coordinate(window, injector, kept, first, along)?
                .max(coordinate(window, injector, kept, second, along)?)
                + i64::from(*plus)
        }
    })
}

fn pane<'a>(
    scene: &'a Scene,
    directory: Option<&str>,
) -> Result<&'a state::PaneWidgets, LookupError> {
    scene.pane_widgets(directory).ok_or_else(|| {
        missing(format!(
            "no {} pane on screen",
            directory.map_or_else(|| "browser".to_owned(), |dir| format!("{dir:?}"))
        ))
    })
}

/// `background_point`: on the entry list itself, below the last entry.
fn background(
    scene: &Scene,
    injector: &Injector,
    directory: Option<&str>,
) -> Result<Located, LookupError> {
    let widgets = pane(scene, directory)?;
    let Some(container) = widgets.container.as_ref() else {
        let rect = screen_rect(&widgets.pane, injector)?;
        return Ok(Located {
            widget: widgets.pane.clone(),
            rect,
            point: rect.centre(),
        });
    };
    let rect = screen_rect(container, injector)?;
    let mut lowest = rect.y;
    let mut any = false;
    for entry in &widgets.entries {
        let entry = screen_rect(entry, injector)?;
        let bottom = entry.y.floor() + entry.height.floor();
        lowest = if any { lowest.max(bottom) } else { bottom };
        any = true;
    }
    let y = (rect.y + rect.height - 6.0).min((lowest + 12.0).max(rect.y + 6.0));
    Ok(Located {
        widget: container.clone(),
        rect,
        point: (rect.x + (rect.width / 2.0).floor(), y),
    })
}

pub(crate) fn presented_target(
    window: &gtk::Window,
    injector: &Injector,
    kept: Kept<'_>,
    target: &Target,
) -> Result<Option<gtk::Widget>, LookupError> {
    match target {
        Target::Role { within, .. } => {
            let wanted = Wanted::of(target)?;
            let scope = match within {
                None => window.clone().upcast::<gtk::Widget>(),
                Some(within) => match locate(window, injector, kept, within, None, Aim::Point) {
                    Ok(located) => located.widget,
                    Err(LookupError::Missing(_)) => return Ok(None),
                    Err(broken) => return Err(broken),
                },
            };
            Ok(wanted.find_presented(&scope))
        }
        other => match locate(window, injector, kept, other, None, Aim::Point) {
            Ok(located) => Ok(Some(located.widget)),
            Err(LookupError::Missing(_)) => Ok(None),
            Err(broken) => Err(broken),
        },
    }
}
