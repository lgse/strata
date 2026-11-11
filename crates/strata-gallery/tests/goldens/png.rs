//! Each golden scene rendered headless (`--snap --scale 1`) against
//! `tests/golden/png/<id>/<scene>.png`.
//!
//! A pixel is off when its largest channel differs by more than [`CHANNEL_TOLERANCE`], the
//! "visible" threshold of rune-kit's gallery diff (`scripts/kit-diff.py` at the pinned rev).
//! No off pixel is allowed. A failure writes the fresh render and an amplified difference
//! image next to each other for review.

use std::collections::BTreeSet;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use crate::support::{self, Mode};

/// The largest channel difference, out of 255, that still counts as the same pixel.
const CHANNEL_TOLERANCE: u8 = 24;
/// How many off pixels a scene may have.
const MAX_OFF_PIXELS: usize = 0;
const SCALE: &str = "1";

pub fn check(id: &str) {
    let page = support::page(id);
    let mut failures = Vec::new();
    for &(name, scene) in page.scenes {
        let mode = support::mode(id, name);
        if mode == Mode::Skip {
            continue;
        }
        if let Err(e) = one(id, name, support::is_dark(scene), mode) {
            failures.push(format!("png {name}: {e}"));
        }
    }
    let names: BTreeSet<String> = page.scenes.iter().map(|(n, _)| (*n).to_owned()).collect();
    support::orphans("png", id, "png", &names, &mut failures);
    support::report(id, "png", failures);
}

#[expect(
    clippy::absurd_extreme_comparisons,
    reason = "MAX_OFF_PIXELS is a tunable that is 0 today"
)]
fn one(id: &str, name: &str, dark: bool, mode: Mode) -> Result<(), String> {
    let golden = support::golden_dir("png", id).join(format!("{name}.png"));
    let fresh = support::fresh_dir("png", id).join(format!("{name}.png"));
    std::fs::create_dir_all(fresh.parent().expect("fresh directory")).expect("fresh directory");
    let mut cmd = support::gallery();
    cmd.arg("--snap").arg(&fresh).args([
        "--scene",
        name,
        "--scale",
        SCALE,
        "--at",
        &support::at_arg(),
    ]);
    cmd.args(["--theme", if dark { "dark" } else { "light" }]);
    let stderr = support::run(cmd)?;
    // Blessing keeps a golden that still matches within the tolerance, so a render on another
    // adapter doesn't churn every PNG.
    let bless = || {
        let bytes = std::fs::read(&fresh).map_err(|e| format!("no render: {e}"))?;
        support::write(&golden, &bytes);
        Ok(())
    };
    if !golden.exists() {
        return match mode {
            Mode::Bless => bless(),
            _ => Err(format!("missing golden; run `mise run bless {id}`")),
        };
    }
    let want = decode(&golden)?;
    let got = decode(&fresh)?;
    let same_size = (want.0, want.1) == (got.0, got.1);
    let (off, heat) = if same_size {
        compare(&want.2, &got.2)
    } else {
        (usize::MAX, Vec::new())
    };
    if off <= MAX_OFF_PIXELS {
        return Ok(());
    }
    if mode == Mode::Bless {
        return bless();
    }
    if !same_size {
        return Err(format!(
            "size {}×{} differs from the golden's {}×{}\n  fresh: {}",
            got.0,
            got.1,
            want.0,
            want.1,
            fresh.display()
        ));
    }
    let diff = fresh.with_file_name(format!("{name}-diff.png"));
    rune::render::write_png(&diff, got.0, got.1, &heat).map_err(|e| e.to_string())?;
    Err(format!(
        "{off} pixel(s) differ by more than {CHANNEL_TOLERANCE}/255 (rendered on {})\n  \
         golden: {}\n  fresh:  {}\n  diff:   {}",
        support::gpu_line(&stderr),
        golden.display(),
        fresh.display(),
        diff.display(),
    ))
}

/// The number of off pixels, and a heat map: each pixel's largest channel difference ×4, in
/// red on black.
fn compare(want: &[u8], got: &[u8]) -> (usize, Vec<u8>) {
    let mut off = 0;
    let mut heat = Vec::with_capacity(got.len());
    for (a, b) in want.as_chunks::<4>().0.iter().zip(got.as_chunks::<4>().0) {
        let d = a
            .iter()
            .zip(b)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        if d > CHANNEL_TOLERANCE {
            off += 1;
        }
        heat.extend_from_slice(&[d.saturating_mul(4), 0, 0, 255]);
    }
    (off, heat)
}

/// A PNG file as 8-bit RGBA.
fn decode(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let err = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
    let file = File::open(path).map_err(|e| err(&e))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| err(&e))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| err(&"image too large"))?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(|e| err(&e))?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|&[v, a]| [v, v, v, a])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&v| [v, v, v, 255]).collect(),
        png::ColorType::Indexed => return Err(err(&"indexed colour was not expanded")),
    };
    Ok((info.width, info.height, rgba))
}

/// The PNG run and the in-process mount end on the same clock, so the open page's `poll` has run
/// as often for the PNG golden as for the a11y golden. `--tree` takes the same timeline as
/// `--snap`.
#[test]
fn the_png_render_ends_on_the_mounts_clock() {
    let scene = support::page("gallery").scenes[0];
    let mount_clock = support::mount(scene.1).t;
    let tree = support::fresh_dir("png", "clock").join("tree.json");
    std::fs::create_dir_all(tree.parent().expect("tree directory")).expect("tree directory");
    let mut cmd = support::gallery();
    cmd.args([
        "--scene",
        scene.0,
        "--at",
        &support::at_arg(),
        "--scale",
        SCALE,
    ]);
    cmd.arg("--tree").arg(&tree);
    support::run(cmd).expect("the gallery renders headless");
    let text = std::fs::read_to_string(&tree).expect("tree written");
    let snap: serde_json::Value = serde_json::from_str(&text).expect("tree parses");
    let snap_clock = snap["anim_t"].as_f64().expect("anim_t in the tree");
    // The two sums of 1/60 round differently in the last bits; a frame more or less on either
    // side would put them 1/60 apart.
    let same_frame = |a: f64, b: f64| (a - b).abs() < 1e-9;
    assert!(
        same_frame(snap_clock, mount_clock),
        "--at {} ends at {snap_clock}, the mount at {mount_clock}",
        support::AT
    );
    assert!(
        same_frame(mount_clock, support::AT),
        "the mount ends at {mount_clock}"
    );
    assert_eq!(snap["t"].as_f64(), Some(support::AT));
}
