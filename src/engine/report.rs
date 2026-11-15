//! From the engine's cells to the tracks and vias of a solution.

use super::ends::Run;
use super::*;
use crate::geom::{dist, push};
use crate::grid::round6;

/// What an entry is of a pair, if anything: (pair, leg) for a stub, (pair, None) for the trunk.
type Part = Option<(usize, Option<usize>)>;

impl Engine<'_> {
    /// What the attempt has laid, as a solution, and what attempts that end alike are ranked by (the score,
    /// which the solution itself carries only if it is complete).
    pub(super) fn report(&mut self, status: Status, iterations: usize, variant: usize) -> (Solution, f64) {
        let p = self.p;
        let mut nets = Vec::new();
        let mut conflicts = Vec::new();
        let mut score = 0.0;
        // entries that are part of a pair are not reported by themselves
        let part = self.parts();
        // per pair and leg: the tracks and vias of its stubs
        let mut pieces: Vec<[(Vec<Track>, Vec<Pt>); 2]> =
            p.pairs.iter().map(|_| [(Vec::new(), Vec::new()), (Vec::new(), Vec::new())]).collect();
        // a trunk that has been split has given up its claim: its legs stand for it
        let split: Vec<bool> =
            part.iter().map(|part| matches!(part, Some((k, None)) if self.pairs.legs[*k].is_some())).collect();
        let mut runs: Vec<Vec<Run>> = (0..p.nets.len()).map(|n| self.runs(n)).collect();
        self.meet_exactly(&mut runs, &split);
        for ((n, net), runs) in p.nets.iter().enumerate().zip(runs) {
            let split = split[n];
            let tracks: Vec<Track> = runs
                .into_iter()
                .filter(|run| run.points.len() > 1)
                .map(|run| Track { layer: p.layers[run.layer].clone(), points: run.points })
                .collect();
            let length = length(&tracks);
            // lower is better: weighted length, a via as 1 mm, and each mm over the wanted length as 100 mm
            score += net.weight * (length + 100.0 * net.max_mm.map_or(0.0, |m| (length - m).max(0.0)))
                + self.laid.nets[n].vias.len() as f64;
            if !split {
                self.list_conflicts(n, &mut conflicts);
            }
            let vias: Vec<Pt> = self.laid.nets[n].vias.iter().map(|&v| self.plan.grid.xy(v as usize)).collect();
            match part[n] {
                Some((k, Some(leg))) => {
                    pieces[k][leg].0.extend(tracks);
                    pieces[k][leg].1.extend(vias);
                }
                // a trunk stands for nothing by itself once split, or once taken out because it could not be
                Some((_, None)) if split || self.laid.nets[n].conns.is_empty() => {}
                _ => nets.push(NetResult {
                    key: net.key.clone(),
                    net: net.net.clone(),
                    length_mm: round6(length),
                    tracks,
                    vias,
                }),
            }
        }
        self.pair_nets(pieces, &mut nets);
        let mut unrouted: Vec<String> = self.laid.nets.iter().flat_map(|n| n.failed.iter().cloned()).collect();
        unrouted.extend(self.laid.unlaid.iter().cloned());
        let status = if !unrouted.is_empty() {
            Status::Unroutable
        } else if conflicts.is_empty() || status == Status::Unroutable {
            status
        } else {
            Status::Conflicts
        };
        let message = unrouted.first().cloned().unwrap_or_default();
        let measure = round6(score);
        let solution = Solution {
            status,
            iterations,
            variant,
            score: (status == Status::Routed).then_some(measure),
            nets,
            conflicts,
            message,
            unrouted,
            attempts: Vec::new(),
            stats: Stats::default(),
        };
        (solution, measure)
    }

    /// Per entry: what it is of a pair.
    fn parts(&self) -> Vec<Part> {
        let mut part: Vec<Part> = vec![None; self.p.nets.len()];
        for (k, pair) in self.p.pairs.iter().enumerate() {
            if let Ok(t) = self.index(&pair.trunk) {
                part[t] = Some((k, None));
            }
            for stubs in &pair.stubs {
                for (leg, key) in stubs.iter().enumerate() {
                    if let Ok(n) = self.index(key) {
                        part[n] = Some((k, Some(leg)));
                    }
                }
            }
        }
        part
    }

    /// The connections of a laid net as runs: one per stretch on one layer, through the cell centres, each
    /// end with the exact point of what the net has there on that layer (a terminal, a hole, the end of a
    /// pair's leg), if it has one. A stretch of one cell is a run too where it has such a point: the pad the
    /// net changes layer in.
    fn runs(&self, n: usize) -> Vec<Run> {
        let (space, grid) = (self.plan.space, &self.plan.grid);
        let fixed = &self.plan.fixed_free[n];
        let exact = &self.exact[n];
        let terms = || self.plan.terms[n].iter().flatten().chain(&self.pairs.extra_terms[n]);
        // (a cell that stands on its exact point needs nothing; a pad of another layer is not met on this one)
        let point = |layer: usize, cell: usize| {
            let here = fixed.contains(&(cell as u32)) || terms().any(|t| t.cell as usize == cell && t.on(layer));
            exact.get(&(cell as u32)).copied().filter(|e| here && dist(*e, grid.xy(cell)) > 1e-9)
        };
        let mut runs: Vec<Run> = Vec::new();
        let mut end = |layer: usize, points: Vec<Pt>, want: [Option<Pt>; 2]| {
            if points.len() > 1 || want[0].is_some() {
                runs.push(Run { layer, points, want });
            }
        };
        for conn in &self.laid.nets[n].conns {
            let (mut layer, first) = space.split(conn[0] as usize);
            let mut from = point(layer, first);
            let mut points: Vec<Pt> = vec![grid.xy(first)];
            for pair in conn.windows(2) {
                let ((la, ca), (lb, cb)) = (space.split(pair[0] as usize), space.split(pair[1] as usize));
                if la != lb {
                    let last = if points.len() > 1 { point(la, ca) } else { None };
                    end(layer, std::mem::take(&mut points), [from, last]);
                    points = vec![grid.xy(ca)];
                    from = point(lb, ca);
                    layer = lb;
                } else {
                    push(&mut points, grid.xy(cb));
                }
            }
            let last = space.split(*conn.last().unwrap() as usize).1;
            let to = if points.len() > 1 { point(layer, last) } else { None };
            end(layer, points, [from, to]);
        }
        runs
    }

    /// Add where net `n` stands in ground another claims, with the nets that claim it; a track conflict once
    /// per set of other nets.
    fn list_conflicts(&mut self, n: usize, conflicts: &mut Vec<Conflict>) {
        let t = self.trouble(n);
        let (p, space, grid) = (self.p, self.plan.space, &self.plan.grid);
        let key = &p.nets[n].key;
        let others = |claims: &dyn Fn(usize) -> bool| -> Vec<String> {
            (0..p.nets.len()).filter(|&m| m != n && claims(m)).map(|m| p.nets[m].key.clone()).collect()
        };
        let (tclass, vclass) = (self.plan.tclass[n], self.plan.vclass[n]);
        let mut seen_with: Vec<Vec<String>> = Vec::new();
        for &s in &t.track {
            let with = others(&|m| self.laid.nets[m].claims_t.get(tclass).is_some_and(|c| c.binary_search(&s).is_ok()));
            if !seen_with.contains(&with) {
                seen_with.push(with.clone());
                let (layer, cell) = space.split(s as usize);
                conflicts.push(Conflict {
                    kind: ConflictKind::Track,
                    key: key.clone(),
                    with,
                    layer: Some(p.layers[layer].clone()),
                    xy: grid.xy(cell),
                });
            }
        }
        for &v in &t.via {
            let with = others(&|m| self.laid.nets[m].claims_v.get(vclass).is_some_and(|c| c.binary_search(&v).is_ok()));
            conflicts.push(Conflict {
                kind: ConflictKind::Via,
                key: key.clone(),
                with,
                layer: None,
                xy: grid.xy(v as usize),
            });
        }
        for &v in &t.own {
            conflicts.push(Conflict {
                kind: ConflictKind::OwnVias,
                key: key.clone(),
                with: Vec::new(),
                layer: None,
                xy: grid.xy(v as usize),
            });
        }
    }

    /// Add each net of each split pair: its leg and its two stubs (`pieces`, per pair and leg).
    fn pair_nets(&self, mut pieces: Vec<[(Vec<Track>, Vec<Pt>); 2]>, nets: &mut Vec<NetResult>) {
        let p = self.p;
        for (k, pair) in p.pairs.iter().enumerate() {
            if let Some((layer, lines)) = &self.pairs.legs[k] {
                for leg in 0..2 {
                    let mut tracks = vec![Track { layer: p.layers[*layer].clone(), points: lines[leg].clone() }];
                    tracks.append(&mut pieces[k][leg].0);
                    nets.push(NetResult {
                        key: format!("{}/{}", pair.key, pair.legs[leg]),
                        net: pair.legs[leg].clone(),
                        length_mm: round6(length(&tracks)),
                        tracks,
                        vias: std::mem::take(&mut pieces[k][leg].1),
                    });
                }
            }
        }
    }
}

/// Length of some tracks together.
fn length(tracks: &[Track]) -> f64 {
    // (the sum of no numbers is -0.0, which would be written so)
    tracks.iter().map(|t| t.points.windows(2).map(|s| dist(s[0], s[1])).sum::<f64>()).sum::<f64>() + 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_is_summed_over_all_tracks() {
        let track = |points: &[Pt]| Track { layer: "F".to_string(), points: points.to_vec() };
        let tracks =
            [track(&[[0.0, 0.0], [3.0, 4.0], [3.0, 6.0]]), track(&[[1.0, 1.0], [2.0, 1.0]]), track(&[[5.0, 5.0]])];
        assert_eq!(length(&tracks), 8.0);
        assert_eq!(length(&[]), 0.0);
    }
}
