//! From the engine's cells to the tracks and vias of a solution.

use super::*;
use crate::grid::round6;

impl Engine<'_> {
    pub(super) fn report(&mut self, status: Status, iterations: usize, variant: usize, message: String) -> Solution {
        let (hw, p) = (self.hw, self.p);
        let mut nets = Vec::new();
        let mut conflicts = Vec::new();
        let mut score = 0.0;
        // entries that are part of a pair are not reported by themselves: (pair, leg) for a stub, the pair for a trunk
        let mut part: Vec<Option<(usize, Option<usize>)>> = vec![None; p.nets.len()];
        for (k, pair) in p.pairs.iter().enumerate() {
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
        let mut pieces: Vec<[(Vec<Track>, Vec<Pt>); 2]> =
            p.pairs.iter().map(|_| [(Vec::new(), Vec::new()), (Vec::new(), Vec::new())]).collect();
        for (n, net) in p.nets.iter().enumerate() {
            let st = &self.nets[n];
            let fixed = &self.fixed_free[n];
            let exact = &self.exact[n];
            let point = |cell: u32| exact.get(&cell).copied();
            let mut tracks: Vec<Track> = Vec::new();
            for conn in &st.conns {
                let first = conn[0];
                let mut layer = first as usize / hw;
                let mut run: Vec<Pt> = Vec::new();
                if let Some(e) = point(first % hw as u32) {
                    run.push(e);
                }
                push(&mut run, self.grid.xy(first as usize % hw));
                for pair in conn.windows(2) {
                    let (a, b) = (pair[0] as usize, pair[1] as usize);
                    if a / hw != b / hw {
                        let cell = (a % hw) as u32;
                        let at = if fixed.contains(&cell) {
                            point(cell).unwrap_or(self.grid.xy(cell as usize))
                        } else {
                            self.grid.xy(cell as usize)
                        };
                        push(&mut run, at);
                        if run.len() > 1 {
                            tracks.push(Track { layer: p.layers[layer].clone(), points: std::mem::take(&mut run) });
                        }
                        run = vec![at];
                        layer = b / hw;
                    } else {
                        push(&mut run, self.grid.xy(b % hw));
                    }
                }
                let last = *conn.last().unwrap();
                if let Some(e) = point(last % hw as u32) {
                    push(&mut run, e);
                }
                if run.len() > 1 {
                    tracks.push(Track { layer: p.layers[layer].clone(), points: run });
                }
            }
            let length: f64 = tracks
                .iter()
                .map(|t| {
                    t.points
                        .windows(2)
                        .map(|s| ((s[0][0] - s[1][0]).powi(2) + (s[0][1] - s[1][1]).powi(2)).sqrt())
                        .sum::<f64>()
                })
                .sum();
            // lower is better: weighted length, a via as 1 mm, and each mm over the wanted length as 100 mm
            score += net.weight * (length + 100.0 * net.max_mm.map_or(0.0, |m| (length - m).max(0.0)))
                + st.vias.len() as f64;
            // a trunk that has been split has given up its claim: its legs stand for it
            let t = if matches!(part[n], Some((k, None)) if self.legs[k].is_some()) {
                Trouble { track: Vec::new(), via: Vec::new(), own: Vec::new() }
            } else {
                self.trouble(n)
            };
            let st = &self.nets[n];
            let others = |q_list: &dyn Fn(usize) -> bool| -> Vec<String> {
                (0..p.nets.len()).filter(|&m| m != n && q_list(m)).map(|m| p.nets[m].key.clone()).collect()
            };
            let mut seen_with: Vec<Vec<String>> = Vec::new();
            for &s in &t.track {
                let with =
                    others(&|m| self.nets[m].claims_t.get(self.tclass[n]).is_some_and(|c| c.binary_search(&s).is_ok()));
                if !seen_with.contains(&with) {
                    seen_with.push(with.clone());
                    conflicts.push(Conflict {
                        kind: ConflictKind::Track,
                        key: net.key.clone(),
                        with,
                        layer: Some(p.layers[s as usize / hw].clone()),
                        xy: self.grid.xy(s as usize % hw),
                    });
                }
            }
            for &v in &t.via {
                let with =
                    others(&|m| self.nets[m].claims_v.get(self.vclass[n]).is_some_and(|c| c.binary_search(&v).is_ok()));
                conflicts.push(Conflict {
                    kind: ConflictKind::Via,
                    key: net.key.clone(),
                    with,
                    layer: None,
                    xy: self.grid.xy(v as usize),
                });
            }
            for &v in &t.own {
                conflicts.push(Conflict {
                    kind: ConflictKind::OwnVias,
                    key: net.key.clone(),
                    with: Vec::new(),
                    layer: None,
                    xy: self.grid.xy(v as usize),
                });
            }
            let vias: Vec<Pt> = st.vias.iter().map(|&v| self.grid.xy(v as usize)).collect();
            match part[n] {
                None => nets.push(NetResult {
                    key: net.key.clone(),
                    net: net.net.clone(),
                    length_mm: round6(length),
                    tracks,
                    vias,
                }),
                Some((k, Some(leg))) => {
                    pieces[k][leg].0.extend(tracks);
                    pieces[k][leg].1.extend(vias);
                }
                Some((k, None)) if self.legs[k].is_none() => nets.push(NetResult {
                    key: net.key.clone(),
                    net: net.net.clone(),
                    length_mm: round6(length),
                    tracks,
                    vias,
                }),
                Some(_) => {}
            }
        }
        // each net of a pair: its leg and its two stubs
        for (k, pair) in p.pairs.iter().enumerate() {
            if let Some((layer, lines)) = &self.legs[k] {
                for leg in 0..2 {
                    let mut tracks = vec![Track { layer: p.layers[*layer].clone(), points: lines[leg].clone() }];
                    tracks.append(&mut pieces[k][leg].0);
                    let length: f64 = tracks
                        .iter()
                        .map(|t| {
                            t.points
                                .windows(2)
                                .map(|s| ((s[0][0] - s[1][0]).powi(2) + (s[0][1] - s[1][1]).powi(2)).sqrt())
                                .sum::<f64>()
                        })
                        .sum();
                    nets.push(NetResult {
                        key: format!("{}/{}", pair.key, pair.legs[leg]),
                        net: pair.legs[leg].clone(),
                        length_mm: round6(length),
                        tracks,
                        vias: std::mem::take(&mut pieces[k][leg].1),
                    });
                }
            }
        }
        let mut unrouted: Vec<String> = self.nets.iter().flat_map(|n| n.failed.iter().cloned()).collect();
        unrouted.extend(self.unlaid.iter().cloned());
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
