// SPDX-License-Identifier: MIT

use serde_json::Value;

pub(crate) const MAX_METADATA_BYTES: u64 = 64 * 1024;

/// Only technical properties are exposed; arbitrary embedded tags are not UI text.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MediaMetadata {
    pub(crate) dimensions: Option<(u32, u32)>,
    pub(crate) duration: Option<f64>,
    pub(crate) bitrate: Option<f64>,
    pub(crate) video_codec: Option<String>,
    pub(crate) audio_codec: Option<String>,
    pub(crate) frame_rate: Option<f64>,
    pub(crate) sample_rate: Option<f64>,
    pub(crate) channels: Option<u32>,
}

fn positive(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|number| number.is_finite() && *number > 0.0)
}

fn codec(stream: &Value) -> Option<String> {
    stream["codec_name"]
        .as_str()
        .filter(|name| {
            !name.is_empty()
                && name.len() <= 64
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
        })
        .map(str::to_owned)
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
                metadata.frame_rate = frame_rate(&video["avg_frame_rate"])
                    .or_else(|| frame_rate(&video["r_frame_rate"]));
            }
        }
        if !image {
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

#[cfg(test)]
mod tests;
