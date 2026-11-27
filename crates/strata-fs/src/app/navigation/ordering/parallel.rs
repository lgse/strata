// SPDX-License-Identifier: MIT

//! Safe, dependency-free parallel stable merge sort using `std::thread::scope`.

use std::cmp::Ordering;

/// Minimum element count before parallelizing sort work across worker threads.
const PARALLEL_THRESHOLD: usize = 8_192;
/// Minimum elements per chunk to keep thread spawn overhead negligible.
const MIN_CHUNK_LEN: usize = 4_096;
/// Cap worker count so a single sort does not oversubscribe the host.
const MAX_WORKERS: usize = 8;
/// Minimum merged run size before splitting a 2-way merge across two threads.
const PARALLEL_MERGE_SPLIT_THRESHOLD: usize = 16_384;

/// Stably sorts `items` in parallel when large enough, otherwise calls `slice::sort_by`.
pub(super) fn par_sort_by<T, F>(items: &mut [T], compare: F)
where
    T: Copy + Send + Sync,
    F: Fn(&T, &T) -> Ordering + Sync,
{
    if items.len() < PARALLEL_THRESHOLD {
        items.sort_by(compare);
        return;
    }
    let workers = worker_count(items.len());
    if workers <= 1 {
        items.sort_by(compare);
        return;
    }

    let chunk_len = items.len().div_ceil(workers);
    let mut ranges: Vec<(usize, usize)> = Vec::with_capacity(workers);
    let mut start = 0;
    while start < items.len() {
        let end = (start + chunk_len).min(items.len());
        ranges.push((start, end));
        start = end;
    }

    std::thread::scope(|scope| {
        let mut rest = &mut *items;
        for &(s, e) in &ranges {
            let (chunk, tail) = rest.split_at_mut(e - s);
            rest = tail;
            let cmp = &compare;
            scope.spawn(move || chunk.sort_by(cmp));
        }
    });

    merge_runs(items, ranges, &compare);
}

fn worker_count(len: usize) -> usize {
    let available = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(MAX_WORKERS);
    available.min(len / MIN_CHUNK_LEN).max(1)
}

fn merge_runs<T, F>(items: &mut [T], mut ranges: Vec<(usize, usize)>, compare: &F)
where
    T: Copy + Send + Sync,
    F: Fn(&T, &T) -> Ordering + Sync,
{
    let mut scratch: Vec<T> = items.to_vec();
    let mut from_items = true;

    while ranges.len() > 1 {
        let mut next_ranges = Vec::with_capacity(ranges.len().div_ceil(2));
        let (src, dst): (&[T], &mut [T]) = if from_items {
            (&*items, &mut scratch)
        } else {
            (&scratch, &mut *items)
        };
        merge_round((src, dst), &ranges, &mut next_ranges, compare);
        ranges = next_ranges;
        from_items = !from_items;
    }

    if !from_items {
        items.copy_from_slice(&scratch);
    }
}

fn merge_round<T, F>(
    (src, mut dst): (&[T], &mut [T]),
    ranges: &[(usize, usize)],
    next_ranges: &mut Vec<(usize, usize)>,
    compare: &F,
) where
    T: Copy + Send + Sync,
    F: Fn(&T, &T) -> Ordering + Sync,
{
    let mut tasks: Vec<(&[T], &[T], &mut [T])> = Vec::new();
    let mut i = 0;
    while i < ranges.len() {
        if i + 1 < ranges.len() {
            let (l_start, l_end) = ranges[i];
            let (r_start, r_end) = ranges[i + 1];
            let (out, tail) = dst.split_at_mut(r_end - l_start);
            dst = tail;
            next_ranges.push((l_start, r_end));
            tasks.push((&src[l_start..l_end], &src[r_start..r_end], out));
            i += 2;
        } else {
            let (start, end) = ranges[i];
            let (out, tail) = dst.split_at_mut(end - start);
            dst = tail;
            out.copy_from_slice(&src[start..end]);
            next_ranges.push((start, end));
            i += 1;
        }
    }

    if tasks.len() == 1 {
        let (left, right, out) = tasks.pop().expect("single merge task");
        parallel_merge_two(left, right, out, compare);
        return;
    }

    std::thread::scope(|scope| {
        for (left, right, out) in tasks {
            scope.spawn(move || merge_two(left, right, out, compare));
        }
    });
}

/// Splits a large 2-way merge at the left median so the final round also uses two threads.
fn parallel_merge_two<T, F>(left: &[T], right: &[T], out: &mut [T], compare: &F)
where
    T: Copy + Send + Sync,
    F: Fn(&T, &T) -> Ordering + Sync,
{
    if left.len() + right.len() < PARALLEL_MERGE_SPLIT_THRESHOLD || left.len() < 2 {
        merge_two(left, right, out, compare);
        return;
    }
    let mid_l = left.len() / 2;
    let pivot = &left[mid_l];
    // Strict `<` keeps elements equal to `pivot` in `right` after `left[mid_l]`, preserving
    // stability (`left` precedes `right` on ties).
    let mid_r = right.partition_point(|item| compare(item, pivot) == Ordering::Less);
    let (out_lo, out_hi) = out.split_at_mut(mid_l + mid_r);
    let (left_lo, left_hi) = left.split_at(mid_l);
    let (right_lo, right_hi) = right.split_at(mid_r);
    std::thread::scope(|scope| {
        scope.spawn(move || merge_two(left_lo, right_lo, out_lo, compare));
        merge_two(left_hi, right_hi, out_hi, compare);
    });
}

/// Stable 2-way merge: `left` wins on `Ordering::Equal`.
fn merge_two<T: Copy, F: Fn(&T, &T) -> Ordering>(left: &[T], right: &[T], out: &mut [T], cmp: &F) {
    let (mut li, mut ri, mut oi) = (0, 0, 0);
    while li < left.len() && ri < right.len() {
        if cmp(&left[li], &right[ri]) != Ordering::Greater {
            out[oi] = left[li];
            li += 1;
        } else {
            out[oi] = right[ri];
            ri += 1;
        }
        oi += 1;
    }
    if li < left.len() {
        out[oi..oi + (left.len() - li)].copy_from_slice(&left[li..]);
    } else if ri < right.len() {
        out[oi..oi + (right.len() - ri)].copy_from_slice(&right[ri..]);
    }
}
