//! Where a track meets the exact place of a pad or hole.
//!
//! The search runs through cell centres, and a pad seldom stands on one. Joining the last cell to the pad's own
//! point with a piece of track leaves a crooked stub, shorter than the track is wide. Here the track is ended
//! on the point without one where it can be: its last corner slides along the stretch that leads to it, so
//! that both stretches keep their directions, or a track of one stretch is turned onto the point. What moves
//! is tested against everything the track must keep away from, as it lies and off the grid; where it may not
//! move, or something of the net joins it where it would, the stub is laid as before.

use super::report::push;
use super::*;
use crate::geom::{dist, dist_segment, dist_segments};

/// A stretch of track of a net on one layer, and for each of its ends the exact place it is to end at where
/// that is not the cell centre it ends on.
pub(super) struct Run {
    pub(super) layer: usize,
    pub(super) points: Vec<Pt>,
    pub(super) want: [Option<Pt>; 2],
}

/// The runs of all entries as they lie, and per entry whether its runs are copper of the result (those of a
/// trunk that has been split are not).
struct Copper<'a> {
    runs: &'a [Vec<Run>],
    live: &'a [bool],
}

/// The points with the last moved to `to` and no corner added: the last corner slid along the stretch before
/// it, or, of two points, the one moved. None if there is a corner and it cannot slide: the two last stretches
/// are in line, or one of them would become shorter than `least`.
fn moved(points: &[Pt], to: Pt, least: f64) -> Option<Vec<Pt>> {
    let n = points.len();
    let mut out = points.to_vec();
    out[n - 1] = to;
    if n == 2 {
        return (dist(points[0], to) >= least).then_some(out);
    }
    let (u, v, c) = (points[n - 3], points[n - 2], points[n - 1]);
    let (l1, l2) = (dist(v, c), dist(u, v));
    let (d1, d2) = ([(c[0] - v[0]) / l1, (c[1] - v[1]) / l1], [(v[0] - u[0]) / l2, (v[1] - u[1]) / l2]);
    let cross = d1[0] * d2[1] - d1[1] * d2[0];
    if cross.abs() < 1e-9 {
        return None;
    }
    // the way from the cell to the point, as so much along the last stretch and so much along the one before
    let by = [to[0] - c[0], to[1] - c[1]];
    let along_last = (by[0] * d2[1] - by[1] * d2[0]) / cross;
    let along_before = (d1[0] * by[1] - d1[1] * by[0]) / cross;
    if l1 + along_last < least || l2 + along_before < least {
        return None;
    }
    out[n - 2] = [v[0] + along_before * d2[0], v[1] + along_before * d2[1]];
    Some(out)
}

/// True if `q` lies on the segment a-b.
fn on(q: Pt, a: Pt, b: Pt) -> bool {
    dist_segment(q, a, b) < 1e-6
}

impl Engine<'_> {
    /// End every run on the exact places it wants, moving it where that is allowed and with a stub where not.
    /// `live`: per entry, whether its runs are copper of the result (those of a trunk that has been split are not).
    pub(super) fn meet_exactly(&self, runs: &mut [Vec<Run>], live: &[bool]) {
        // a stretch shorter than this is not worth keeping for the sake of its direction
        let least = self.plan.grid.step / 2.0;
        for n in 0..runs.len() {
            // where other copper of the net joins: there a track must stay
            let joins: Vec<Pt> = if live[n] { self.joins(n, runs) } else { Vec::new() };
            for r in 0..runs[n].len() {
                for end in 0..2 {
                    let Some(to) = runs[n][r].want[end] else { continue };
                    let mut points = runs[n][r].points.clone();
                    if end == 0 {
                        points.reverse();
                    }
                    let layer = runs[n][r].layer;
                    let copper = Copper { runs, live };
                    let new = moved(&points, to, least)
                        .filter(|new| live[n] && self.may_move(n, layer, &points, new, &joins, &copper));
                    match new {
                        Some(new) => points = new,
                        None => push(&mut points, to),
                    }
                    if end == 0 {
                        points.reverse();
                    }
                    runs[n][r].points = points;
                }
            }
        }
    }

    /// Where copper of net entry `n` or of another entry of the same net begins or ends: the ends of their runs
    /// and their vias.
    fn joins(&self, n: usize, runs: &[Vec<Run>]) -> Vec<Pt> {
        let mut out = Vec::new();
        for m in std::iter::once(n).chain(self.plan.siblings[n].iter().copied()) {
            for run in &runs[m] {
                out.push(run.points[0]);
                out.push(*run.points.last().unwrap());
            }
            out.extend(self.laid.nets[m].vias.iter().map(|&v| self.plan.grid.xy(v as usize)));
        }
        out
    }

    /// Whether a run of net `n` may lie as `new` in place of `old` (both end alike but for the last point and
    /// the corner before it): nothing of the net joins it on what moves, and what has moved keeps the net's
    /// distances from the fixed copper and from the other nets, or at least the distance it kept before.
    fn may_move(&self, n: usize, layer: usize, old: &[Pt], new: &[Pt], joins: &[Pt], copper: &Copper) -> bool {
        let k = old.len();
        let (was, is) = (&old[k.saturating_sub(3)..], &new[k.saturating_sub(3)..]);
        let (corner, cell) = (was[was.len() - 2], was[was.len() - 1]);
        let held = |q: Pt| {
            if k == 2 {
                // the one stretch turns about its first point: nothing may join it between its ends
                on(q, corner, cell) && dist(q, corner) > 1e-6 && dist(q, cell) > 1e-6
            } else {
                // the last stretch moves sideways, corner and all; of the one before, what is cut off goes
                let slid = is[is.len() - 2];
                (on(q, corner, cell) && dist(q, cell) > 1e-6) || (on(slid, was[0], corner) && on(q, corner, slid))
            }
        };
        !joins.iter().any(|q| held(*q)) && is.windows(2).all(|s| self.clear(n, layer, [s[0], s[1]], was, copper))
    }

    /// Whether a track of net `n` from `a` to `b` on a layer keeps its distances: from the fixed copper all
    /// the way, and from each other net's copper as it lies, or at least the distance `was` kept from it (the
    /// grid lets a slanted step come a hair nearer than the rule, and what was accepted stays accepted).
    fn clear(&self, n: usize, layer: usize, [a, b]: [Pt; 2], was: &[Pt], copper: &Copper) -> bool {
        let Copper { runs, live } = *copper;
        let (p, grid) = (self.p, &self.plan.grid);
        // tested every eighth of a step: between two tests the track comes no nearer than a hair
        let pieces = (dist(a, b) / (grid.step / 8.0)).ceil().max(1.0) as usize;
        let open = (0..=pieces).all(|i| {
            let f = i as f64 / pieces as f64;
            let at = [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f];
            self.shared.fixed.mask_at(n, at).is_some_and(|mask| mask & (1 << layer) == 0)
        });
        if !open {
            return false;
        }
        let rules = &p.rules;
        let quiet = if rules.quiet_clearance_mm > 0.0 { rules.quiet_clearance_mm } else { rules.new_clearance_mm };
        let half = p.nets[n].width / 2.0;
        // far enough from a segment of half width `other`, or no nearer than before
        let far = |c: Pt, d: Pt, need: f64| {
            let now = dist_segments(a, b, c, d);
            now >= need - 1e-9
                || now >= was.windows(2).map(|s| dist_segments(s[0], s[1], c, d)).fold(f64::MAX, f64::min) - 1e-9
        };
        (0..runs.len()).filter(|&m| m != n && live[m] && !self.plan.siblings[n].contains(&m)).all(|m| {
            let net = &p.nets[m];
            let gap = if net.quiet != p.nets[n].quiet { quiet } else { rules.new_clearance_mm };
            let tracks = runs[m]
                .iter()
                .filter(|run| run.layer == layer)
                .all(|run| run.points.windows(2).all(|s| far(s[0], s[1], half + gap + net.width / 2.0)));
            let vias = self.laid.nets[m].vias.iter().all(|&v| {
                let at = grid.xy(v as usize);
                far(at, at, half + gap + net.via[0] / 2.0)
            });
            let shapes = self.laid.nets[m]
                .shapes
                .iter()
                .filter(|shape| shape.0 == layer)
                .all(|&(_, c, d, other)| far(c, d, half + gap + other));
            tracks && vias && shapes
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: Pt, b: Pt) -> bool {
        dist(a, b) < 1e-9
    }

    #[test]
    fn the_last_corner_slides_so_that_both_stretches_keep_their_directions() {
        // along x, then up; the pad is a little right of and above the last cell
        let new = moved(&[[0.0, 0.0], [2.0, 0.0], [2.0, 1.0]], [2.03, 1.02], 0.05).unwrap();
        assert!(near(new[0], [0.0, 0.0]) && near(new[1], [2.03, 0.0]) && near(new[2], [2.03, 1.02]), "{new:?}");
        // along x, then slanted
        let new = moved(&[[0.0, 0.0], [2.0, 0.0], [3.0, 1.0]], [3.0, 1.04], 0.05).unwrap();
        assert!(near(new[1], [1.96, 0.0]) && near(new[2], [3.0, 1.04]), "{new:?}");
    }

    #[test]
    fn a_point_short_of_the_cell_shortens_the_stretch_and_does_not_turn_back() {
        let new = moved(&[[0.0, 0.0], [2.0, 0.0], [2.0, 1.0]], [2.0, 0.97], 0.05).unwrap();
        assert!(near(new[1], [2.0, 0.0]) && near(new[2], [2.0, 0.97]), "{new:?}");
    }

    #[test]
    fn a_stretch_is_not_slid_away() {
        assert!(moved(&[[0.0, 0.0], [2.0, 0.0], [2.0, 0.05]], [2.0, 0.02], 0.05).is_none());
        assert!(moved(&[[1.99, 0.0], [2.0, 0.0], [2.0, 1.0]], [1.97, 1.0], 0.05).is_none());
    }

    #[test]
    fn a_run_of_one_stretch_turns_onto_the_point() {
        let new = moved(&[[0.0, 0.0], [2.0, 0.0]], [2.02, 0.03], 0.05).unwrap();
        assert!(near(new[0], [0.0, 0.0]) && near(new[1], [2.02, 0.03]));
    }
}
