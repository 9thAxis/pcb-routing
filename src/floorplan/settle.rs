//! Places for bodies: along a line, on a ground, turned by quarters.

use super::constants::SETTLE;
use crate::model::Pt;

pub(super) fn middle(rect: [f64; 4]) -> Pt {
    [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0]
}

/// The middle value of each coordinate of some points.
pub(super) fn median(points: &[Pt]) -> Option<Pt> {
    if points.is_empty() {
        return None;
    }
    let of = |k: usize| {
        let mut values: Vec<f64> = points.iter().map(|p| p[k]).collect();
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    };
    Some([of(0), of(1)])
}

/// The ground between two grounds that are neighbours, no further apart than `most`, and whether it runs up
/// and down (they are side by side) or across (one is above the other): as long as they are beside each other.
pub(super) fn strip_between(a: [f64; 4], b: [f64; 4], most: f64) -> Option<([f64; 4], bool)> {
    for k in [0usize, 1] {
        let j = 1 - k;
        let (low, high) = (a[j].max(b[j]), a[j + 2].min(b[j + 2]));
        let (first, second) = if a[k] <= b[k] { (a, b) } else { (b, a) };
        let wide = second[k] - first[k + 2];
        if high - low > 1e-6 && wide > -1e-6 && wide <= most + 1e-6 {
            let mut strip = [0.0; 4];
            (strip[k], strip[k + 2], strip[j], strip[j + 2]) = (first[k + 2], second[k], low, high);
            return Some((strip, k == 0));
        }
    }
    None
}

/// Places along a line from `low` to `high` for things of the sizes given, each of which wants one: in the
/// order of what they want, none over another, each as near what it wants as that allows (the squares of how
/// far each is from it are the least they can be). What does not fit sticks out as much at both ends.
pub(super) fn spread(want: &[f64], size: &[f64], low: f64, high: f64) -> Vec<f64> {
    struct Lot {
        first: usize,
        count: usize,
        size: f64,
        asks: f64,
        start: f64,
    }
    let room = high - low;
    let settled = |mut lot: Lot| {
        let start = lot.asks / lot.count as f64;
        lot.start = if lot.size >= room { low - (lot.size - room) / 2.0 } else { start.clamp(low, high - lot.size) };
        lot
    };
    let mut order: Vec<usize> = (0..want.len()).collect();
    order.sort_by(|&a, &b| want[a].total_cmp(&want[b]).then(a.cmp(&b)));
    let mut lots: Vec<Lot> = Vec::new();
    for (k, &i) in order.iter().enumerate() {
        let mut lot = settled(Lot { first: k, count: 1, size: size[i], asks: want[i] - size[i] / 2.0, start: 0.0 });
        // (what would stand over the lot before it is one lot with it, placed as one)
        while let Some(last) = lots.pop() {
            if last.start + last.size <= lot.start + 1e-9 {
                lots.push(last);
                break;
            }
            lot = settled(Lot {
                first: last.first,
                count: last.count + lot.count,
                size: last.size + lot.size,
                asks: last.asks + lot.asks - lot.count as f64 * last.size,
                start: 0.0,
            });
        }
        lots.push(lot);
    }
    let mut at = vec![0.0; want.len()];
    for lot in &lots {
        let mut from = lot.start;
        for &i in &order[lot.first..lot.first + lot.count] {
            at[i] = from + size[i] / 2.0;
            from += size[i];
        }
    }
    at
}

/// A place turned so many quarters counter-clockwise as the board is looked at (its y runs down).
pub(super) fn quartered(quarters: u8, at: Pt) -> Pt {
    match quarters % 4 {
        0 => at,
        1 => [at[1], -at[0]],
        2 => [-at[0], -at[1]],
        _ => [-at[1], at[0]],
    }
}

/// What a capacitor's value says it holds, in farads: `100n`, `4u7 16V`, `10uF`, `22 pF`; None if it says none.
pub fn farads(value: &str) -> Option<f64> {
    let text: String = value.trim().replace(['µ', 'μ'], "u");
    let mut digits = String::new();
    let mut scale = None;
    for c in text.chars() {
        match c {
            '0'..='9' | '.' => digits.push(c),
            'p' | 'n' | 'u' | 'm' if scale.is_none() && !digits.is_empty() => {
                scale = Some(match c {
                    'p' => 1e-12,
                    'n' => 1e-9,
                    'u' => 1e-6,
                    _ => 1e-3,
                });
                if !digits.contains('.') {
                    digits.push('.');
                }
            }
            ' ' if scale.is_none() => {}
            _ => break,
        }
    }
    let number: f64 = digits.trim_end_matches('.').parse().ok()?;
    scale.map(|scale| number * scale)
}

/// Bodies settled on a ground: each ([wide, high], where its middle wants to be) is given a place for its
/// middle and whether it is turned a quarter, the largest first, each as near what it wants as what is
/// settled before it and the ground `shut` allow, and as it stands rather than turned where both are as near.
/// None for one there is no room left for.
pub(super) fn settled(rect: [f64; 4], bodies: &[([f64; 2], Pt)], shut: &[[f64; 4]]) -> Vec<Option<(Pt, bool)>> {
    let cells = |mm: f64| (mm / SETTLE - 1e-6).ceil().max(1.0) as usize;
    let (wide, high) = (cells(rect[2] - rect[0]), cells(rect[3] - rect[1]));
    let mut taken = vec![false; wide * high];
    let mark = |taken: &mut Vec<bool>, r: [f64; 4]| {
        let low = |v: f64, from: f64, most: usize| (((v - from) / SETTLE + 1e-6).floor().max(0.0) as usize).min(most);
        let top = |v: f64, from: f64, most: usize| (((v - from) / SETTLE - 1e-6).ceil().max(0.0) as usize).min(most);
        for y in low(r[1], rect[1], high)..top(r[3], rect[1], high) {
            for x in low(r[0], rect[0], wide)..top(r[2], rect[0], wide) {
                taken[y * wide + x] = true;
            }
        }
    };
    shut.iter().for_each(|&k| mark(&mut taken, k));
    let mut order: Vec<usize> = (0..bodies.len()).collect();
    order.sort_by(|&a, &b| {
        let area = |i: usize| bodies[i].0[0] * bodies[i].0[1];
        area(b).total_cmp(&area(a)).then(a.cmp(&b))
    });
    let mut found: Vec<Option<(Pt, bool)>> = vec![None; bodies.len()];
    let mut sums = vec![0u32; (wide + 1) * (high + 1)];
    for i in order {
        // (how much is taken up to each cell, that any body's ground is asked in one go)
        for y in 0..high {
            let mut row = 0u32;
            for x in 0..wide {
                row += u32::from(taken[y * wide + x]);
                sums[(y + 1) * (wide + 1) + x + 1] = sums[y * (wide + 1) + x + 1] + row;
            }
        }
        let ([w, h], want) = bodies[i];
        let mut best: Option<(f64, usize, usize, bool)> = None;
        for turn in [false, true] {
            let (bw, bh) = if turn { (cells(h), cells(w)) } else { (cells(w), cells(h)) };
            if bw > wide || bh > high || (turn && bw == bh) {
                continue;
            }
            for y in 0..=high - bh {
                for x in 0..=wide - bw {
                    let at = |x: usize, y: usize| sums[y * (wide + 1) + x];
                    if at(x + bw, y + bh) + at(x, y) != at(x + bw, y) + at(x, y + bh) {
                        continue;
                    }
                    let mid = [
                        rect[0] + (x as f64 + bw as f64 / 2.0) * SETTLE,
                        rect[1] + (y as f64 + bh as f64 / 2.0) * SETTLE,
                    ];
                    let far = (mid[0] - want[0]).powi(2) + (mid[1] - want[1]).powi(2) + if turn { 1e-3 } else { 0.0 };
                    if best.map_or(true, |b| far < b.0) {
                        best = Some((far, x, y, turn));
                    }
                }
            }
        }
        if let Some((_, x, y, turn)) = best {
            let (bw, bh) = if turn { (cells(h), cells(w)) } else { (cells(w), cells(h)) };
            let r = [
                rect[0] + x as f64 * SETTLE,
                rect[1] + y as f64 * SETTLE,
                rect[0] + (x + bw) as f64 * SETTLE,
                rect[1] + (y + bh) as f64 * SETTLE,
            ];
            mark(&mut taken, r);
            found[i] = Some((middle(r), turn));
        }
    }
    found
}

/// A place on a ground for a body of several pieces (each [x0, y0, x1, y1] from its middle), given in each of
/// the ways it may be turned: for each way, the place for its middle nearest `want` at which every piece is
/// on the ground and off what is `shut`, no further from `want` than `within`; None where there is none.
pub(super) fn settled_shape(
    rect: [f64; 4],
    ways: &[Vec<[f64; 4]>],
    want: Pt,
    shut: &[[f64; 4]],
    within: f64,
) -> Vec<Option<Pt>> {
    let cells = |mm: f64| (mm / SETTLE - 1e-6).ceil().max(1.0) as i64;
    let (wide, high) = (cells(rect[2] - rect[0]), cells(rect[3] - rect[1]));
    let mut sums = vec![0u32; ((wide + 1) * (high + 1)) as usize];
    {
        let mut taken = vec![false; (wide * high) as usize];
        let low = |v: f64, from: f64, most: i64| (((v - from) / SETTLE + 1e-6).floor() as i64).clamp(0, most);
        let top = |v: f64, from: f64, most: i64| (((v - from) / SETTLE - 1e-6).ceil() as i64).clamp(0, most);
        for r in shut {
            for y in low(r[1], rect[1], high)..top(r[3], rect[1], high) {
                for x in low(r[0], rect[0], wide)..top(r[2], rect[0], wide) {
                    taken[(y * wide + x) as usize] = true;
                }
            }
        }
        for y in 0..high {
            let mut row = 0u32;
            for x in 0..wide {
                row += u32::from(taken[(y * wide + x) as usize]);
                sums[((y + 1) * (wide + 1) + x + 1) as usize] = sums[(y * (wide + 1) + x + 1) as usize] + row;
            }
        }
    }
    let at = |x: i64, y: i64| sums[(y * (wide + 1) + x) as usize];
    // (the middles tried: the points of the grid no further from what is wanted than `within`)
    let span = |want: f64, from: f64, most: i64| {
        let low = (((want - within - from) / SETTLE).floor() as i64).clamp(0, most);
        let top = (((want + within - from) / SETTLE).ceil() as i64).clamp(0, most);
        (low, top)
    };
    let ((x_low, x_top), (y_low, y_top)) = (span(want[0], rect[0], wide), span(want[1], rect[1], high));
    ways.iter()
        .map(|pieces| {
            // (each piece as cells from the middle's)
            let pieces: Vec<[i64; 4]> = pieces
                .iter()
                .map(|r| {
                    [
                        (r[0] / SETTLE + 1e-6).floor() as i64,
                        (r[1] / SETTLE + 1e-6).floor() as i64,
                        (r[2] / SETTLE - 1e-6).ceil() as i64,
                        (r[3] / SETTLE - 1e-6).ceil() as i64,
                    ]
                })
                .collect();
            let mut best: Option<(f64, Pt)> = None;
            for y in y_low..=y_top {
                for x in x_low..=x_top {
                    let mid = [rect[0] + x as f64 * SETTLE, rect[1] + y as f64 * SETTLE];
                    let far = (mid[0] - want[0]).powi(2) + (mid[1] - want[1]).powi(2);
                    if best.is_some_and(|b| far >= b.0) {
                        continue;
                    }
                    let free = pieces.iter().all(|r| {
                        let (x0, y0, x1, y1) = (x + r[0], y + r[1], x + r[2], y + r[3]);
                        x0 >= 0
                            && y0 >= 0
                            && x1 <= wide
                            && y1 <= high
                            && at(x1, y1) + at(x0, y0) == at(x1, y0) + at(x0, y1)
                    });
                    if free {
                        best = Some((far, mid));
                    }
                }
            }
            best.map(|b| b.1)
        })
        .collect()
}
