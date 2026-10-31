// SPDX-License-Identifier: MIT

use gtk::{gio, glib};

const SIGNATURE: [u32; 2] = [0x7261_5647, 0x746e_6169];
const HEADER_BYTES: usize = 24;
const NO_PARENT: u32 = u32::MAX;

pub(crate) fn with_file(
    folder: &str,
    name: &str,
    data: &[u8],
) -> Result<gio::Resource, glib::Error> {
    gio::Resource::from_data(&glib::Bytes::from_owned(bundle(folder, name, data)))
}

fn hash(key: &str) -> u32 {
    key.bytes().fold(5381_u32, |hash, byte| {
        hash.wrapping_mul(33).wrapping_add(byte as i8 as u32)
    })
}

fn bundle(folder: &str, name: &str, data: &[u8]) -> Vec<u8> {
    let mut own = vec!["/".to_owned()];
    own.extend(
        folder
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(|segment| format!("{segment}/")),
    );
    own.push(name.to_owned());
    let keys: Vec<String> = own
        .iter()
        .scan(String::new(), |path, key| {
            path.push_str(key);
            Some(path.clone())
        })
        .collect();
    let count = keys.len();
    let buckets: Vec<usize> = keys.iter().map(|key| hash(key) as usize % count).collect();
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by_key(|&item| buckets[item]);
    let mut slot = vec![0_u32; count];
    for (position, &item) in order.iter().enumerate() {
        slot[item] = position as u32;
    }

    let mut out = vec![0_u8; HEADER_BYTES];
    let key_spans: Vec<(u32, u16)> = own
        .iter()
        .map(|key| {
            let start = out.len() as u32;
            out.extend_from_slice(key.as_bytes());
            (start, key.len() as u16)
        })
        .collect();
    let mut values = Vec::with_capacity(count);
    for child in &slot[1..] {
        align(&mut out, 4);
        let start = out.len();
        out.extend_from_slice(&child.to_le_bytes());
        values.push((b'L', start, out.len()));
    }
    align(&mut out, 8);
    let start = out.len();
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(data);
    out.push(0);
    out.push(0);
    out.extend_from_slice(b"(uuay)");
    values.push((b'v', start, out.len()));

    align(&mut out, 4);
    let root = out.len();
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(&(count as u32).to_le_bytes());
    for bucket in 0..count {
        let first = order
            .iter()
            .position(|&item| buckets[item] >= bucket)
            .unwrap_or(count);
        out.extend_from_slice(&(first as u32).to_le_bytes());
    }
    for &item in &order {
        let parent = item.checked_sub(1).map_or(NO_PARENT, |parent| slot[parent]);
        let (key_start, key_size) = key_spans[item];
        let (kind, value_start, value_end) = values[item];
        out.extend_from_slice(&hash(&keys[item]).to_le_bytes());
        out.extend_from_slice(&parent.to_le_bytes());
        out.extend_from_slice(&key_start.to_le_bytes());
        out.extend_from_slice(&key_size.to_le_bytes());
        out.extend_from_slice(&[kind, 0]);
        out.extend_from_slice(&(value_start as u32).to_le_bytes());
        out.extend_from_slice(&(value_end as u32).to_le_bytes());
    }
    let end = out.len() as u32;
    let header = [SIGNATURE[0], SIGNATURE[1], 0, 0, root as u32, end];
    for (field, value) in out.as_chunks_mut::<4>().0.iter_mut().zip(header) {
        *field = value.to_le_bytes();
    }
    out
}

fn align(out: &mut Vec<u8>, to: usize) {
    out.resize(out.len().next_multiple_of(to), 0);
}

#[cfg(test)]
mod tests;
