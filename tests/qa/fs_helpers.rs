// SPDX-License-Identifier: MIT

use std::{
    fs::{self, File},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Instant, SystemTime},
};

use serde_json::Value;

use crate::{
    FFMPEG_TIMEOUT, POLL_INTERVAL,
    fs_check::{PngCheck, contained},
    process::{spawn, terminate},
    scenario::{Image, Thumbnail},
    variables::quote_path,
};

pub(crate) fn check_png(path: &Path, check: &PngCheck) -> Result<(), String> {
    if path.extension().is_none_or(|extension| extension != "png") {
        return Err(format!("{} has no .png suffix", path.display()));
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if image::guess_format(&bytes).ok() != Some(image::ImageFormat::Png) {
        return Err("the file is no PNG".to_owned());
    }
    let decoded = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?
        .to_rgb8();
    if decoded.dimensions() != (check.size[0], check.size[1]) {
        return Err(format!("the size is {:?}", decoded.dimensions()));
    }
    let pixel = decoded
        .get_pixel_checked(check.at[0], check.at[1])
        .ok_or("the pixel lies outside the image")?
        .0;
    (0..3)
        .all(|channel| (check.min[channel]..=check.max[channel]).contains(&pixel[channel]))
        .then_some(())
        .ok_or(format!("the pixel is {pixel:?}"))
}

pub(crate) fn glob_matches(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let middle: Vec<&str> = parts.collect();
    let Some((last, between)) = middle.split_last() else {
        return rest.is_empty();
    };
    for part in between {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.len() >= last.len() && rest.ends_with(last)
}

pub(crate) fn png(image: &Image) -> Result<Vec<u8>, String> {
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(image.width, image.height, image::Rgb(image.rgb))
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(png.into_inner())
}

pub(crate) fn percent_decoded(path: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(path.len());
    let mut rest = path.as_bytes();
    while let Some((&byte, after)) = rest.split_first() {
        if byte == b'%' {
            let digits = after
                .get(..2)
                .and_then(|digits| std::str::from_utf8(digits).ok())
                .and_then(|digits| u8::from_str_radix(digits, 16).ok())
                .ok_or_else(|| format!("{path:?} has a broken % escape"))?;
            bytes.push(digits);
            rest = &after[2..];
        } else {
            bytes.push(byte);
            rest = after;
        }
    }
    Ok(bytes)
}

pub(crate) fn cached_thumbnail(cache: &Path, source: &Path) -> PathBuf {
    let uri = file_uri(source);
    let digest = gtk::glib::compute_checksum_for_string(gtk::glib::ChecksumType::Md5, &uri)
        .map(|digest| digest.to_string())
        .unwrap_or_default();
    cache.join(format!("{digest}.png"))
}

fn file_uri(path: &Path) -> String {
    format!("file://{}", quote_path(path.as_os_str().as_encoded_bytes()))
}

pub(crate) fn write_thumbnail(
    thumbnail: &Thumbnail,
    source: &Path,
    roots: &[PathBuf],
) -> Result<(), String> {
    let cache = Path::new(&thumbnail.cache);
    fs::create_dir_all(cache).map_err(|error| format!("{}: {error}", cache.display()))?;
    contained(cache, roots)?;
    contained(&cached_thumbnail(cache, source), roots)?;
    if let Some(mode) = &thumbnail.mode {
        let mode =
            u32::from_str_radix(mode, 8).map_err(|_| format!("mode is octal, not {mode:?}"))?;
        fs::set_permissions(cache, fs::Permissions::from_mode(mode))
            .map_err(|error| error.to_string())?;
    }
    let seconds = fs::metadata(source)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| format!("{}: {error}", source.display()))?
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let width = i32::try_from(thumbnail.width).map_err(|error| error.to_string())?;
    let height = i32::try_from(thumbnail.height).map_err(|error| error.to_string())?;
    let pixbuf =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, width, height)
            .ok_or("cannot make the thumbnail's pixels")?;
    let [red, green, blue] = thumbnail.rgb;
    pixbuf.fill(u32::from_be_bytes([red, green, blue, 0xff]));
    let uri = file_uri(source);
    let bytes = pixbuf
        .save_to_bufferv(
            "png",
            &[
                ("tEXt::Thumb::URI", uri.as_str()),
                ("tEXt::Thumb::MTime", &seconds.to_string()),
            ],
        )
        .map_err(|error| error.to_string())?;
    fs::write(cached_thumbnail(cache, source), bytes).map_err(|error| error.to_string())
}

pub(crate) fn modified_nanos(path: &Path) -> Result<u128, String> {
    Ok(fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| format!("{}: {error}", path.display()))?
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos())
}

pub(crate) fn local_epoch(
    [year, month, day, hour, minute, second]: [i32; 6],
) -> Result<i64, String> {
    gtk::glib::DateTime::from_local(year, month, day, hour, minute, f64::from(second))
        .map(|time| time.to_unix())
        .map_err(|error| error.to_string())
}

pub(crate) fn archive_matches(
    root: &Path,
    members: &[(String, String)],
    names: &[String],
    contents: &[Vec<u8>],
) -> Result<(), String> {
    let expected: Vec<&String> = members.iter().map(|(name, _)| name).collect();
    if names.iter().collect::<Vec<_>>() != expected {
        return Err(format!("the members are {names:?}"));
    }
    for ((name, source), bytes) in members.iter().zip(contents) {
        let wanted = fs::read(root.join(source)).map_err(|error| format!("{source}: {error}"))?;
        if *bytes != wanted {
            return Err(format!("{name} holds other bytes than {source}"));
        }
    }
    Ok(())
}

pub(crate) fn parsed_toml(text: &str) -> Result<Value, String> {
    let table: toml::Table = text
        .parse()
        .map_err(|error| format!("invalid TOML: {error}"))?;
    serde_json::to_value(table).map_err(|error| error.to_string())
}

pub(crate) fn raw_photo() -> Vec<u8> {
    type Tag = (u16, u16, u32, Vec<u8>);
    fn short(tag: u16, value: u16) -> Tag {
        (tag, 3, 1, value.to_le_bytes().to_vec())
    }
    fn long(tag: u16, value: u32) -> Tag {
        (tag, 4, 1, value.to_le_bytes().to_vec())
    }
    fn text(tag: u16, value: &str) -> Tag {
        let mut bytes = value.as_bytes().to_vec();
        bytes.push(0);
        (tag, 2, bytes.len() as u32, bytes)
    }
    fn rational(tag: u16, values: &[(u32, u32)]) -> Tag {
        let bytes = values
            .iter()
            .flat_map(|(numerator, denominator)| {
                [numerator.to_le_bytes(), denominator.to_le_bytes()].concat()
            })
            .collect();
        (tag, 5, values.len() as u32, bytes)
    }
    fn put(data: &mut [u8], at: usize, bytes: &[u8]) {
        data[at..at + bytes.len()].copy_from_slice(bytes);
    }
    fn directory(data: &mut Vec<u8>, mut tags: Vec<Tag>) -> u32 {
        tags.sort();
        let offset = data.len();
        data.extend((tags.len() as u16).to_le_bytes());
        data.extend(vec![0; tags.len() * 12 + 4]);
        for (index, (tag, kind, count, value)) in tags.iter().enumerate() {
            let position = offset + 2 + index * 12;
            put(data, position, &tag.to_le_bytes());
            put(data, position + 2, &kind.to_le_bytes());
            put(data, position + 4, &count.to_le_bytes());
            if value.len() <= 4 {
                put(data, position + 8, value);
            } else {
                let at = data.len() as u32;
                put(data, position + 8, &at.to_le_bytes());
                data.extend(value);
                if data.len() % 2 == 1 {
                    data.push(0);
                }
            }
        }
        offset as u32
    }
    fn pixels(data: &mut Vec<u8>, offset: u32, fields: &[Tag], width: u32, height: u32) {
        let mut tags: Vec<u16> = fields.iter().map(|(tag, ..)| *tag).collect();
        tags.sort_unstable();
        let strip = tags.iter().position(|tag| *tag == 273).unwrap_or_default();
        let at = data.len() as u32;
        put(
            data,
            offset as usize + 2 + strip * 12 + 8,
            &at.to_le_bytes(),
        );
        for y in 0..height {
            for x in 0..width {
                data.extend([(x * 255 / width) as u8, (y * 255 / height) as u8, 100]);
            }
        }
    }
    let mut data = b"II\x2a\0\0\0\0\0".to_vec();
    let exif = directory(
        &mut data,
        vec![
            rational(33434, &[(1, 250)]),
            short(34855, 400),
            rational(37386, &[(50, 1)]),
            text(42036, "Synthetic 50mm lens"),
            long(40962, 600),
            long(40963, 400),
        ],
    );
    let gps = directory(
        &mut data,
        vec![
            text(1, "S"),
            rational(2, &[(12, 1), (30, 1), (0, 1)]),
            text(3, "W"),
            rational(4, &[(45, 1), (15, 1), (0, 1)]),
        ],
    );
    let bits: Tag = (
        258,
        3,
        3,
        [8u16, 8, 8]
            .iter()
            .flat_map(|bits| bits.to_le_bytes())
            .collect(),
    );
    let version: Tag = (50706, 1, 4, vec![1, 4, 0, 0]);
    let (width, height) = (600, 400);
    let tags = vec![
        long(256, width),
        long(257, height),
        bits.clone(),
        short(259, 1),
        short(262, 34892),
        text(271, "Strata"),
        text(272, "Test Camera"),
        long(273, 0),
        short(274, 1),
        short(277, 3),
        long(278, height),
        long(279, width * height * 3),
        long(34665, exif),
        long(34853, gps),
        version.clone(),
        text(50708, "Strata Test Camera"),
    ];
    let root = directory(&mut data, tags.clone());
    pixels(&mut data, root, &tags, width, height);
    let (preview_width, preview_height) = (300, 200);
    let preview_tags = vec![
        long(254, 1),
        long(256, preview_width),
        long(257, preview_height),
        bits,
        short(259, 1),
        short(262, 2),
        text(271, "Strata"),
        text(272, "Test Camera"),
        long(273, 0),
        short(274, 1),
        short(277, 3),
        long(278, preview_height),
        long(279, preview_width * preview_height * 3),
        long(330, root),
        long(34665, exif),
        long(34853, gps),
        version,
        text(50708, "Strata Test Camera"),
    ];
    let preview = directory(&mut data, preview_tags.clone());
    put(&mut data, 4, &preview.to_le_bytes());
    pixels(
        &mut data,
        preview,
        &preview_tags,
        preview_width,
        preview_height,
    );
    data
}

pub(crate) fn pdf_image(image: &Image) -> Result<Vec<u8>, String> {
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 75)
        .encode_image(&image::RgbImage::from_pixel(
            image.width,
            image.height,
            image::Rgb(image.rgb),
        ))
        .map_err(|error| error.to_string())?;
    let (width, height) = (image.width, image.height);
    let contents = format!("q {width} 0 0 {height} 0 0 cm /image Do Q");
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] \
             /Resources << /XObject << /image 4 0 R >> >> /Contents 5 0 R >>"
        )
        .into_bytes(),
    ];
    let mut image_object = format!(
        "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} \
         /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
        jpeg.len()
    )
    .into_bytes();
    image_object.extend(&jpeg);
    image_object.extend(b"\nendstream");
    objects.push(image_object);
    objects.push(
        format!(
            "<< /Length {} >>\nstream\n{contents}\nendstream",
            contents.len()
        )
        .into_bytes(),
    );
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend(format!("{} 0 obj\n", index + 1).into_bytes());
        pdf.extend(object);
        pdf.extend(b"\nendobj\n");
    }
    let table = pdf.len();
    pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).into_bytes());
    for offset in offsets {
        pdf.extend(format!("{offset:010} 00000 n \n").into_bytes());
    }
    pdf.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{table}\n%%EOF\n",
            objects.len() + 1
        )
        .into_bytes(),
    );
    Ok(pdf)
}

pub(crate) fn magick(arguments: &[String], output: &Path) -> Result<(), String> {
    let found = |name: &str| {
        std::env::var_os("PATH")
            .into_iter()
            .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    };
    let program = found("magick")
        .or_else(|| found("convert"))
        .ok_or("the pinned image must provide ImageMagick")?;
    let mut child = spawn(
        Command::new(program)
            .args(arguments)
            .arg(output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped()),
    )?;
    let deadline = Instant::now() + FFMPEG_TIMEOUT;
    loop {
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => {
                let mut errors = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut stderr, &mut errors);
                }
                return Err(format!(
                    "ImageMagick exited with {status}: {}",
                    errors.trim()
                ));
            }
            None if Instant::now() >= deadline => {
                terminate(&mut child);
                return Err(format!(
                    "ImageMagick ran past {} s",
                    FFMPEG_TIMEOUT.as_secs()
                ));
            }
            None => std::thread::sleep(POLL_INTERVAL),
        }
    }
}

pub(crate) fn copy_program(name: &str, path: &Path) -> Result<(), String> {
    let source = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(name))
        .find(|candidate| {
            fs::metadata(candidate).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
        .ok_or_else(|| format!("no program {name:?} on PATH"))?;
    fs::copy(&source, path).map_err(|error| format!("{}: {error}", source.display()))?;
    let metadata = fs::metadata(&source).map_err(|error| error.to_string())?;
    let times = fs::FileTimes::new()
        .set_accessed(metadata.accessed().map_err(|error| error.to_string())?)
        .set_modified(metadata.modified().map_err(|error| error.to_string())?);
    File::options()
        .write(true)
        .open(path)
        .and_then(|file| file.set_times(times))
        .map_err(|error| error.to_string())
}

pub(crate) fn sorted_names(path: &Path) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = fs::read_dir(path)
        .map_err(|error| error.to_string())?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    Ok(names)
}

pub(crate) fn ffmpeg(arguments: &[String], output: &Path) -> Result<(), String> {
    let mut child = spawn(
        Command::new("ffmpeg")
            .args(["-v", "error", "-y"])
            .args(arguments)
            .arg(output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped()),
    )?;
    let deadline = Instant::now() + FFMPEG_TIMEOUT;
    loop {
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => {
                let mut errors = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = std::io::Read::read_to_string(&mut stderr, &mut errors);
                }
                return Err(format!("ffmpeg exited with {status}: {}", errors.trim()));
            }
            None if Instant::now() >= deadline => {
                terminate(&mut child);
                return Err(format!("ffmpeg ran past {} s", FFMPEG_TIMEOUT.as_secs()));
            }
            None => std::thread::sleep(POLL_INTERVAL),
        }
    }
}
