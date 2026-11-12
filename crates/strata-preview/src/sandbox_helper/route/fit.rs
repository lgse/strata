// SPDX-License-Identifier: MIT

use super::{Point, Route, elevation};
use crate::services::route_preview::INVALID_ROUTE;
use fitparser::{
    FitDataRecord, Value,
    de::{FitObject, FitStreamProcessor},
    profile::MesgNum,
};

pub(super) fn parse(mut bytes: &[u8]) -> Result<Route, String> {
    let mut route = Route::default();
    let mut messages = 0usize;
    while !bytes.is_empty() {
        let header = *bytes.first().ok_or(INVALID_ROUTE)? as usize;
        if !matches!(header, 12 | 14) || bytes.get(8..12) != Some(b".FIT") {
            return Err(INVALID_ROUTE.into());
        }
        let data = u32::from_le_bytes(
            bytes
                .get(4..8)
                .ok_or(INVALID_ROUTE)?
                .try_into()
                .map_err(|_| INVALID_ROUTE)?,
        ) as usize;
        let length = header
            .checked_add(data)
            .and_then(|n| n.checked_add(2))
            .ok_or(INVALID_ROUTE)?;
        let mut remaining = bytes.get(..length).ok_or(INVALID_ROUTE)?;
        bytes = &bytes[length..];
        let mut processor = FitStreamProcessor::new();
        let mut crc = false;
        route.split();
        while !remaining.is_empty() {
            messages += 1;
            if messages > 1_000_000 {
                return Err(INVALID_ROUTE.into());
            }
            let (rest, object) = processor
                .deserialize_next(remaining)
                .map_err(|_| INVALID_ROUTE)?;
            if rest.len() >= remaining.len() {
                return Err(INVALID_ROUTE.into());
            }
            remaining = rest;
            match object {
                FitObject::Crc(_) => crc = true,
                FitObject::DataMessage(message) => {
                    // Read physical fields before the decoder expands legacy altitude into field 78.
                    let fields = message.fields();
                    let point = if message.global_message_number() == 20 {
                        number(fields.get(&0))
                            .zip(number(fields.get(&1)))
                            .map(|(lat, lon)| Point {
                                lat: lat * 180.0 / 2147483648.0,
                                lon: lon * 180.0 / 2147483648.0,
                                elevation: number(fields.get(&78))
                                    .or_else(|| number(fields.get(&2)))
                                    .and_then(|raw| elevation(raw / 5.0 - 500.0)),
                            })
                    } else {
                        None
                    };
                    let record = processor
                        .decode_message(message)
                        .map_err(|_| INVALID_ROUTE)?;
                    match record.kind() {
                        MesgNum::Record => {
                            if let Some(Value::Timestamp(time)) = field(&record, "timestamp") {
                                route.record_timestamp(time.timestamp());
                            }
                            route.push(point)?;
                        }
                        MesgNum::Event
                            if field(&record, "event")
                                .is_some_and(|v| matches!(v, Value::String(s) if s == "timer")) =>
                        {
                            route.split()
                        }
                        MesgNum::Session | MesgNum::Activity => route.split(),
                        MesgNum::FileId => {
                            if let Some(Value::String(kind)) = field(&record, "type")
                                && !matches!(kind.as_str(), "activity" | "course")
                            {
                                return Err(INVALID_ROUTE.into());
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        if !crc {
            return Err(INVALID_ROUTE.into());
        }
    }
    Ok(route)
}

fn field<'a>(record: &'a FitDataRecord, name: &str) -> Option<&'a Value> {
    record
        .fields()
        .iter()
        .find(|field| field.name() == name)
        .map(|field| field.value())
}

fn number(value: Option<&Value>) -> Option<f64> {
    let number: f64 = value?.clone().try_into().ok()?;
    number.is_finite().then_some(number)
}
