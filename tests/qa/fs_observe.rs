// SPDX-License-Identifier: MIT

use std::{
    fs::{self, File},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde_json::Value;

use crate::{
    REPOSITORY,
    fixture::{expand_directory, expand_name, listing, numbered, pin_times},
    fs_helpers::{
        archive_matches, cached_thumbnail, check_png, copy_program, ffmpeg, glob_matches,
        local_epoch, magick, modified_nanos, parsed_toml, pdf_image, percent_decoded, png,
        raw_photo, sorted_names, write_thumbnail,
    },
    populate,
    scenario::Image,
};

use super::{FsCheck, FsKind, FsState, Mtime, ZipEntry, contained};

impl FsCheck {
    pub(crate) fn observe(
        &self,
        root: &Path,
        fixture_time: SystemTime,
        state: &mut FsState,
    ) -> Result<(), String> {
        let mut path = if self.percent_path {
            root.join(
                <std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(percent_decoded(
                    &self.path,
                )?),
            )
        } else {
            root.join(&self.path)
        };
        if let Some(cache) = &self.cached_in {
            path = cached_thumbnail(Path::new(cache), &path);
        }
        let roots = state.roots.clone();
        let guarded = matches!(
            self.kind()?,
            FsKind::Thumbnail(_)
                | FsKind::Hex(_)
                | FsKind::Base64(_)
                | FsKind::Append(_)
                | FsKind::RawPhoto
                | FsKind::TarGz(_)
                | FsKind::PdfImage(_)
                | FsKind::Magick(_)
                | FsKind::DefaultApp(_)
                | FsKind::Opened(_)
                | FsKind::Toml(_)
                | FsKind::TomlLike(_)
                | FsKind::Json(_)
                | FsKind::JsonSet(_)
                | FsKind::Launches(_)
                | FsKind::ReplaceLines(_)
                | FsKind::Random(_)
                | FsKind::TomlPrefix(_)
                | FsKind::Png(_)
        ) || (self.percent_path && self.acts());
        if guarded {
            contained(&path, &roots)?;
        }
        let listings = &mut state.listings;
        let other = |name: &str| root.join(name);
        match self.kind()? {
            FsKind::Exists(exists) => {
                let actual = path.symlink_metadata().is_ok();
                (actual == exists)
                    .then_some(())
                    .ok_or(format!("exists is {actual}"))
            }
            FsKind::Text(text) => match fs::read_to_string(&path) {
                Ok(actual) if actual == text => Ok(()),
                Ok(actual) => Err(format!("text is {actual:?}")),
                Err(error) => Err(error.to_string()),
            },
            FsKind::Names(names) => {
                let actual = sorted_names(&path)?;
                (actual == names)
                    .then_some(())
                    .ok_or(format!("names are {actual:?}"))
            }
            FsKind::RememberNames(name) => {
                state.names.insert(name.to_owned(), sorted_names(&path)?);
                Ok(())
            }
            FsKind::NamesFrom(name) => {
                let mut expected: Vec<String> = state
                    .names
                    .get(name)
                    .ok_or_else(|| format!("no names were remembered as {name:?}"))?
                    .iter()
                    .filter(|entry| !self.minus.contains(entry))
                    .chain(&self.plus)
                    .cloned()
                    .collect();
                expected.sort();
                expected.dedup();
                let actual = sorted_names(&path)?;
                (actual == expected)
                    .then_some(())
                    .ok_or(format!("names are {actual:?}, not {expected:?}"))
            }
            FsKind::Is(kind) => {
                let holds = match kind {
                    "file" => path.is_file(),
                    "dir" => path.is_dir(),
                    _ => path.is_symlink(),
                };
                holds.then_some(()).ok_or(format!("is not a {kind}"))
            }
            FsKind::Empty(empty) => {
                let actual = match fs::read_dir(&path) {
                    Ok(entries) => entries.count() == 0,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                    Err(error) => return Err(error.to_string()),
                };
                (actual == empty)
                    .then_some(())
                    .ok_or(format!("empty is {actual}"))
            }
            FsKind::Unchanged(name) => {
                let before = listings
                    .get(name)
                    .ok_or_else(|| format!("no listing was remembered as {name:?}"))?;
                let now = listing(&path);
                (now == *before)
                    .then_some(())
                    .ok_or(format!("the listing changed to:\n{now}"))
            }
            FsKind::CopyOf(source) => {
                let read = |path: &Path| {
                    fs::read(path).map_err(|error| format!("{}: {error}", path.display()))
                };
                let (copy, source) = (read(&path)?, read(&other(source))?);
                (path.is_file() && copy == source)
                    .then_some(())
                    .ok_or(format!("the bytes differ: {copy:?}"))
            }
            FsKind::SizeOf(source) => {
                let size = |path: &Path| {
                    fs::metadata(path)
                        .map(|metadata| metadata.len())
                        .map_err(|error| format!("{}: {error}", path.display()))
                };
                let (copy, source) = (size(&path)?, size(&other(source))?);
                (copy == source)
                    .then_some(())
                    .ok_or(format!("the size is {copy}, not {source}"))
            }
            FsKind::Write(text) => {
                let parent = path.parent().unwrap_or(root);
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                for (name, number) in numbered(&name)? {
                    let (target, text) = match number {
                        Some(number) => {
                            (parent.join(&name), text.replace("{}", &number.to_string()))
                        }
                        None => (path.clone(), (*text).to_owned()),
                    };
                    fs::write(target, text).map_err(|error| format!("{name}: {error}"))?;
                }
                Ok(())
            }
            FsKind::Chmod(mode) => fs::set_permissions(&path, fs::Permissions::from_mode(mode))
                .map_err(|error| error.to_string()),
            FsKind::Volume(directory, prefix, files) => {
                let volume = tempfile::Builder::new()
                    .prefix(prefix)
                    .tempdir_in(directory)
                    .map_err(|error| format!("cannot make a directory in {directory}: {error}"))?;
                let device = |path: &Path| {
                    fs::metadata(path)
                        .map(|metadata| std::os::unix::fs::MetadataExt::dev(&metadata))
                        .map_err(|error| format!("{}: {error}", path.display()))
                };
                if device(volume.path())? == device(root)? {
                    return Err(format!("{directory} is on the fixture's own device"));
                }
                for (name, text) in files {
                    fs::write(volume.path().join(name), text).map_err(|error| error.to_string())?;
                }
                std::os::unix::fs::symlink(volume.path(), &path)
                    .map_err(|error| error.to_string())?;
                state.volumes.push(volume);
                Ok(())
            }
            FsKind::Remember(name) => {
                listings.insert(name.to_owned(), listing(&path));
                Ok(())
            }
            FsKind::Symlink(target) => {
                std::os::unix::fs::symlink(target, &path).map_err(|error| error.to_string())
            }
            FsKind::Program(name) => copy_program(name, &path),
            FsKind::RemoveVolume => {
                let target = fs::read_link(&path).map_err(|error| error.to_string())?;
                let index = state
                    .volumes
                    .iter()
                    .position(|volume| volume.path() == target)
                    .ok_or_else(|| {
                        format!("{} links to no volume this run made", path.display())
                    })?;
                state
                    .volumes
                    .remove(index)
                    .close()
                    .map_err(|error| error.to_string())
            }
            FsKind::Has(has) => {
                let text = fs::read_to_string(&path).ok();
                let contains = |part: &String| {
                    text.as_deref()
                        .is_some_and(|text| text.contains(part.as_str()))
                };
                let missing: Vec<&String> =
                    has.contains.iter().filter(|part| !contains(part)).collect();
                let present: Vec<&String> =
                    has.lacks.iter().filter(|part| contains(part)).collect();
                if missing.is_empty() && present.is_empty() {
                    Ok(())
                } else {
                    Err(format!("lacks {missing:?} and has {present:?}"))
                }
            }
            FsKind::NamesWithout(part) => {
                let names = sorted_names(&path)?;
                let found: Vec<&String> = names.iter().filter(|name| name.contains(part)).collect();
                found
                    .is_empty()
                    .then_some(())
                    .ok_or(format!("names {found:?} contain {part:?}"))
            }
            FsKind::CanTrash(expected) => {
                use gtk::gio::prelude::FileExt as _;
                let actual = gtk::gio::File::for_path(&path)
                    .query_info(
                        gtk::gio::FILE_ATTRIBUTE_ACCESS_CAN_TRASH,
                        gtk::gio::FileQueryInfoFlags::NONE,
                        gtk::gio::Cancellable::NONE,
                    )
                    .map_err(|error| error.to_string())?
                    .boolean(gtk::gio::FILE_ATTRIBUTE_ACCESS_CAN_TRASH);
                (actual == expected)
                    .then_some(())
                    .ok_or(format!("can_trash is {actual}"))
            }
            FsKind::Mkdir { parents } if self.percent_path => {
                let made = if parents {
                    fs::create_dir_all(&path)
                } else {
                    fs::create_dir(&path)
                };
                made.map_err(|error| error.to_string())
            }
            FsKind::Mkdir { parents } => {
                let parent = path.parent().unwrap_or(root);
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                for name in expand_name(&name)? {
                    let made = if parents {
                        fs::create_dir_all(parent.join(&name))
                    } else {
                        fs::create_dir(parent.join(&name))
                    };
                    made.map_err(|error| format!("{name}: {error}"))?;
                }
                Ok(())
            }
            FsKind::Populate(tree) => {
                fs::create_dir_all(&path).map_err(|error| error.to_string())?;
                populate(&path, &expand_directory(tree)?);
                pin_times(&path, fixture_time);
                Ok(())
            }
            // PIL's buffered save writes a file this small in one write; written in
            // pieces, the listing would see it grow and replace its row.
            FsKind::Image(image) => {
                let parent = path.parent().unwrap_or(root);
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                for (name, number) in numbered(&name)? {
                    let target = parent.join(&name);
                    let wider = image.width_step * number.unwrap_or_default();
                    let bytes = png(&Image {
                        width: image.width + wider,
                        height: image.height,
                        rgb: image.rgb,
                        width_step: 0,
                        thumbnail: None,
                    })?;
                    fs::write(&target, bytes).map_err(|error| format!("{name}: {error}"))?;
                    if let Some(thumbnail) = &image.thumbnail {
                        contained(&target, &roots)?;
                        write_thumbnail(thumbnail, &target, &roots)?;
                    }
                }
                Ok(())
            }
            FsKind::AllCached(cache, except) => {
                let mut missing = Vec::new();
                for entry in fs::read_dir(&path).map_err(|error| error.to_string())? {
                    let source = entry.map_err(|error| error.to_string())?.path();
                    let name = source
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if except.is_some_and(|suffix| name.ends_with(suffix)) {
                        continue;
                    }
                    if !cached_thumbnail(Path::new(cache), &source).is_file() {
                        missing.push(name);
                    }
                }
                missing.sort();
                missing
                    .is_empty()
                    .then_some(())
                    .ok_or(format!("no cached thumbnail for {missing:?}"))
            }
            FsKind::Thumbnail(thumbnail) => write_thumbnail(thumbnail, &path, &roots),
            FsKind::Pixel(pixel) => {
                let image = image::open(&path)
                    .map_err(|error| format!("{}: {error}", path.display()))?
                    .to_rgb8();
                let [x, y] = pixel.at;
                let found = image
                    .get_pixel_checked(x, y)
                    .map(|colour| colour.0)
                    .ok_or_else(|| format!("{x},{y} lies outside the image"))?;
                (found == pixel.rgb)
                    .then_some(())
                    .ok_or(format!("the pixel is {found:?}"))
            }
            FsKind::AlphaRange(range) => {
                let image = image::open(&path)
                    .map_err(|error| format!("{}: {error}", path.display()))?
                    .to_rgba8();
                let alphas = image.pixels().map(|pixel| pixel.0[3]);
                let found = [
                    alphas.clone().min().unwrap_or_default(),
                    alphas.max().unwrap_or_default(),
                ];
                (found == range)
                    .then_some(())
                    .ok_or(format!("the alpha range is {found:?}"))
            }
            FsKind::KeepMtime(name) => {
                let mtime = modified_nanos(&path)?;
                state.mtimes.insert(name.to_owned(), mtime);
                Ok(())
            }
            FsKind::MtimeOf(name) => {
                let kept = *state
                    .mtimes
                    .get(name)
                    .ok_or_else(|| format!("no modification time was kept as {name:?}"))?;
                let now = modified_nanos(&path)?;
                (now == kept)
                    .then_some(())
                    .ok_or(format!("the modification time is {now}, not {kept}"))
            }
            FsKind::Executable(wanted) => {
                let mode = fs::metadata(&path)
                    .map_err(|error| error.to_string())?
                    .permissions()
                    .mode();
                (((mode & 0o111) == 0o111) == wanted)
                    .then_some(())
                    .ok_or(format!("the mode is {mode:o}"))
            }
            FsKind::Mtime(mtime) => {
                let expected = match mtime {
                    Mtime::Epoch(seconds) => seconds,
                    Mtime::Local { local } => local_epoch(local)?,
                };
                let seconds = fs::metadata(&path)
                    .map_err(|error| error.to_string())?
                    .modified()
                    .map_err(|error| error.to_string())?
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_err(|error| error.to_string())?
                    .as_secs();
                (i64::try_from(seconds).ok() == Some(expected))
                    .then_some(())
                    .ok_or(format!(
                        "the modification time is {seconds}, not {expected}"
                    ))
            }
            FsKind::Hex(text) => {
                fs::write(&path, hex::decode(text).map_err(|error| error.to_string())?)
                    .map_err(|error| error.to_string())
            }
            FsKind::Base64(text) => {
                fs::write(&path, gtk::glib::base64_decode(text)).map_err(|error| error.to_string())
            }
            FsKind::Append(text) => File::options()
                .append(true)
                .create(true)
                .open(&path)
                .and_then(|mut file| file.write_all(text.as_bytes()))
                .map_err(|error| error.to_string()),
            FsKind::RawPhoto => fs::write(&path, raw_photo()).map_err(|error| error.to_string()),
            FsKind::TarGz(archive) => {
                let mode =
                    u32::from_str_radix(&archive.mode, 8).map_err(|error| error.to_string())?;
                let mtime = u64::try_from(archive.mtime).map_err(|error| error.to_string())?;
                let encoder =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
                let mut builder = tar::Builder::new(encoder);
                for (name, text) in &archive.members {
                    let text = text.bytes();
                    let mut header = tar::Header::new_ustar();
                    header.set_size(text.len() as u64);
                    header.set_mode(mode);
                    header.set_mtime(mtime);
                    header.set_uid(0);
                    header.set_gid(0);
                    header.set_entry_type(tar::EntryType::Regular);
                    builder
                        .append_data(&mut header, name, text.as_slice())
                        .map_err(|error| format!("{name}: {error}"))?;
                }
                let mut bytes = builder
                    .into_inner()
                    .and_then(flate2::write::GzEncoder::finish)
                    .map_err(|error| error.to_string())?;
                if let Some(back) = archive.flip_from_end {
                    let index = bytes
                        .len()
                        .checked_sub(back)
                        .ok_or("flip_from_end lies before the archive")?;
                    bytes[index] ^= 0xFF;
                }
                fs::write(&path, bytes).map_err(|error| error.to_string())
            }
            FsKind::ZipMembers(members) => {
                let file = File::open(&path).map_err(|error| error.to_string())?;
                let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
                let names = archive
                    .file_names()
                    .map(|name| name.map(|name| name.into_owned()))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| error.to_string())?;
                let mut contents = Vec::new();
                for index in 0..archive.len() {
                    let mut member = archive.by_index(index).map_err(|error| error.to_string())?;
                    let mut bytes = Vec::new();
                    std::io::Read::read_to_end(&mut member, &mut bytes)
                        .map_err(|error| error.to_string())?;
                    contents.push(bytes);
                }
                archive_matches(root, members, &names, &contents)
            }
            FsKind::TarMembers(members) => {
                let file = File::open(&path).map_err(|error| error.to_string())?;
                let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
                let mut names = Vec::new();
                let mut contents = Vec::new();
                for entry in archive.entries().map_err(|error| error.to_string())? {
                    let mut entry = entry.map_err(|error| error.to_string())?;
                    names.push(
                        entry
                            .path()
                            .map_err(|error| error.to_string())?
                            .display()
                            .to_string(),
                    );
                    let mut bytes = Vec::new();
                    std::io::Read::read_to_end(&mut entry, &mut bytes)
                        .map_err(|error| error.to_string())?;
                    contents.push(bytes);
                }
                archive_matches(root, members, &names, &contents)
            }
            FsKind::IsZip(wanted) => {
                let is_zip = File::open(&path)
                    .ok()
                    .is_some_and(|file| zip::ZipArchive::new(file).is_ok());
                (is_zip == wanted)
                    .then_some(())
                    .ok_or(format!("is_zip is {is_zip}"))
            }
            FsKind::DefaultApp(default) => {
                use gtk::gio::prelude::FileExt as _;
                let content_type = gtk::gio::File::for_path(other(&default.file))
                    .query_info(
                        gtk::gio::FILE_ATTRIBUTE_STANDARD_CONTENT_TYPE,
                        gtk::gio::FileQueryInfoFlags::NONE,
                        gtk::gio::Cancellable::NONE,
                    )
                    .map_err(|error| error.to_string())?
                    .content_type()
                    .ok_or("the file has no content type")?;
                let associations = gtk::glib::KeyFile::new();
                associations
                    .load_from_file(&path, gtk::glib::KeyFileFlags::NONE)
                    .map_err(|error| error.to_string())?;
                let listed = associations
                    .string("Default Applications", &content_type)
                    .map_err(|error| format!("no default for {content_type}: {error}"))?;
                let first = listed.split(';').next().unwrap_or_default();
                (first == default.is)
                    .then_some(())
                    .ok_or(format!("the default for {content_type} is {first:?}"))
            }
            FsKind::Opened(paths) => {
                use gtk::gio::prelude::FileExt as _;
                let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
                let received: Vec<gtk::gio::File> = text
                    .lines()
                    .map(gtk::gio::File::for_commandline_arg)
                    .collect();
                let missing: Vec<&String> = paths
                    .iter()
                    .filter(|expected| {
                        let expected = gtk::gio::File::for_path(other(expected));
                        !received.iter().any(|file| file.equal(&expected))
                    })
                    .collect();
                (received.len() == paths.len() && missing.is_empty())
                    .then_some(())
                    .ok_or(format!("received {:?}", text.lines().collect::<Vec<_>>()))
            }
            FsKind::Toml(values) => {
                let parsed =
                    parsed_toml(&fs::read_to_string(&path).map_err(|error| error.to_string())?)?;
                let wrong: Vec<String> = values
                    .iter()
                    .filter(|(key, value)| {
                        key.split('.')
                            .try_fold(&parsed, |node, part| node.get(part))
                            != Some(*value)
                    })
                    .map(|(key, _)| key.clone())
                    .collect();
                wrong
                    .is_empty()
                    .then_some(())
                    .ok_or(format!("{wrong:?} differ in {parsed}"))
            }
            FsKind::TomlLike(name) => {
                let kept = state
                    .bytes
                    .get(name)
                    .ok_or_else(|| format!("no bytes were kept as {name:?}"))?;
                let kept = parsed_toml(&String::from_utf8_lossy(kept))?;
                let now =
                    parsed_toml(&fs::read_to_string(&path).map_err(|error| error.to_string())?)?;
                let same = if self.toml_keys.is_empty() {
                    now == kept
                } else {
                    self.toml_keys
                        .iter()
                        .all(|key| now.get(key) == kept.get(key))
                };
                same.then_some(()).ok_or(format!("the TOML is now {now}"))
            }
            FsKind::TomlPrefix(values) => {
                let parsed =
                    parsed_toml(&fs::read_to_string(&path).map_err(|error| error.to_string())?)?;
                let wrong: Vec<String> = values
                    .iter()
                    .filter(|(key, value)| {
                        let wanted = value.as_array().map(Vec::as_slice).unwrap_or_default();
                        !key.split('.')
                            .try_fold(&parsed, |node, part| node.get(part))
                            .and_then(Value::as_array)
                            .is_some_and(|found| found.starts_with(wanted))
                    })
                    .map(|(key, _)| key.clone())
                    .collect();
                wrong
                    .is_empty()
                    .then_some(())
                    .ok_or(format!("{wrong:?} differ in {parsed}"))
            }
            FsKind::Random(size) => {
                let mut bytes = vec![0; usize::try_from(size).map_err(|error| error.to_string())?];
                File::open("/dev/urandom")
                    .and_then(|mut random| std::io::Read::read_exact(&mut random, &mut bytes))
                    .map_err(|error| error.to_string())?;
                fs::write(&path, bytes).map_err(|error| error.to_string())
            }
            FsKind::Size(size) => {
                let actual = fs::metadata(&path)
                    .map(|metadata| metadata.len())
                    .map_err(|error| error.to_string())?;
                (actual == size)
                    .then_some(())
                    .ok_or(format!("the size is {actual}"))
            }
            FsKind::GlobNone(pattern) => {
                let names = sorted_names(&path)?;
                let found: Vec<&String> = names
                    .iter()
                    .filter(|name| glob_matches(pattern, name))
                    .collect();
                found
                    .is_empty()
                    .then_some(())
                    .ok_or(format!("names {found:?} match {pattern:?}"))
            }
            FsKind::LastLine(expected) => {
                let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
                let last = text.lines().last();
                (last == Some(expected))
                    .then_some(())
                    .ok_or(format!("the last line is {last:?}"))
            }
            FsKind::LinesEqual(expected) => {
                let lines: Vec<String> = match fs::read_to_string(&path) {
                    Ok(text) => text.lines().map(str::to_owned).collect(),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                    Err(error) => {
                        let error = format!("cannot read the lines: {error}");
                        state.fatal = Some(error.clone());
                        return Err(error);
                    }
                };
                (lines == expected)
                    .then_some(())
                    .ok_or(format!("the lines are {lines:?}"))
            }
            FsKind::ZipMember(member, file) => {
                let archive = File::open(&path).map_err(|error| error.to_string())?;
                let mut archive =
                    zip::ZipArchive::new(archive).map_err(|error| error.to_string())?;
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(
                    &mut archive.by_name(member).map_err(|error| error.to_string())?,
                    &mut bytes,
                )
                .map_err(|error| error.to_string())?;
                let expected = fs::read(other(file)).map_err(|error| error.to_string())?;
                (bytes == expected)
                    .then_some(())
                    .ok_or(format!("{member} holds other bytes than {file}"))
            }
            FsKind::Png(check) => check_png(&path, check),
            FsKind::Json(values) | FsKind::JsonSet(values) => {
                let as_set = matches!(self.kind()?, FsKind::JsonSet(_));
                let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
                let parsed: Value =
                    serde_json::from_str(&text).map_err(|error| error.to_string())?;
                let same = |found: &Value, wanted: &Value| match (found, wanted) {
                    (Value::Array(found), Value::Array(wanted)) if as_set => {
                        found.iter().all(|item| wanted.contains(item))
                            && wanted.iter().all(|item| found.contains(item))
                    }
                    _ => found == wanted,
                };
                let wrong: Vec<&String> = values
                    .iter()
                    .filter(|(key, value)| {
                        parsed
                            .get(key.as_str())
                            .is_none_or(|found| !same(found, value))
                    })
                    .map(|(key, _)| key)
                    .collect();
                wrong
                    .is_empty()
                    .then_some(())
                    .ok_or(format!("{wrong:?} differ in {parsed}"))
            }
            FsKind::Launches(launches) => {
                let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
                let launched: Vec<(&str, &str)> = text
                    .lines()
                    .map(|line| line.split_once('\t').unwrap_or((line, "")))
                    .collect();
                let resolve =
                    |path: &str| fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
                if let Some(first_dir) = &launches.first_dir {
                    let (_, arguments) = launched.first().ok_or("no launch was recorded")?;
                    let folder = resolve(arguments.strip_prefix("--dir=").unwrap_or(arguments));
                    return (folder == resolve(&other(first_dir).display().to_string()))
                        .then_some(())
                        .ok_or(format!("the terminal opened in {}", folder.display()));
                }
                let count = launches.count.unwrap_or_default();
                let within = resolve(
                    &other(launches.within.as_deref().unwrap_or_default())
                        .display()
                        .to_string(),
                );
                let (cwd, arguments) = launched.first().copied().unwrap_or_default();
                let folder = resolve(arguments.strip_prefix("--dir=").unwrap_or(arguments));
                (launched.len() == count
                    && launched.iter().all(|launch| *launch == launched[0])
                    && arguments.starts_with("--dir=")
                    && resolve(cwd) == folder
                    && folder.starts_with(&within))
                .then_some(())
                .ok_or(format!("the launches are {launched:?}"))
            }
            FsKind::ReplaceLines(replace) => {
                let mut files: Vec<PathBuf> = fs::read_dir(&path)
                    .map_err(|error| error.to_string())?
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|file| file.to_string_lossy().ends_with(&replace.suffix))
                    .collect();
                files.sort();
                for file in files {
                    contained(&file, &roots)?;
                    let text = fs::read_to_string(&file).map_err(|error| error.to_string())?;
                    let lines: Vec<&str> = text
                        .lines()
                        .map(|line| {
                            if line.starts_with(&replace.prefix) {
                                replace.with.as_str()
                            } else {
                                line
                            }
                        })
                        .collect();
                    fs::write(&file, lines.join("\n") + "\n").map_err(|error| error.to_string())?;
                }
                Ok(())
            }
            FsKind::PdfImage(image) => {
                fs::write(&path, pdf_image(image)?).map_err(|error| error.to_string())
            }
            FsKind::Magick(arguments) => magick(arguments, &path),
            FsKind::Ffmpeg(arguments) => ffmpeg(arguments, &path),
            FsKind::Zip(entries) => {
                let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored);
                for (name, entry) in entries {
                    let mut entry_options = options;
                    let bytes = match entry {
                        ZipEntry::Text(text) => text.as_bytes().to_vec(),
                        ZipEntry::Image { image } => png(image)?,
                        ZipEntry::Member {
                            text,
                            date_time: [year, month, day, hour, minute, second],
                            mode,
                        } => {
                            let mode = u32::from_str_radix(mode, 8)
                                .map_err(|_| format!("{name}: mode is octal, not {mode:?}"))?;
                            let time = zip::DateTime::from_date_and_time(
                                *year,
                                u8::try_from(*month).map_err(|error| error.to_string())?,
                                u8::try_from(*day).map_err(|error| error.to_string())?,
                                u8::try_from(*hour).map_err(|error| error.to_string())?,
                                u8::try_from(*minute).map_err(|error| error.to_string())?,
                                u8::try_from(*second).map_err(|error| error.to_string())?,
                            )
                            .map_err(|error| format!("{name}: {error}"))?;
                            entry_options = entry_options
                                .last_modified_time(time)
                                .unix_permissions(mode);
                            text.as_bytes().to_vec()
                        }
                    };
                    archive
                        .start_file(name.as_str(), entry_options)
                        .and_then(|()| Ok(archive.write_all(&bytes)?))
                        .map_err(|error| format!("{name}: {error}"))?;
                }
                let archive = archive.finish().map_err(|error| error.to_string())?;
                fs::write(&path, archive.into_inner()).map_err(|error| error.to_string())
            }
            FsKind::Truncate(size) => File::create(&path)
                .and_then(|file| file.set_len(size))
                .map_err(|error| error.to_string()),
            FsKind::FromRepository(name) => {
                let source = Path::new(REPOSITORY).join(name);
                let bytes =
                    fs::read(&source).map_err(|error| format!("{}: {error}", source.display()))?;
                fs::write(&path, bytes).map_err(|error| error.to_string())
            }
            FsKind::Remove => {
                contained(&path, &state.roots)?;
                if path.is_dir() && !path.is_symlink() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                }
                .map_err(|error| error.to_string())
            }
            FsKind::Rename(name) => {
                contained(&path, &state.roots)?;
                contained(&other(name), &state.roots)?;
                fs::rename(&path, other(name)).map_err(|error| error.to_string())
            }
            FsKind::KeepBytes(name) => {
                let bytes = fs::read(&path).map_err(|error| error.to_string())?;
                state.bytes.insert(name.to_owned(), bytes);
                Ok(())
            }
            FsKind::BytesOf(name) => {
                let kept = state
                    .bytes
                    .get(name)
                    .ok_or_else(|| format!("no bytes were kept as {name:?}"))?;
                let bytes = match fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        let error = format!("cannot read the bytes: {error}");
                        state.fatal = Some(error.clone());
                        return Err(error);
                    }
                };
                (bytes == *kept)
                    .then_some(())
                    .ok_or(format!("the bytes differ from {name:?}"))
            }
            FsKind::Lines(count) => {
                let lines = match fs::read_to_string(&path) {
                    Ok(text) => text.lines().count(),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                    Err(error) => {
                        let error = format!("cannot read the lines: {error}");
                        state.fatal = Some(error.clone());
                        return Err(error);
                    }
                };
                count
                    .matches(lines)
                    .then_some(())
                    .ok_or(format!("has {lines} lines"))
            }
        }
    }
}
