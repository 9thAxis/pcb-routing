//! Polygons as the work on zones needs them: boxes, sides, crossings, and a polygon kept so that a point is
//! soon told to be on it or not.

use crate::geom::{dist, dist_segment, nearest_on_segment, within};
use crate::model::Pt;

/// A box `[x0, y0, x1, y1]`.
pub(super) type Bounds = [f64; 4];

/// A point rounded to a millionth of a millimetre.
pub(super) fn rounded(p: Pt) -> Pt {
    [(p[0] * 1e6).round() / 1e6, (p[1] * 1e6).round() / 1e6]
}

/// A point in millionths of a millimetre: what two places are the same by.
pub(super) fn micro(p: Pt) -> (i64, i64) {
    ((p[0] * 1e6).round() as i64, (p[1] * 1e6).round() as i64)
}

/// The head of the set that `i` is in, where `leader` names for each a member of its set nearer the head.
pub(super) fn head_of(leader: &[usize], mut i: usize) -> usize {
    while leader[i] != i {
        i = leader[i];
    }
    i
}

/// True if two pieces of copper of one net hang together: tracks (a layer and the points of a line) where an
/// end of one lies on the other on the same layer, and a via (no layer, one point) with what it stands on.
pub(super) fn hang_together(a: (Option<&str>, &[Pt]), b: (Option<&str>, &[Pt])) -> bool {
    let on = |p: Pt, line: &[Pt]| match line {
        [only] => dist(p, *only) < 1e-6,
        _ => line.windows(2).any(|pair| dist_segment(p, pair[0], pair[1]) < 1e-6),
    };
    let ends = |line: &[Pt]| [line.first().copied(), line.last().copied()];
    let ends_on = |one: &[Pt], other: &[Pt]| ends(one).iter().flatten().any(|&p| on(p, other));
    let same_layer = a.0.is_none() || b.0.is_none() || a.0 == b.0;
    same_layer && (ends_on(a.1, b.1) || ends_on(b.1, a.1))
}

/// The box round some points; one that holds nothing if there are none.
pub(super) fn bounds<'a>(points: impl IntoIterator<Item = &'a Pt>) -> Bounds {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in points {
        b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    }
    b
}

/// How far apart two boxes are; 0 if they touch or overlap.
pub(super) fn apart(a: Bounds, b: Bounds) -> f64 {
    (a[0] - b[2]).max(b[0] - a[2]).max(0.0).hypot((a[1] - b[3]).max(b[1] - a[3]).max(0.0))
}

/// True if two boxes touch or overlap.
pub(super) fn meet(a: Bounds, b: Bounds) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// The sides of a ring, the last back to its first point.
pub(super) fn sides(ring: &[Pt]) -> impl Iterator<Item = (Pt, Pt)> + '_ {
    (0..ring.len()).map(move |k| (ring[k], ring[(k + 1) % ring.len()]))
}

/// The point a share `t` of the way from `a` to `b`.
pub(super) fn between(a: Pt, b: Pt, t: f64) -> Pt {
    [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]
}

/// Where the segments a-b and c-d cross, as the share of the way along each; None if they are parallel or
/// do not meet. Their ends count.
pub(super) fn crossing(a: Pt, b: Pt, c: Pt, d: Pt) -> Option<(f64, f64)> {
    let den = (b[0] - a[0]) * (d[1] - c[1]) - (b[1] - a[1]) * (d[0] - c[0]);
    if den == 0.0 {
        return None;
    }
    let t = ((c[0] - a[0]) * (d[1] - c[1]) - (c[1] - a[1]) * (d[0] - c[0])) / den;
    let u = ((c[0] - a[0]) * (b[1] - a[1]) - (c[1] - a[1]) * (b[0] - a[0])) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some((t, u))
}

/// Places along a ring, `spacing` apart, or further apart for `most` of them at most.
pub(super) fn along(ring: &[Pt], spacing: f64, most: usize) -> Vec<Pt> {
    let round: f64 = sides(ring).map(|(a, b)| dist(a, b)).sum();
    let spacing = spacing.max(round / most as f64);
    let mut places = Vec::new();
    let mut left = 0.0; // how far the next place lies along the side at hand
    for (a, b) in sides(ring) {
        let length = dist(a, b);
        while left < length - 1e-12 {
            places.push([a[0] + (b[0] - a[0]) * left / length, a[1] + (b[1] - a[1]) * left / length]);
            left += spacing;
        }
        left -= length;
    }
    places.truncate(most);
    places
}

/// The outline of everything within `r` of the segment a-b (which has length), its round ends as a few
/// straight sides.
pub(super) fn stroke(a: Pt, b: Pt, r: f64) -> Vec<Pt> {
    const SIDES: usize = 6; // of each round end
    let along = (b[1] - a[1]).atan2(b[0] - a[0]);
    let end = |centre: Pt, from: f64| {
        (0..=SIDES).map(move |k| {
            let angle = from - std::f64::consts::PI * k as f64 / SIDES as f64;
            [centre[0] + r * angle.cos(), centre[1] + r * angle.sin()]
        })
    };
    let quarter = std::f64::consts::FRAC_PI_2;
    end(b, along + quarter).chain(end(a, along - quarter)).collect()
}

/// The smallest polygon without a hollow that holds some points.
pub(super) fn hull(mut points: Vec<Pt>) -> Vec<Pt> {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let turn = |o: Pt, a: Pt, b: Pt| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut ring: Vec<Pt> = Vec::new();
    for half in [points.clone(), points.into_iter().rev().collect()] {
        let from = ring.len();
        for p in half {
            while ring.len() >= from + 2 && turn(ring[ring.len() - 2], ring[ring.len() - 1], p) <= 0.0 {
                ring.pop();
            }
            ring.push(p);
        }
        ring.pop(); // (the last of each half is the first of the other)
    }
    ring
}

/// A polygon (its outline, then its holes; even-odd) with its sides sorted by the rows they pass, so that
/// what is asked about a place looks only at the sides near it.
pub(super) struct Piece<'a> {
    pub rings: &'a [Vec<Pt>],
    /// The box round all of it.
    pub bounds: Bounds,
    all: Vec<(Pt, Pt)>,
    /// Per row of the box, top to bottom: the sides that pass it, as indices into `all`.
    rows: Vec<Vec<u32>>,
    row_height: f64,
}

impl<'a> Piece<'a> {
    pub fn new(rings: &'a [Vec<Pt>]) -> Piece<'a> {
        let all: Vec<(Pt, Pt)> = rings.iter().flat_map(|ring| sides(ring)).collect();
        let bounds = bounds(rings.iter().flatten());
        let count = ((all.len() as f64).sqrt() as usize).clamp(1, 1024);
        let height = bounds[3] - bounds[1];
        let row_height = if height > 0.0 { height / count as f64 } else { 1.0 };
        let mut piece = Piece { rings, bounds, all, rows: vec![Vec::new(); count], row_height };
        for k in 0..piece.all.len() {
            let (a, b) = piece.all[k];
            for row in piece.row(a[1].min(b[1]))..=piece.row(a[1].max(b[1])) {
                piece.rows[row].push(k as u32);
            }
        }
        piece
    }

    fn row(&self, y: f64) -> usize {
        (((y - self.bounds[1]) / self.row_height).floor().max(0.0) as usize).min(self.rows.len() - 1)
    }

    /// True if a point is on it: the same answer as [`crate::geom::within`] gives for its rings.
    pub fn contains(&self, p: Pt) -> bool {
        let b = self.bounds;
        if !(b[0] <= p[0] && p[0] <= b[2] && b[1] <= p[1] && p[1] <= b[3]) {
            return false;
        }
        let mut within = false;
        for &k in &self.rows[self.row(p[1])] {
            let (a, b) = self.all[k as usize];
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) {
                within = !within;
            }
        }
        within
    }

    /// The sides of it whose boxes touch a box, each once.
    pub fn sides_by(&self, near: Bounds) -> impl Iterator<Item = (Pt, Pt)> + '_ {
        let (first, last) = (self.row(near[1]), self.row(near[3]));
        let reaches = meet(self.bounds, near);
        (first..=last).filter(move |_| reaches).flat_map(move |row| {
            self.rows[row].iter().filter_map(move |&k| {
                let (a, b) = self.all[k as usize];
                // (a side that passes several of the rows is given in the first of them)
                let given_in = self.row(a[1].min(b[1])).max(first);
                (given_in == row && meet(bounds([&a, &b]), near)).then_some((a, b))
            })
        })
    }

    /// The point of its edge nearest to `p`, and how far that is; of several as near, that of its first side.
    /// None for a polygon without sides.
    pub fn nearest_edge(&self, p: Pt) -> Option<(f64, Pt)> {
        // The rows are looked at from the one `p` is level with outwards, until those left lie further
        // off than the nearest side found.
        let level = self.row(p[1]);
        let off = |row: usize| {
            let top = self.bounds[1] + row as f64 * self.row_height;
            (top - p[1]).max(p[1] - top - self.row_height).max(0.0)
        };
        let mut best: Option<(f64, u32, Pt)> = None;
        for step in 0..self.rows.len() {
            let above = level.checked_sub(step);
            let below = Some(level + step).filter(|&row| step > 0 && row < self.rows.len());
            let worth = |row: &usize| best.map_or(true, |(least, ..)| off(*row) <= least + 1e-9);
            let rows: Vec<usize> = [above, below].into_iter().flatten().filter(worth).collect();
            if rows.is_empty() && (best.is_some() || (above.is_none() && below.is_none())) {
                break;
            }
            for &k in rows.iter().flat_map(|&row| &self.rows[row]) {
                let (a, b) = self.all[k as usize];
                let q = nearest_on_segment(p, a, b);
                let d = dist(p, q);
                if best.map_or(true, |(least, first, _)| d < least || (d == least && k < first)) {
                    best = Some((d, k, q));
                }
            }
        }
        best.map(|(d, _, q)| (d, q))
    }

    /// True if another polygon and this one touch: sides that meet, or one with a corner on the other. Two
    /// that lie edge to edge or corner to corner do: copper that touches is joined, as it is for KiCad.
    pub fn touches(&self, other: &[Vec<Pt>]) -> bool {
        let theirs = bounds(other.iter().flatten());
        if !meet(self.bounds, theirs) {
            return false;
        }
        if other.iter().flatten().any(|&p| self.contains(p)) {
            return true;
        }
        if self.sides_by(theirs).any(|(a, _)| within(other, a)) {
            return true;
        }
        let their_sides = || other.iter().flat_map(|ring| sides(ring));
        their_sides().any(|(c, d)| self.sides_by(bounds([&c, &d])).any(|(a, b)| crossing(a, b, c, d).is_some()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Pt> {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    #[test]
    fn places_along_a_ring_are_evenly_apart_and_no_more_than_asked() {
        let ring = square(0.0, 0.0, 10.0, 10.0);
        let all = along(&ring, 5.0, 100);
        assert_eq!(all, [[0., 0.], [5., 0.], [10., 0.], [10., 5.], [10., 10.], [5., 10.], [0., 10.], [0., 5.]]);
        assert_eq!(along(&ring, 0.1, 16).len(), 16); // 2.5 mm apart instead
        assert!(along(&[[0.0, 0.0], [0.0, 0.0]], 1.0, 8).is_empty());
    }

    #[test]
    fn boxes_are_apart_by_the_gap_between_them() {
        assert_eq!(apart([0., 0., 1., 1.], [4., 5., 6., 6.]), 5.0);
        assert_eq!(apart([0., 0., 1., 1.], [1., 0., 2., 1.]), 0.0);
        assert!(meet([0., 0., 1., 1.], [1., 1., 2., 2.]));
        assert!(!meet([0., 0., 1., 1.], [1.1, 0., 2., 1.]));
    }

    #[test]
    fn a_piece_tells_a_point_on_it_as_its_rings_do() {
        // an L with a hole, and many points across it
        let rings = vec![
            vec![[0., 0.], [10., 0.], [10., 4.], [4., 4.], [4., 10.], [0., 10.]],
            square(1.0, 1.0, 2.0, 2.0),
            square(6.0, 1.0, 8.5, 3.0),
        ];
        let piece = Piece::new(&rings);
        for j in -4..48 {
            for i in -4..48 {
                let p = [i as f64 * 0.25 + 0.013, j as f64 * 0.25 + 0.007];
                assert_eq!(piece.contains(p), within(&rings, p), "{p:?}");
            }
        }
    }

    #[test]
    fn the_nearest_edge_is_the_one_a_look_at_every_side_finds() {
        // a comb: many sides, in many rows
        let mut ring: Vec<Pt> = vec![[0.0, 0.0], [40.0, 0.0]];
        for k in (0..40).rev() {
            let x = k as f64;
            ring.extend([[x + 0.7, 3.0], [x + 0.7, 30.0 - x * 0.5], [x + 0.3, 30.0 - x * 0.5], [x + 0.3, 3.0]]);
        }
        let rings = vec![ring, square(10.0, 1.0, 12.0, 2.0)];
        let piece = Piece::new(&rings);
        for j in -6..40 {
            for i in -6..50 {
                let p = [i as f64 * 0.93 + 0.011, j as f64 * 0.97 + 0.013];
                let mut all: Option<(f64, Pt)> = None;
                for (a, b) in rings.iter().flat_map(|ring| sides(ring)) {
                    let q = nearest_on_segment(p, a, b);
                    if all.map_or(true, |(least, _)| dist(p, q) < least) {
                        all = Some((dist(p, q), q));
                    }
                }
                assert_eq!(piece.nearest_edge(p), all, "{p:?}");
            }
        }
    }

    #[test]
    fn a_hull_holds_its_points_and_has_no_hollow() {
        let ring = hull(vec![[0., 0.], [4., 0.], [4., 4.], [0., 4.], [2., 2.], [1., 3.], [4., 0.], [2., 0.]]);
        assert_eq!(ring, [[0., 0.], [4., 0.], [4., 4.], [0., 4.]]);
        assert_eq!(hull(vec![[1., 1.], [1., 1.]]), [[1., 1.]]);
    }

    #[test]
    fn the_sides_by_a_box_are_those_that_touch_it_each_once() {
        let rings = vec![square(0.0, 0.0, 10.0, 10.0), square(4.0, 4.0, 6.0, 6.0)];
        let piece = Piece::new(&rings);
        let near: Vec<_> = piece.sides_by([3.5, 3.5, 4.5, 6.5]).collect();
        // the left side of the hole, and its top and bottom where they begin
        assert_eq!(near.len(), 3);
        assert_eq!(piece.sides_by([-1.0, -1.0, 11.0, 11.0]).count(), 8);
        assert_eq!(piece.sides_by([20.0, 0.0, 21.0, 1.0]).count(), 0);
    }

    #[test]
    fn polygons_touch_where_they_have_ground_in_common() {
        let rings = vec![square(0.0, 0.0, 10.0, 10.0), square(4.0, 4.0, 6.0, 6.0)];
        let piece = Piece::new(&rings);
        assert!(piece.touches(&[square(1.0, 1.0, 2.0, 2.0)])); // all on it
        assert!(piece.touches(&[square(9.0, 9.0, 12.0, 12.0)])); // across its edge
        assert!(piece.touches(&[square(-1.0, -1.0, 11.0, 11.0)])); // all round it
        assert!(piece.touches(&[square(3.0, 4.5, 7.0, 5.5)])); // across the hole: no corner of either on the other
        assert!(!piece.touches(&[square(4.5, 4.5, 5.5, 5.5)])); // in the hole
        assert!(!piece.touches(&[square(11.0, 0.0, 12.0, 1.0)])); // beside it
    }

    #[test]
    fn polygons_that_lie_against_each_other_touch() {
        // (a spoke that ends on the edge of its pad joins it)
        let rings = vec![square(0.0, 0.0, 10.0, 10.0)];
        let piece = Piece::new(&rings);
        assert!(piece.touches(&[square(10.0, 4.5, 11.0, 5.5)])); // edge to edge
        assert!(piece.touches(&[square(-1.0, 4.5, 0.0, 5.5)]));
        assert!(piece.touches(&[square(10.0, 10.0, 11.0, 11.0)])); // corner to corner
        assert!(piece.touches(&[vec![[10.0, 5.0], [11.0, 4.0], [12.0, 5.0], [11.0, 6.0]]])); // a corner on its edge
        assert!(!piece.touches(&[square(10.001, 4.5, 11.0, 5.5)])); // a thousandth off
    }

    #[test]
    fn a_polygon_without_height_or_without_points_is_no_ground() {
        let flat = vec![vec![[0.0, 1.0], [5.0, 1.0], [9.0, 1.0]]];
        assert!(!Piece::new(&flat).contains([2.0, 1.0]));
        let none: Vec<Vec<Pt>> = vec![vec![]];
        let piece = Piece::new(&none);
        assert!(!piece.contains([0.0, 0.0]) && piece.nearest_edge([0.0, 0.0]).is_none());
        assert!(!piece.touches(&[square(0.0, 0.0, 1.0, 1.0)]));
    }

    #[test]
    fn a_stroke_holds_what_is_near_the_segment() {
        let ring = stroke([0.0, 0.0], [4.0, 0.0], 1.0);
        assert!(within(std::slice::from_ref(&ring), [2.0, 0.9]));
        assert!(within(std::slice::from_ref(&ring), [4.9, 0.0]));
        assert!(within(std::slice::from_ref(&ring), [-0.9, 0.0]));
        assert!(!within(std::slice::from_ref(&ring), [2.0, 1.1]));
        assert!(!within(std::slice::from_ref(&ring), [5.1, 0.0]));
    }

    #[test]
    fn copper_hangs_together_where_an_end_lies_on_the_other_on_its_layer() {
        let (a, b): (&[Pt], &[Pt]) = (&[[0., 0.], [5., 0.]], &[[2., 0.], [2., 3.]]);
        assert!(hang_together((Some("F"), a), (Some("F"), b))); // a branch from the middle
        assert!(!hang_together((Some("F"), a), (Some("B"), b)));
        assert!(!hang_together((Some("F"), a), (Some("F"), &[[2., 1.], [2., 3.]])));
        assert!(hang_together((None, &[[2., 0.]]), (Some("B"), a))); // a via on a track of any layer
        assert!(hang_together((None, &[[2., 0.]]), (None, &[[2., 0.]])));
        assert!(!hang_together((None, &[[2., 0.5]]), (Some("F"), a)));
        assert!(!hang_together((Some("F"), &[]), (Some("F"), a)));
    }

    #[test]
    fn segments_cross_where_both_are_and_not_when_parallel() {
        assert_eq!(crossing([0., 0.], [4., 0.], [1., -1.], [1., 1.]), Some((0.25, 0.5)));
        assert_eq!(crossing([0., 0.], [4., 0.], [5., -1.], [5., 1.]), None);
        assert_eq!(crossing([0., 0.], [4., 0.], [0., 1.], [4., 1.]), None);
    }
}
