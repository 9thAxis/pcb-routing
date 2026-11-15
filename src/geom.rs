//! Plane geometry the router uses in more than one place.

use crate::model::{Pt, Shape};

/// Distance between two points.
#[inline]
pub(crate) fn dist(a: Pt, b: Pt) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Square of the distance from a point to the segment a-b (to the point `a` if the segment has no length).
#[inline]
pub(crate) fn dist2_segment(p: Pt, a: Pt, b: Pt) -> f64 {
    let (vx, vy) = (b[0] - a[0], b[1] - a[1]);
    let (wx, wy) = (p[0] - a[0], p[1] - a[1]);
    let l2 = vx * vx + vy * vy;
    let t = if l2 > 0.0 { ((wx * vx + wy * vy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (dx, dy) = (wx - t * vx, wy - t * vy);
    dx * dx + dy * dy
}

/// Distance from a point to the segment a-b.
#[inline]
pub(crate) fn dist_segment(p: Pt, a: Pt, b: Pt) -> f64 {
    dist2_segment(p, a, b).sqrt()
}

/// Distance between the segments a-b and c-d (either may be a point); 0 where they cross.
pub(crate) fn dist_segments(a: Pt, b: Pt, c: Pt, d: Pt) -> f64 {
    let side = |p: Pt, q: Pt, r: Pt| (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let (s1, s2, s3, s4) = (side(a, b, c), side(a, b, d), side(c, d, a), side(c, d, b));
    if s1 * s2 < 0.0 && s3 * s4 < 0.0 {
        return 0.0;
    }
    dist2_segment(a, c, d).min(dist2_segment(b, c, d)).min(dist2_segment(c, a, b)).min(dist2_segment(d, a, b)).sqrt()
}

/// Append a point to a line, dropping repeats and the middle point of three in a straight line (same direction).
pub(crate) fn push(run: &mut Vec<Pt>, p: Pt) {
    if let Some(last) = run.last() {
        if (last[0] - p[0]).abs() < 1e-9 && (last[1] - p[1]).abs() < 1e-9 {
            return;
        }
    }
    run.push(p);
    while run.len() >= 3 {
        let (a, b, c) = (run[run.len() - 3], run[run.len() - 2], run[run.len() - 1]);
        let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
        if (u[0] * v[1] - u[1] * v[0]).abs() < 1e-7 && u[0] * v[0] + u[1] * v[1] > 0.0 {
            let n = run.len();
            run.remove(n - 2);
        } else {
            break;
        }
    }
}

/// True if a point lies on a shape.
pub(crate) fn inside(shape: &Shape, p: Pt) -> bool {
    match shape {
        Shape::Seg { a, b, w } => dist2_segment(p, *a, *b) <= (w / 2.0) * (w / 2.0),
        Shape::Circle { c, r } => (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) <= r * r,
        Shape::Poly { rings } => {
            let mut within = false;
            for ring in rings {
                let n = ring.len();
                for k in 0..n {
                    let (a, b) = (ring[k], ring[(k + 1) % n]);
                    if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]) {
                        within = !within;
                    }
                }
            }
            within
        }
    }
}

/// Length of the shortest way between two cells `a` rows and `b` columns apart (both not negative) in the
/// eight directions, in grid steps.
#[inline]
pub(crate) fn octile(a: f32, b: f32) -> f32 {
    a.max(b) + (std::f32::consts::SQRT_2 - 1.0) * a.min(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_to_a_segment_is_to_its_nearest_point() {
        let (a, b) = ([0.0, 0.0], [4.0, 0.0]);
        assert_eq!(dist_segment([2.0, 3.0], a, b), 3.0); // beside the middle
        assert_eq!(dist_segment([-3.0, 4.0], a, b), 5.0); // beyond the first end
        assert_eq!(dist_segment([7.0, -4.0], a, b), 5.0); // beyond the second end
        assert_eq!(dist_segment([1.0, 0.0], a, b), 0.0); // on it
        assert_eq!(dist2_segment([2.0, 3.0], a, b), 9.0);
    }

    #[test]
    fn points_in_a_straight_line_are_one_segment() {
        let mut run = Vec::new();
        for q in [[0.0, 0.0], [0.1, 0.0], [0.1, 0.0], [0.2, 0.0], [0.3, 0.1], [0.4, 0.2], [0.4, 0.3]] {
            push(&mut run, q);
        }
        assert_eq!(run, vec![[0.0, 0.0], [0.2, 0.0], [0.4, 0.2], [0.4, 0.3]]);
    }

    #[test]
    fn distance_between_segments_is_between_their_nearest_points() {
        let (a, b) = ([0.0, 0.0], [4.0, 0.0]);
        assert_eq!(dist_segments(a, b, [1.0, 2.0], [3.0, 2.0]), 2.0); // side by side
        assert_eq!(dist_segments(a, b, [2.0, -1.0], [2.0, 1.0]), 0.0); // crossing
        assert_eq!(dist_segments(a, b, [2.0, 1.0], [2.0, 5.0]), 1.0); // end on
        assert_eq!(dist_segments(a, b, [7.0, 4.0], [9.0, 9.0]), 5.0); // end to end
        assert_eq!(dist_segments(a, b, [2.0, 3.0], [2.0, 3.0]), 3.0); // a point
        assert_eq!(dist_segments([1.0, 1.0], [1.0, 1.0], [4.0, 5.0], [4.0, 5.0]), 5.0);
        // two points
    }

    #[test]
    fn a_segment_without_length_is_a_point() {
        assert_eq!(dist_segment([3.0, 4.0], [0.0, 0.0], [0.0, 0.0]), 5.0);
        assert_eq!(dist([3.0, 4.0], [0.0, 0.0]), 5.0);
        assert_eq!(dist([0.0, 0.0], [3.0, 4.0]), 5.0);
    }

    #[test]
    fn a_point_is_inside_a_segment_shape_up_to_half_its_width() {
        let seg = Shape::Seg { a: [0.0, 0.0], b: [4.0, 0.0], w: 1.0 };
        assert!(inside(&seg, [2.0, 0.5])); // the edge counts
        assert!(inside(&seg, [-0.5, 0.0])); // the round end
        assert!(!inside(&seg, [2.0, 0.51]));
        assert!(!inside(&seg, [-0.4, 0.4]));
    }

    #[test]
    fn a_point_is_inside_a_circle_up_to_its_radius() {
        let circle = Shape::Circle { c: [1.0, 1.0], r: 0.5 };
        assert!(inside(&circle, [1.5, 1.0]));
        assert!(inside(&circle, [1.0, 1.0]));
        assert!(!inside(&circle, [1.4, 1.4]));
    }

    #[test]
    fn a_hole_in_a_polygon_is_outside() {
        let square = |lo: f64, hi: f64| vec![[lo, lo], [hi, lo], [hi, hi], [lo, hi]];
        let poly = Shape::Poly { rings: vec![square(0.0, 4.0), square(1.0, 3.0)] };
        assert!(inside(&poly, [0.5, 2.0]));
        assert!(!inside(&poly, [2.0, 2.0])); // in the hole
        assert!(!inside(&poly, [5.0, 2.0]));
        assert!(!inside(&poly, [2.0, -0.1]));
    }

    #[test]
    fn octile_is_straight_plus_slanted() {
        assert_eq!(octile(0.0, 0.0), 0.0);
        assert_eq!(octile(5.0, 0.0), 5.0);
        assert_eq!(octile(0.0, 5.0), 5.0);
        assert_eq!(octile(3.0, 3.0), 3.0 + (std::f32::consts::SQRT_2 - 1.0) * 3.0);
        assert_eq!(octile(2.0, 7.0), octile(7.0, 2.0));
    }
}
