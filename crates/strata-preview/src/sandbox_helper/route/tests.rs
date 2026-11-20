// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn gpx_preserves_gaps_and_missing_zero_and_negative_altitude() {
    let route = parse(br#"<gpx><trk><trkseg><trkpt lat="0" lon="0"><ele>-5</ele></trkpt><trkpt lat="0" lon="0.001"><ele>0</ele></trkpt><trkpt lat="NaN" lon="0"/><trkpt lat="1" lon="1"/></trkseg><trkseg><trkpt lat="2" lon="2"/></trkseg></trk></gpx>"#, RouteFormat::Gpx).expect("parse GPX fixture");
    assert_eq!(
        route.segments.iter().map(Vec::len).collect::<Vec<_>>(),
        [2, 1, 1]
    );
    assert_eq!(route.segments[0][0].elevation, Some(-5.0));
    assert_eq!(route.segments[0][1].elevation, Some(0.0));
    assert_eq!(route.segments[1][0].elevation, None);
    let preview = render::draw(&route, ModelPalette::default()).expect("render route");
    assert!((preview.distance_m - 111.195).abs() < 0.01);
    assert_eq!(preview.elevation, Some([-5.0, 0.0]));
    for (sample, expected) in preview.samples.iter().zip([-5.0, 0.0]) {
        let height = crate::services::route_preview::profile_elevation(
            sample.elevation_y.expect("recorded height"),
            preview.elevation.expect("elevation range"),
        );
        assert!((height - expected).abs() < 0.001);
    }
    assert!(
        RoutePreview::from_json(&serde_json::to_vec(&preview).expect("serialize preview")).is_ok()
    );
}

#[test]
fn waypoints_and_flat_or_missing_altitude_remain_useful() {
    for (xml, profile) in [
        (r#"<gpx><wpt lat="51" lon="0"/></gpx>"#, false),
        (
            r#"<gpx><rte><rtept lat="51" lon="0"/><rtept lat="51" lon="1"/></rte></gpx>"#,
            false,
        ),
        (
            r#"<gpx><rte><rtept lat="51" lon="0"><ele>0</ele></rtept><rtept lat="51" lon="1"><ele>0</ele></rtept></rte></gpx>"#,
            true,
        ),
    ] {
        let route = parse(xml.as_bytes(), RouteFormat::Gpx).expect("parse GPX fixture");
        let preview = render::draw(&route, ModelPalette::default()).expect("render route");
        assert_eq!(preview.profile.is_some(), profile);
        assert!(preview.distance_m.is_finite());
        assert!(!preview.png.is_empty());
    }
}

#[test]
fn xml_rejects_truncation_wrong_root_dtd_and_excessive_depth() {
    for xml in [
        "<gpx><trk>",
        "<html/>",
        "<!DOCTYPE gpx><gpx/>",
        "<gpx/><gpx/>",
    ] {
        assert!(parse(xml.as_bytes(), RouteFormat::Gpx).is_err(), "{xml}");
    }
    let deep = format!("<gpx>{}{}</gpx>", "<x>".repeat(65), "</x>".repeat(65));
    assert!(parse(deep.as_bytes(), RouteFormat::Gpx).is_err());
}

#[test]
fn kml_keeps_lines_separate_and_requires_absolute_altitude() {
    let route = parse(br#"<kml xmlns:gx="http://www.google.com/kml/ext/2.2"><Document><Placemark><MultiGeometry><LineString><altitudeMode>absolute</altitudeMode><coordinates>0,0,10 0.001,0,20</coordinates></LineString><LineString><coordinates>1,1,100 2,2,200</coordinates></LineString><gx:Track><altitudeMode>absolute</altitudeMode><gx:coord>3 3 30</gx:coord><gx:coord/><gx:coord>4 4 40</gx:coord></gx:Track></MultiGeometry></Placemark></Document></kml>"#, RouteFormat::Kml).expect("parse KML fixture");
    assert_eq!(
        route.segments.iter().map(Vec::len).collect::<Vec<_>>(),
        [2, 2, 1, 1]
    );
    assert_eq!(route.segments[0][0].elevation, Some(10.0));
    assert_eq!(route.segments[1][0].elevation, None);
    assert_eq!(route.segments[2][0].elevation, Some(30.0));
}

#[test]
fn dateline_distance_uses_the_short_arc() {
    let route = parse(
        br#"<gpx><rte><rtept lat="0" lon="179.999"/><rtept lat="0" lon="-179.999"/></rte></gpx>"#,
        RouteFormat::Gpx,
    )
    .expect("parse dateline route");
    let preview = render::draw(&route, ModelPalette::default()).expect("render route");
    assert!((preview.distance_m - 222.39).abs() < 0.1);
}

fn fit_fixture() -> Vec<u8> {
    let mut data = vec![
        0x40, 0, 0, 20, 0, 4, 0, 4, 0x85, 1, 4, 0x85, 2, 2, 0x84, 78, 4, 0x86,
    ];
    for (lat, lon, altitude, enhanced) in [
        (0i32, 0i32, 2500u16, 2475u32),
        (0, 11930, 2600, u32::MAX),
        (i32::MAX, i32::MAX, u16::MAX, u32::MAX),
        (10000, 10000, u16::MAX, u32::MAX),
    ] {
        data.push(0);
        data.extend(lat.to_le_bytes());
        data.extend(lon.to_le_bytes());
        data.extend(altitude.to_le_bytes());
        data.extend(enhanced.to_le_bytes());
    }
    let mut bytes = vec![12, 0x20, 0, 0];
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(b".FIT");
    bytes.extend(data);
    append_fit_crc(&mut bytes);
    bytes
}

fn append_fit_crc(bytes: &mut Vec<u8>) {
    let mut crc = 0u16;
    for byte in bytes.iter() {
        crc ^= u16::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xa001
            } else {
                crc >> 1
            };
        }
    }
    bytes.extend(crc.to_le_bytes());
}

#[test]
fn fit_decodes_coordinates_prefers_enhanced_altitude_and_preserves_gaps() {
    let bytes = fit_fixture();
    let route = parse(&bytes, RouteFormat::Fit).expect("parse FIT fixture");
    assert_eq!(
        route.segments.iter().map(Vec::len).collect::<Vec<_>>(),
        [2, 1]
    );
    assert_eq!(route.segments[0][0].elevation, Some(-5.0));
    assert_eq!(route.segments[0][1].elevation, Some(20.0));
    assert_eq!(route.segments[1][0].elevation, None);
    assert!((geometry::distance(route.segments[0][0], route.segments[0][1]) - 111.2).abs() < 0.1);
    let chained = [bytes.as_slice(), bytes.as_slice()].concat();
    assert_eq!(
        parse(&chained, RouteFormat::Fit)
            .expect("parse chained FIT records")
            .segments
            .len(),
        4
    );

    let hex = concat!(
        "0c2034083c0000002e464954",
        "4000000000010001000004",
        "410000140003fd0486000485010485",
        "0180b4f43f0000000000000000",
        "410000140002000485010485",
        "a1000000009a2e0000",
    );
    let mut compressed: Vec<_> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("decode fixture hex"))
        .collect();
    append_fit_crc(&mut compressed);
    let route = parse(&compressed, RouteFormat::Fit).expect("parse compressed FIT records");
    assert_eq!(route.segments.len(), 1);
    assert_eq!(route.points, 2);
    assert_eq!(route.duration_seconds(), Some(1));
    assert!((geometry::prepare(&route).distance_m - 111.2).abs() < 0.1);
}

#[test]
fn fit_rejects_damage_missing_crc_and_truncated_sequences() {
    let bytes = fit_fixture();
    for length in [0, 6, bytes.len() - 2, bytes.len() - 1] {
        assert!(parse(&bytes[..length], RouteFormat::Fit).is_err());
    }
    let mut damaged = bytes.clone();
    damaged[32] ^= 1;
    assert!(parse(&damaged, RouteFormat::Fit).is_err());
    let chained = [bytes.as_slice(), &bytes[..bytes.len() - 1]].concat();
    assert!(parse(&chained, RouteFormat::Fit).is_err());
}

#[test]
fn route_limits_are_aggregate_and_output_metadata_is_validated() {
    let mut route = Route::default();
    for _ in 0..MAX_ROUTE_POINTS {
        route
            .push(Some(Point {
                lat: 0.0,
                lon: 0.0,
                elevation: None,
            }))
            .expect("insert point within limit");
    }
    route.split();
    assert!(
        route
            .push(Some(Point {
                lat: 0.0,
                lon: 0.0,
                elevation: None
            }))
            .is_err()
    );
    let route =
        parse(br#"<gpx><wpt lat="0" lon="0"/></gpx>"#, RouteFormat::Gpx).expect("parse waypoint");
    let mut preview = render::draw(&route, ModelPalette::default()).expect("render waypoint");
    preview.distance_m = -1.0;
    assert!(
        RoutePreview::from_json(&serde_json::to_vec(&preview).expect("serialize invalid preview"))
            .is_err()
    );
    preview.distance_m = 0.0;
    preview.elevation = Some([0.0, 1.0]);
    assert!(
        RoutePreview::from_json(&serde_json::to_vec(&preview).expect("serialize invalid preview"))
            .is_err()
    );
}

#[test]
fn kml_multitrack_inherits_absolute_altitude_even_when_mode_follows_tracks() {
    for children in [
        "<altitudeMode>absolute</altitudeMode><gx:Track><gx:coord>0 0 10</gx:coord><gx:coord>0.01 0 20</gx:coord></gx:Track>",
        "<gx:Track><gx:coord>0 0 10</gx:coord><gx:coord>0.01 0 20</gx:coord></gx:Track><altitudeMode>absolute</altitudeMode>",
    ] {
        let xml = format!(
            "<kml xmlns:gx=\"http://www.google.com/kml/ext/2.2\"><Placemark><gx:MultiTrack>{children}</gx:MultiTrack></Placemark></kml>"
        );
        let route = parse(xml.as_bytes(), RouteFormat::Kml).expect("parse MultiTrack fixture");
        let preview = render::draw(&route, ModelPalette::default()).expect("render route");
        assert_eq!(preview.elevation, Some([10.0, 20.0]));
    }
}

#[test]
fn gpx_extensions_cannot_replace_point_altitude() {
    let xml = br#"<gpx><metadata><name>Ride &amp; walk</name></metadata><wpt lat="1" lon="2"><ele>12</ele><extensions><wpt><ele>900</ele></wpt></extensions></wpt></gpx>"#;
    let route = parse(xml, RouteFormat::Gpx).expect("parse GPX extensions");
    assert_eq!(route.points, 1);
    assert_eq!(route.segments[0][0].elevation, Some(12.0));
    assert!(parse(&[xml.as_slice(), b"junk"].concat(), RouteFormat::Gpx).is_err());
    let route = parse(br#"<gpx><rte><rtept lat="0" lon="0"/><rtept lat="&#48;" lon="&#x30;.001"/><rtept lat="0" lon="0.002"/></rte></gpx>"#, RouteFormat::Gpx).expect("parse escaped coordinates");
    assert_eq!(route.segments.len(), 1);
    assert_eq!(route.points, 3);
    assert!((geometry::prepare(&route).distance_m - 222.39).abs() < 0.01);
}

#[test]
fn projection_preserves_local_direction_across_longitude_normalization() {
    for lon in [165.846129, -0.14971, -179.999, 179.999] {
        let mut route = Route::default();
        for offset in [0.0, 0.001] {
            route
                .push(Some(Point {
                    lat: offset,
                    lon: lon + offset,
                    elevation: None,
                }))
                .expect("insert route point");
        }
        let trace = geometry::prepare(&route);
        let [x0, y0] = trace.samples[0].position;
        let [x1, y1] = trace.samples[1].position;
        assert!(x1 > x0, "eastward motion at {lon}");
        assert!(
            ((y1 - y0) / (x1 - x0) + 1.0).abs() < 0.0001,
            "northeast bearing at {lon}"
        );
    }
}

#[test]
fn interaction_samples_preserve_gaps_and_reject_invalid_renderer_coordinates() {
    let route = parse(br#"<gpx><trk><trkseg><trkpt lat="0" lon="0"><ele>0</ele></trkpt><trkpt lat="0" lon="0.01"><ele>10</ele></trkpt></trkseg><trkseg><trkpt lat="1" lon="1"/></trkseg></trk></gpx>"#, RouteFormat::Gpx).expect("parse segmented route");
    let preview = render::draw(&route, ModelPalette::default()).expect("render route");
    assert_eq!(preview.samples.len(), 3);
    assert!(!preview.samples[1].starts_segment);
    assert!(preview.samples[2].starts_segment);
    assert_eq!(preview.samples[1].distance_m, preview.samples[2].distance_m);
    assert!(preview.samples[2].elevation_y.is_none());
    for invalid in 0..9 {
        let mut changed = preview.clone();
        match invalid {
            0 => changed.samples[0].position[0] = -1.0,
            1 => changed.samples[1].distance_m = -1.0,
            2 => changed.samples[0].starts_segment = false,
            3 => changed.samples[1].elevation_y = Some(141.0),
            4 => {
                changed.samples.pop();
            }
            5 => {
                changed.png = resvg::tiny_skia::Pixmap::new(1, 1)
                    .expect("allocate tiny route image")
                    .encode_png()
                    .expect("encode tiny route image")
            }
            7 => changed.duration_seconds = Some(u64::MAX),
            8 => changed.duration_seconds = Some(0),
            _ => {
                changed.profile = Some(
                    resvg::tiny_skia::Pixmap::new(1, 1)
                        .expect("allocate tiny profile image")
                        .encode_png()
                        .expect("encode tiny profile image"),
                )
            }
        }
        assert!(
            RoutePreview::from_json(
                &serde_json::to_vec(&changed).expect("serialize invalid samples")
            )
            .is_err()
        );
    }
}

#[test]
fn route_elapsed_time_uses_recorded_timestamps_without_requiring_them() {
    for (first, last, expected) in [
        ("2026-10-24T10:00:00Z", "2026-10-24T11:01:02Z", Some(3662)),
        (
            "2026-10-24T10:00:00+01:00",
            "2026-10-24T10:00:30Z",
            Some(3630),
        ),
        ("not a time", "2026-10-24T10:00:30Z", None),
        ("", "", None),
        ("2026-10-24T10:00:00Z", "2026-10-24T10:00:00Z", None),
    ] {
        let gpx = format!(
            r#"<gpx><metadata><time>2000-01-01T00:00:00Z</time></metadata><trk><trkseg><trkpt lat="0" lon="0"><time>{first}</time></trkpt></trkseg><trkseg><trkpt lat="0" lon="1"><time>{last}</time><extensions><time>2099-01-01T00:00:00Z</time></extensions></trkpt></trkseg></trk></gpx>"#
        );
        let kml = format!(
            r#"<kml xmlns:gx="http://www.google.com/kml/ext/2.2"><Placemark><TimeStamp><when>2000-01-01T00:00:00Z</when></TimeStamp><gx:Track><when>{first}</when><when>{last}</when><gx:coord>0 0 0</gx:coord><gx:coord>1 0 0</gx:coord></gx:Track></Placemark></kml>"#
        );
        for (xml, format) in [(gpx, RouteFormat::Gpx), (kml, RouteFormat::Kml)] {
            let route = parse(xml.as_bytes(), format).expect("parse timed route");
            let preview =
                render::draw(&route, ModelPalette::default()).expect("render timed route");
            assert_eq!(
                preview.duration_seconds, expected,
                "{format:?}: {first}, {last}"
            );
            assert_eq!(preview.points, 2);
            assert!(
                RoutePreview::from_json(&serde_json::to_vec(&preview).expect("serialize preview"))
                    .is_ok()
            );
        }
    }
}
