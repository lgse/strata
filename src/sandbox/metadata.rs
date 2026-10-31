// SPDX-License-Identifier: MIT

use serde_json::Value;

/// Pretty-printed chapter lists and subtitle streams need far more than a plain file.
pub(crate) const MAX_METADATA_BYTES: u64 = 256 * 1024;

const MAX_CHAPTERS: usize = 200;
const MAX_SUBTITLE_TRACKS: usize = 64;

/// Chapter titles and languages are sanitized; other arbitrary tags are not exposed.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MediaMetadata {
    pub(crate) dimensions: Option<(u32, u32)>,
    pub(crate) duration: Option<f64>,
    pub(crate) bitrate: Option<f64>,
    pub(crate) video_codec: Option<String>,
    pub(crate) pixel_format: Option<String>,
    pub(crate) color_transfer: Option<String>,
    pub(crate) audio_codec: Option<String>,
    pub(crate) frame_rate: Option<f64>,
    pub(crate) sample_rate: Option<f64>,
    pub(crate) channels: Option<u32>,
    pub(crate) channel_layout: Option<String>,
    pub(crate) chapters: Vec<Chapter>,
    pub(crate) subtitle_tracks: Vec<SubtitleTrack>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Chapter {
    pub(crate) start: f64,
    pub(crate) end: f64,
    pub(crate) title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SubtitleTrack {
    pub(crate) language: Option<String>,
}

fn positive(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|number| number.is_finite() && *number > 0.0)
}

fn codec(stream: &Value) -> Option<String> {
    token(&stream["codec_name"], |byte| b"_-".contains(&byte))
}

fn token(value: &Value, extra: impl Fn(u8) -> bool) -> Option<String> {
    value
        .as_str()
        .filter(|name| {
            !name.is_empty()
                && name.len() <= 64
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || extra(byte))
        })
        .map(str::to_owned)
}

fn label_token(value: &Value) -> Option<String> {
    token(value, |byte| b"_-.() ".contains(&byte))
}

fn language(stream: &Value) -> Option<String> {
    stream["tags"]
        .as_object()?
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("language"))
        .and_then(|(_, value)| value.as_str())
        .filter(|code| {
            (2..=8).contains(&code.len())
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_alphabetic() || byte == b'-')
                && code != &"und"
        })
        .map(str::to_ascii_lowercase)
}

fn time(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0 && *seconds <= 315_576_000.0)
}

fn duration(value: &Value) -> Option<f64> {
    positive(value).filter(|duration| *duration <= 315_576_000.0)
}

fn frame_rate(value: &Value) -> Option<f64> {
    let (numerator, denominator) = value.as_str()?.split_once('/')?;
    let rate = numerator.parse::<f64>().ok()? / denominator.parse::<f64>().ok()?;
    (rate.is_finite() && rate > 0.0 && rate <= 1000.0).then_some(rate)
}

impl MediaMetadata {
    pub(crate) fn hdr_format(&self) -> Option<&'static str> {
        match self.color_transfer.as_deref()? {
            "smpte2084" => Some("HDR10"),
            "arib-std-b67" => Some("HLG"),
            _ => None,
        }
    }

    pub(crate) fn from_json(bytes: &[u8], image: bool) -> Result<Self, String> {
        if bytes.len() as u64 > MAX_METADATA_BYTES {
            return Err("Media metadata exceeds the size limit".into());
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        let streams = value["streams"].as_array().ok_or("Missing media streams")?;
        let video = streams.iter().find(|stream| {
            stream["codec_type"] == "video"
                && stream["disposition"]["attached_pic"].as_u64() != Some(1)
        });
        let audio = streams
            .iter()
            .find(|stream| stream["codec_type"] == "audio");
        let mut metadata = Self::default();
        if let Some(video) = video {
            metadata.dimensions = video["width"]
                .as_u64()
                .zip(video["height"].as_u64())
                .filter(|(width, height)| {
                    (1..=1_000_000).contains(width) && (1..=1_000_000).contains(height)
                })
                .map(|(width, height)| (width as u32, height as u32));
            if video["side_data_list"].as_array().is_some_and(|data| {
                data.iter().any(|data| {
                    data["rotation"]
                        .as_i64()
                        .is_some_and(|rotation| rotation.rem_euclid(180) == 90)
                })
            }) {
                metadata.dimensions = metadata.dimensions.map(|(width, height)| (height, width));
            }
            if !image {
                metadata.video_codec = codec(video);
                metadata.pixel_format = token(&video["pix_fmt"], |byte| byte == b'_');
                metadata.color_transfer = token(&video["color_transfer"], |byte| byte == b'-');
                metadata.frame_rate = frame_rate(&video["avg_frame_rate"])
                    .or_else(|| frame_rate(&video["r_frame_rate"]));
            }
        }
        if !image {
            metadata.chapters = value["chapters"]
                .as_array()
                .map(|chapters| {
                    chapters
                        .iter()
                        .filter_map(|chapter| {
                            let start = time(&chapter["start_time"])?;
                            let end = time(&chapter["end_time"]).filter(|end| *end >= start)?;
                            Some(Chapter {
                                start,
                                end,
                                title: chapter["tags"]["title"].as_str().and_then(display_text),
                            })
                        })
                        .take(MAX_CHAPTERS)
                        .collect()
                })
                .unwrap_or_default();
            metadata.subtitle_tracks = streams
                .iter()
                .filter(|stream| stream["codec_type"] == "subtitle")
                .map(|stream| SubtitleTrack {
                    language: language(stream),
                })
                .take(MAX_SUBTITLE_TRACKS)
                .collect();
            metadata.duration = duration(&value["format"]["duration"]).or_else(|| {
                streams
                    .iter()
                    .filter(|stream| {
                        stream["codec_type"] == "audio"
                            || (stream["codec_type"] == "video"
                                && stream["disposition"]["attached_pic"].as_u64() != Some(1))
                    })
                    .filter_map(|stream| duration(&stream["duration"]))
                    .max_by(f64::total_cmp)
            });
            metadata.bitrate =
                positive(&value["format"]["bit_rate"]).filter(|bitrate| *bitrate <= 1e12);
            if let Some(audio) = audio {
                metadata.audio_codec = codec(audio);
                metadata.channel_layout = label_token(&audio["channel_layout"]);
                metadata.sample_rate =
                    positive(&audio["sample_rate"]).filter(|rate| *rate <= 10_000_000.0);
                metadata.channels = audio["channels"]
                    .as_u64()
                    .filter(|channels| (1..=1024).contains(channels))
                    .map(|channels| channels as u32);
            }
        }
        Ok(metadata)
    }
}

pub(crate) const TAG_KEYS: &str = "title,artist,album,album_artist,track,tracktotal,totaltracks";
const MAX_TAG_CHARS: usize = 200;

/// Untrusted tags sanitized for plain-text display.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AudioTags {
    pub(crate) title: Option<String>,
    pub(crate) artist: Option<String>,
    pub(crate) album: Option<String>,
    pub(crate) track: Option<u32>,
    pub(crate) track_total: Option<u32>,
}

fn display_text(value: &str) -> Option<String> {
    let mut text = String::new();
    for word in value
        .chars()
        .filter(|character| {
            !character.is_control()
                && !matches!(
                    character,
                    '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
                )
        })
        .collect::<String>()
        .split_whitespace()
    {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(word);
    }
    if text.chars().count() > MAX_TAG_CHARS {
        text = text.chars().take(MAX_TAG_CHARS - 1).collect::<String>();
        text.push('…');
    }
    (!text.is_empty()).then_some(text)
}

fn track_number(value: &str) -> Option<u32> {
    value
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|number| (1..=9_999).contains(number))
}

impl AudioTags {
    pub(crate) fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() as u64 > MAX_METADATA_BYTES {
            return Err("Audio tags exceed the size limit".into());
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        let sources = [&value["format"]["tags"], &value["streams"][0]["tags"]];
        let tag = |name: &str| {
            sources.iter().find_map(|tags| {
                tags.as_object()?
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .and_then(|(_, value)| value.as_str())
            })
        };
        let (track, slash_total) = tag("track").map_or((None, None), |track| {
            let (number, total) = track.split_once('/').unwrap_or((track, ""));
            (track_number(number), track_number(total))
        });
        Ok(Self {
            title: tag("title").and_then(display_text),
            artist: tag("artist")
                .or_else(|| tag("album_artist"))
                .and_then(display_text),
            album: tag("album").and_then(display_text),
            track,
            track_total: slash_total
                .or_else(|| tag("tracktotal").and_then(track_number))
                .or_else(|| tag("totaltracks").and_then(track_number))
                .filter(|total| track.is_some_and(|track| track <= *total)),
        })
    }
}

#[cfg(test)]
mod tests;
