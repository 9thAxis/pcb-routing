//! Where a track meets the exact place of a pad or hole.
//!
//! The search runs through cell centres, and a pad seldom stands on one. Joining the last cell to the pad's own
//! point with a piece of track leaves a crooked stub, shorter than the track is wide. Here the track is ended
//! on the point without one where it can be: its last corner slides along the stretch that leads to it, so
//! that both stretches keep their directions, or a track of one stretch is turned onto the point. What moves
//! is tested against everything the track must keep away from, as it lies and off the grid. Where it may not
//! move, or something of the net joins it where it would, the stub is laid, and where the stub would come too
//! near another net the track ends on its cell, which is on the pad too.

use super::*;
use crate::geom::{dist, dist_segment, dist_segments, inside, push};
use crate::grid::round6;

/// A stretch of track of a net on one layer, and for each of its ends the exact place it is to end at where
/// that is not the cell centre it ends on. Of one point: no track yet, only the cell a point is to be met from.
pub(super) struct Run {
    pub(super) layer: usize,
    pub(super) points: Vec<Pt>,
    pub(super) want: [Option<Pt>; 2],
}

/// The runs of all entries as they lie, and per entry whether its runs are no copper of the result (those of
/// a trunk that has been split).
struct Copper<'a> {
    runs: &'a [Vec<Run>],
    dead: &'a [bool],
}

/// A place where copper of a net begins or ends, which a track that passes there must keep passing.
struct Join {
    at: Pt,
    /// The layer of a run's end; None for a via, which is on all.
    layer: Option<usize>,
    /// The end of a run that is itself to be ended on the exact place of its cell.
    follows: bool,
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
    // (stretches of the grid are 45 degrees apart at least; nearly in line, the corner would slide far)
    if cross.abs() < 0.5 {
        return None;
    }
    // the way from the cell to the point, as so much along the last stretch and so much along the one before
    let by = [to[0] - c[0], to[1] - c[1]];
    let along_last = (by[0] * d2[1] - by[1] * d2[0]) / cross;
    let along_before = (d1[0] * by[1] - d1[1] * by[0]) / cross;
    if l1 + along_last < least || l2 + along_before < least {
        return None;
    }
    out[n - 2] = [round6(v[0] + along_before * d2[0]), round6(v[1] + along_before * d2[1])];
    Some(out)
}

/// True if `q` lies on the line through `points`.
fn on(q: Pt, points: &[Pt]) -> bool {
    points.windows(2).any(|s| dist_segment(q, s[0], s[1]) < 1e-6)
}

impl Engine<'_> {
    /// End every run on the exact places it wants: by moving it where that is allowed, with a stub where the
    /// stub is, and on its cell where neither is and the cell's centre is on the pad as well. `dead`: per entry, whether its runs are no copper of the
    /// result; those get their stubs unasked.
    pub(super) fn meet_exactly(&self, runs: &mut [Vec<Run>], dead: &[bool]) {
        let _t = self.shared.timed(8);
        // a stretch shorter than this is not worth keeping for the sake of its direction
        let least = self.plan.grid.step / 2.0;
        let joins: Vec<Vec<Join>> = (0..runs.len()).map(|n| self.joins(n, &runs[n])).collect();
        for n in 0..runs.len() {
            let mine = || std::iter::once(n).chain(self.plan.siblings[n].iter().copied()).flat_map(|m| &joins[m]);
            for r in 0..runs[n].len() {
                for end in 0..2 {
                    // (taken: what is still wanted of the other nets stands in this net's way, see `clear`)
                    let Some(to) = runs[n][r].want[end].take() else { continue };
                    let layer = runs[n][r].layer;
                    let mut points = runs[n][r].points.clone();
                    if end == 0 {
                        points.reverse();
                    }
                    let copper = Copper { runs, dead };
                    let cell = points[points.len() - 1];
                    let was = &points[points.len().saturating_sub(3)..];
                    let new = (points.len() > 1 && !dead[n])
                        .then(|| moved(&points, to, least))
                        .flatten()
                        .filter(|new| self.may_move(n, layer, was, &new[points.len() - was.len()..], mine(), &copper));
                    // Where the cell's centre is on the pad too, the track is joined there as well, and the stub
                    // is left out if it would come too near another net. The end of a pair's leg is met in any case.
                    let in_pad = self.in_pad(n, layer, cell, to);
                    match new {
                        Some(new) => points = new,
                        None if dead[n] || !in_pad || self.clear(n, layer, [cell, to], was, &copper) => {
                            push(&mut points, to)
                        }
                        None => {}
                    }
                    if end == 0 {
                        points.reverse();
                    }
                    runs[n][r].points = points;
                }
            }
        }
    }

    /// Whether `to` is the place of a pad or hole of net `n` and one piece of the net's copper on the layer
    /// holds both it and the point `cell`: a track that ends on either is joined to the pad.
    fn in_pad(&self, n: usize, layer: usize, cell: Pt, to: Pt) -> bool {
        let (p, grid) = (self.p, &self.plan.grid);
        let planned = grid.cell(cell).is_some_and(|c| self.plan.exact[n].get(&(c as u32)) == Some(&to));
        let (net, name) = (p.nets[n].net.as_str(), &p.layers[layer]);
        let holds = |piece: &crate::model::Copper| {
            piece.net.as_deref() == Some(net)
                && piece.layers.contains(name)
                && inside(&piece.shape, cell)
                && inside(&piece.shape, to)
        };
        planned && p.copper.iter().any(holds)
    }

    /// Where copper of net entry `n` begins or ends: the ends of its runs and its vias.
    fn joins(&self, n: usize, runs: &[Run]) -> Vec<Join> {
        let ends = runs.iter().flat_map(|run| {
            let k = run.points.len() - 1;
            [(run.points[0], run.want[0]), (run.points[k], if k > 0 { run.want[1] } else { run.want[0] })]
                .map(|(at, want)| Join { at, layer: Some(run.layer), follows: want.is_some() })
        });
        let vias = self.laid.nets[n].vias.iter().map(|&v| self.plan.grid.xy(v as usize));
        ends.chain(vias.map(|at| Join { at, layer: None, follows: false })).collect()
    }

    /// Whether the end of a run of net `n` may lie as `is` where it lay as `was` (the last points of the run,
    /// three at most, the first of them unmoved). Nothing of the net may join it on what is taken away: at
    /// the cell it leaves, only ends that leave with it; elsewhere, nothing that the new line does not pass
    /// too. And what is new keeps the net's distances (see `clear`).
    fn may_move<'a>(
        &self,
        n: usize,
        layer: usize,
        was: &[Pt],
        is: &[Pt],
        mut joins: impl Iterator<Item = &'a Join>,
        copper: &Copper,
    ) -> bool {
        let cell = was[was.len() - 1];
        let held = |q: &Join| {
            if q.layer.is_some_and(|l| l != layer) {
                false
            } else if dist(q.at, cell) < 1e-6 {
                !q.follows
            } else {
                on(q.at, was) && !on(q.at, is)
            }
        };
        if joins.any(held) {
            return false;
        }
        // The last stretch is new all along. The one before only where it has grown past its old corner.
        let k = is.len();
        let last = self.clear(n, layer, [is[k - 2], is[k - 1]], was, copper);
        let grown = k == 3 && !on(is[1], &was[..2]);
        last && (!grown || self.clear(n, layer, [was[1], is[1]], was, copper))
    }

    /// Whether a track of net `n` from `a` to `b` on a layer keeps its distances: from the fixed copper all
    /// the way, and from each other net's copper as it lies, or at least the distance `was` kept from it (the
    /// grid lets a slanted step come a hair nearer than the rule, and what was accepted stays accepted). The
    /// stubs the other nets may yet lay to the places they want count as their copper.
    fn clear(&self, n: usize, layer: usize, [a, b]: [Pt; 2], was: &[Pt], copper: &Copper) -> bool {
        let Copper { runs, dead } = *copper;
        let (p, grid) = (self.p, &self.plan.grid);
        if !self.shared.fixed.open_along(n, a, b, 1 << layer) {
            return false;
        }
        let rules = &p.rules;
        let quiet = if rules.quiet_clearance_mm > 0.0 { rules.quiet_clearance_mm } else { rules.new_clearance_mm };
        let half = p.nets[n].width / 2.0;
        let (lo, hi) = ([a[0].min(b[0]), a[1].min(b[1])], [a[0].max(b[0]), a[1].max(b[1])]);
        // far enough from the segment c-d of other copper, or no nearer than before
        let far = |c: Pt, d: Pt, need: f64| {
            let beside = |k: usize| c[k].max(d[k]) < lo[k] - need || c[k].min(d[k]) > hi[k] + need;
            if beside(0) || beside(1) {
                return true;
            }
            let now = dist_segments(a, b, c, d);
            let before = || was.windows(2).map(|s| dist_segments(s[0], s[1], c, d)).fold(f64::MAX, f64::min);
            now >= need - 1e-9 || now >= before() - 1e-9
        };
        (0..runs.len()).filter(|&m| m != n && !dead[m] && !self.plan.siblings[n].contains(&m)).all(|m| {
            let net = &p.nets[m];
            let gap = if net.quiet != p.nets[n].quiet { quiet } else { rules.new_clearance_mm };
            let need = half + gap + net.width / 2.0;
            let tracks = runs[m].iter().filter(|run| run.layer == layer).all(|run| {
                let k = run.points.len() - 1;
                let stubs = [(0, run.want[0]), (k, if k > 0 { run.want[1] } else { None })];
                run.points.windows(2).all(|s| far(s[0], s[1], need))
                    && stubs.iter().all(|&(i, want)| want.map_or(true, |to| far(run.points[i], to, need)))
            });
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
    fn a_corner_between_stretches_nearly_in_line_does_not_slide() {
        // (the stretch before is a stub laid at the other end, at any angle: the corner would slide far)
        assert!(moved(&[[0.0, 0.02], [0.05, 0.0], [2.0, 0.0]], [2.03, 0.02], 0.05).is_none());
    }

    #[test]
    fn a_run_of_one_stretch_turns_onto_the_point() {
        let new = moved(&[[0.0, 0.0], [2.0, 0.0]], [2.02, 0.03], 0.05).unwrap();
        assert!(near(new[0], [0.0, 0.0]) && near(new[1], [2.02, 0.03]));
    }
}
