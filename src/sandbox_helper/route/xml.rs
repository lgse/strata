// SPDX-License-Identifier: MIT

use super::{Point, Route, elevation};
use crate::services::route_preview::{INVALID_ROUTE, RouteFormat};
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};

pub(super) fn parse(bytes: &[u8], format: RouteFormat) -> Result<Route, String> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().expand_empty_elements = true;
    let mut stack = Vec::<Vec<u8>>::new();
    let mut route = Route::default();
    let mut point = None;
    let mut text = String::new();
    let mut root_seen = false;
    let mut altitude = Vec::<Altitude>::new();
    let mut geometries = Vec::<usize>::new();
    loop {
        match reader.read_event().map_err(|_| INVALID_ROUTE)? {
            Event::Start(start) => {
                let name = start.local_name().as_ref().to_vec();
                if stack.is_empty() {
                    if root_seen
                        || name
                            != if format == RouteFormat::Gpx {
                                b"gpx".as_slice()
                            } else {
                                b"kml".as_slice()
                            }
                    {
                        return Err(INVALID_ROUTE.into());
                    }
                    root_seen = true;
                }
                if stack.len() >= 64 {
                    return Err(INVALID_ROUTE.into());
                }
                if format == RouteFormat::Gpx {
                    if (name == b"trkseg" && stack.len() == 2 && stack[1] == b"trk")
                        || (matches!(name.as_slice(), b"rte" | b"wpt") && stack.len() == 1)
                    {
                        route.split();
                    }
                    if gpx_point(&stack, &name) {
                        point = attributes(&start, reader.decoder());
                    }
                } else if matches!(
                    name.as_slice(),
                    b"LineString" | b"Point" | b"Track" | b"MultiTrack"
                ) {
                    if altitude.len() >= 250_000 {
                        return Err(INVALID_ROUTE.into());
                    }
                    route.split();
                    altitude.push(Altitude {
                        parent: geometries.last().copied(),
                        absolute: None,
                        start: route.segments.len(),
                        end: route.segments.len(),
                        group: name == b"MultiTrack",
                    });
                    geometries.push(altitude.len() - 1);
                }
                stack.push(name);
                text.clear();
            }
            Event::Text(value) => {
                let value = value.decode().map_err(|_| INVALID_ROUTE)?;
                if stack.is_empty() && !value.trim().is_empty() {
                    return Err(INVALID_ROUTE.into());
                }
                text.push_str(&value);
            }
            Event::CData(value) => text.push_str(&value.decode().map_err(|_| INVALID_ROUTE)?),
            Event::DocType(_) => return Err(INVALID_ROUTE.into()),
            Event::GeneralRef(value) => {
                let name = value.decode().map_err(|_| INVALID_ROUTE)?;
                if let Some(entity) = quick_xml::escape::resolve_predefined_entity(&name) {
                    text.push_str(entity);
                } else if let Some(character) =
                    value.resolve_char_ref().map_err(|_| INVALID_ROUTE)?
                {
                    text.push(character);
                } else {
                    return Err(INVALID_ROUTE.into());
                }
            }
            Event::End(_) => {
                let Some(name) = stack.pop() else {
                    return Err(INVALID_ROUTE.into());
                };
                let parent = stack.last().map(Vec::as_slice);
                if format == RouteFormat::Gpx {
                    if name == b"ele"
                        && stack
                            .split_last()
                            .is_some_and(|(point, parents)| gpx_point(parents, point))
                    {
                        if let Some(point) = point.as_mut() {
                            point.elevation = text.trim().parse().ok().and_then(elevation);
                        }
                    } else if name == b"time"
                        && stack
                            .split_last()
                            .is_some_and(|(point, parents)| gpx_point(parents, point))
                    {
                        record_timestamp(&mut route, &text);
                    } else if gpx_point(&stack, &name) {
                        route.push(point.take())?;
                        if name == b"wpt" {
                            route.split();
                        }
                    }
                } else if name == b"coordinates"
                    && parent.is_some_and(|p| matches!(p, b"LineString" | b"Point"))
                {
                    for tuple in text.split_whitespace() {
                        route.push(coordinate(tuple.split(',')))?;
                        if parent == Some(b"Point") {
                            route.split();
                        }
                    }
                } else if name == b"coord" && parent == Some(b"Track") {
                    route.push(coordinate(text.split_whitespace()))?;
                } else if name == b"when" && parent == Some(b"Track") {
                    record_timestamp(&mut route, &text);
                } else if name == b"altitudeMode"
                    && parent.is_some_and(|p| {
                        matches!(p, b"LineString" | b"Point" | b"Track" | b"MultiTrack")
                    })
                {
                    if let Some(&index) = geometries.last() {
                        altitude[index].absolute = Some(text.trim() == "absolute");
                    }
                } else if matches!(
                    name.as_slice(),
                    b"LineString" | b"Point" | b"Track" | b"MultiTrack"
                ) {
                    let index = geometries.pop().ok_or(INVALID_ROUTE)?;
                    altitude[index].end = route.segments.len();
                    route.split();
                }
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !root_seen || !stack.is_empty() {
        return Err(INVALID_ROUTE.into());
    }
    for (index, geometry) in altitude.iter().enumerate().filter(|(_, g)| !g.group) {
        let mut inherited = Some(index);
        let mut absolute = false;
        while let Some(index) = inherited {
            let context = &altitude[index];
            if let Some(mode) = context.absolute {
                absolute = mode;
                break;
            }
            inherited = context.parent;
        }
        // Terrain-relative KML altitude cannot be resolved offline.
        if !absolute {
            for segment in &mut route.segments[geometry.start..geometry.end] {
                for point in segment {
                    point.elevation = None;
                }
            }
        }
    }
    Ok(route)
}

struct Altitude {
    parent: Option<usize>,
    absolute: Option<bool>,
    start: usize,
    end: usize,
    group: bool,
}

fn gpx_point(stack: &[Vec<u8>], name: &[u8]) -> bool {
    match name {
        b"trkpt" => stack == [b"gpx".as_slice(), b"trk", b"trkseg"],
        b"rtept" => stack == [b"gpx".as_slice(), b"rte"],
        b"wpt" => stack == [b"gpx".as_slice()],
        _ => false,
    }
}

fn attributes(start: &BytesStart<'_>, decoder: quick_xml::encoding::Decoder) -> Option<Point> {
    let mut lat = None;
    let mut lon = None;
    for attribute in start.attributes() {
        let attribute = attribute.ok()?;
        let number = || {
            attribute
                .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, decoder)
                .ok()?
                .trim()
                .parse::<f64>()
                .ok()
        };
        match attribute.key.as_ref() {
            b"lat" => lat = number(),
            b"lon" => lon = number(),
            _ => {}
        }
    }
    Some(Point {
        lat: lat?,
        lon: lon?,
        elevation: None,
    })
}

fn coordinate<'a>(mut values: impl Iterator<Item = &'a str>) -> Option<Point> {
    let lon = values.next()?.trim().parse().ok()?;
    let lat = values.next()?.trim().parse().ok()?;
    let elevation = values
        .next()
        .and_then(|s| s.trim().parse().ok())
        .and_then(elevation);
    if values.next().is_some() {
        return None;
    }
    Some(Point {
        lat,
        lon,
        elevation,
    })
}

fn record_timestamp(route: &mut Route, text: &str) {
    if let Ok(time) = gtk::glib::DateTime::from_iso8601(text.trim(), None) {
        route.record_timestamp(time.to_unix());
    }
}
