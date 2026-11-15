//! From the engine's cells to the tracks and vias of a solution.

use super::*;
use crate::geom::dist;
use crate::grid::round6;

/// What an entry is of a pair, if anything: (pair, leg) for a stub, (pair, None) for the trunk.
type Part = Option<(usize, Option<usize>)>;

impl Engine<'_> {
    pub(super) fn report(&mut self, status: Status, iterations: usize, variant: usize, message: String) -> Solution {
        let p = self.p;
        let mut nets = Vec::new();
        let mut conflicts = Vec::new();
        let mut score = 0.0;
        // entries that are part of a pair are not reported by themselves
        let part = self.parts();
        // per pair and leg: the tracks and vias of its stubs
        let mut pieces: Vec<[(Vec<Track>, Vec<Pt>); 2]> =
            p.pairs.iter().map(|_| [(Vec::new(), Vec::new()), (Vec::new(), Vec::new())]).collect();
        for (n, net) in p.nets.iter().enumerate() {
            let tracks = self.tracks(n);
            let length = length(&tracks);
            // lower is better: weighted length, a via as 1 mm, and each mm over the wanted length as 100 mm
            score += net.weight * (length + 100.0 * net.max_mm.map_or(0.0, |m| (length - m).max(0.0)))
                + self.laid.nets[n].vias.len() as f64;
            // a trunk that has been split has given up its claim: its legs stand for it
            let split = matches!(part[n], Some((k, None)) if self.pairs.legs[k].is_some());
            if !split {
                self.list_conflicts(n, &mut conflicts);
            }
            let vias: Vec<Pt> = self.laid.nets[n].vias.iter().map(|&v| self.plan.grid.xy(v as usize)).collect();
            match part[n] {
                Some((k, Some(leg))) => {
                    pieces[k][leg].0.extend(tracks);
                    pieces[k][leg].1.extend(vias);
                }
                Some((_, None)) if split => {}
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
        let message = if message.is_empty() { unrouted.first().cloned().unwrap_or_default() } else { message };
        let score = if status == Status::Routed { Some(round6(score)) } else { None };
        Solution {
            status,
            iterations,
            variant,
            score,
            nets,
            conflicts,
            message,
            unrouted,
            attempts: Vec::new(),
            stats: Stats::default(),
        }
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

    /// The connections of a laid net as tracks: one per stretch on one layer, through the cell centres, ending
    /// at the exact point of a terminal or hole where the net has one.
    fn tracks(&self, n: usize) -> Vec<Track> {
        let (space, p, grid) = (self.plan.space, self.p, &self.plan.grid);
        let fixed = &self.plan.fixed_free[n];
        let exact = &self.exact[n];
        let point = |cell: u32| exact.get(&cell).copied();
        let mut tracks: Vec<Track> = Vec::new();
        for conn in &self.laid.nets[n].conns {
            let (mut layer, first) = space.split(conn[0] as usize);
            let mut run: Vec<Pt> = Vec::new();
            if let Some(e) = point(first as u32) {
                run.push(e);
            }
            push(&mut run, grid.xy(first));
            for pair in conn.windows(2) {
                let ((la, ca), (lb, cb)) = (space.split(pair[0] as usize), space.split(pair[1] as usize));
                if la != lb {
                    let cell = ca as u32;
                    let at = if fixed.contains(&cell) { point(cell).unwrap_or(grid.xy(ca)) } else { grid.xy(ca) };
                    push(&mut run, at);
                    if run.len() > 1 {
                        tracks.push(Track { layer: p.layers[layer].clone(), points: std::mem::take(&mut run) });
                    }
                    run = vec![at];
                    layer = lb;
                } else {
                    push(&mut run, grid.xy(cb));
                }
            }
            let last = space.split(*conn.last().unwrap() as usize).1;
            if let Some(e) = point(last as u32) {
                push(&mut run, e);
            }
            if run.len() > 1 {
                tracks.push(Track { layer: p.layers[layer].clone(), points: run });
            }
        }
        tracks
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
    tracks.iter().map(|t| t.points.windows(2).map(|s| dist(s[0], s[1])).sum::<f64>()).sum()
}

/// Append a point, dropping repeats and the middle point of three in a straight line (same direction).
pub(super) fn push(run: &mut Vec<Pt>, p: Pt) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_in_a_straight_line_are_one_segment() {
        let mut run = Vec::new();
        for q in [[0.0, 0.0], [0.1, 0.0], [0.1, 0.0], [0.2, 0.0], [0.3, 0.1], [0.4, 0.2], [0.4, 0.3]] {
            push(&mut run, q);
        }
        assert_eq!(run, vec![[0.0, 0.0], [0.2, 0.0], [0.4, 0.2], [0.4, 0.3]]);
    }

    #[test]
    fn length_is_summed_over_all_tracks() {
        let track = |points: &[Pt]| Track { layer: "F".to_string(), points: points.to_vec() };
        let tracks =
            [track(&[[0.0, 0.0], [3.0, 4.0], [3.0, 6.0]]), track(&[[1.0, 1.0], [2.0, 1.0]]), track(&[[5.0, 5.0]])];
        assert_eq!(length(&tracks), 8.0);
        assert_eq!(length(&[]), 0.0);
    }
}
