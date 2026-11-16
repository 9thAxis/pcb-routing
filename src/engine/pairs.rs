//! Pairs: a settled trunk becomes two legs, each joined to its pads by stubs.

use super::*;
use crate::geom::{dist, dist_segment, dist_segments, push};
use crate::grid::round6;

impl Engine<'_> {
    /// Split the settled trunk of pair `k` into its two legs. The trunk's claim (one wide track) is given up
    /// and each leg is claimed as the shape it is, held by the stub at its first end; every stub gets the end
    /// of its leg as second terminal. Where the trunk changes layer each leg is drawn aside to a via of its
    /// own, the two side by side across the trunk's way; where it passes a pad of each net, the pad's stub
    /// gets the nearest point of its leg. Returns the stub entries, to be laid next: the four of its ends first.
    pub(super) fn split(&mut self, k: usize) -> Result<Vec<usize>, String> {
        let pair = &self.p.pairs[k];
        let trunk = self.index(&pair.trunk)?;
        let stubs = [
            [self.index(&pair.stubs[0][0])?, self.index(&pair.stubs[0][1])?],
            [self.index(&pair.stubs[1][0])?, self.index(&pair.stubs[1][1])?],
        ];
        // (a trunk that passes pads on its way was laid stretch by stretch, each from where the last ended)
        let stretches = &self.laid.nets[trunk].conns;
        if stretches.len() != pair.passes.len() + 1 {
            let why = if stretches.is_empty() { "trunk not laid" } else { "trunk not laid all the way" };
            return Err(format!("{}: {why}", pair.key));
        }
        let mut conn: Vec<u32> = Vec::new();
        for stretch in stretches {
            let met = usize::from(!conn.is_empty() && conn.last() == stretch.first());
            conn.extend_from_slice(&stretch[met..]);
        }
        // the trunk's line on each layer it runs on, in turn
        let mut centres: Vec<(usize, Vec<Pt>)> = Vec::new();
        for &state in &conn {
            let (layer, cell) = self.plan.space.split(state as usize);
            match centres.last_mut() {
                Some((on, line)) if *on == layer => push(line, self.plan.grid.xy(cell)),
                _ => centres.push((layer, vec![self.plan.grid.xy(cell)])),
            }
        }
        if centres.len() > 1 && pair.via_pitch <= 0.0 {
            return Err(format!("{}: the trunk changes layer", pair.key));
        }
        if centres.iter().any(|(_, line)| line.len() < 2) {
            let why = if centres.len() > 1 { "changes layer where it has no length" } else { "has no length" };
            return Err(format!("{}: the trunk {why}", pair.key));
        }
        // the first net of the pair takes the side on which its pad at the first end lies further out than the
        // second net's pad (both pads can lie on one side of the trunk)
        let start = &centres[0].1;
        let first = unit(start[0], start[1]);
        let pad = |n: usize| self.p.nets[n].terminals[0].xy;
        let side_of = |q: Pt| first[0] * (q[1] - start[0][1]) - first[1] * (q[0] - start[0][0]);
        let left = side_of(pad(stubs[0][0])) > side_of(pad(stubs[0][1]));
        let d = pair.pitch / 2.0;
        // (what the trunk is wider than its two legs side by side, on either side)
        let spare = ((self.p.nets[trunk].width - pair.pitch - pair.leg_width) / 2.0).max(0.0);
        let by = |leg: usize| if (leg == 0) == left { d } else { -d };
        let mut beside_trunk = Legs { runs: Vec::new(), vias: Vec::new() };
        for (layer, centre) in &centres {
            let mut lines = [beside(centre, by(0), spare), beside(centre, by(1), spare)];
            drawn_back(&mut lines, pair.pitch);
            beside_trunk.runs.push((*layer, lines));
        }
        let sizes = Sizes {
            pitch: pair.pitch,
            spare,
            leg: pair.leg_width / 2.0,
            via: self.p.nets[stubs[0][0]].via[0].max(self.p.nets[stubs[0][1]].via[0]) / 2.0,
            wide: self.p.nets[trunk].via[0] / 2.0,
            via_pitch: pair.via_pitch,
            hole: self.p.rules.hole_centre_mm,
        };
        let lines: Vec<&[Pt]> = centres.iter().map(|(_, line)| &line[..]).collect();
        let mut legs = beside_trunk;
        through(&lines, &mut legs, [by(0) > 0.0, by(1) > 0.0], &sizes).map_err(|why| format!("{}: {why}", pair.key))?;
        // Nothing else tests the legs: they are laid only if they are what they are meant to be.
        fit(&lines, &legs, &sizes).map_err(|why| format!("{}: {why}", pair.key))?;
        // Where the pair passes a pad of each net, the pad's stub joins its leg where the leg comes nearest
        // the pad: the leg gets a corner there for the stub to end on.
        let mut passed: Vec<usize> = Vec::new();
        // each stub with its leg, and where on the leg it ends: the run and the point
        let mut joints: Vec<(usize, usize, usize, Pt)> = Vec::new();
        for pass in &pair.passes {
            for (leg, key) in pass.iter().enumerate() {
                let n = self.index(key)?;
                let (run, at) = branch_at(&mut legs, leg, pad(n))
                    .ok_or_else(|| format!("{}: no place on its leg for the stub {key}", pair.key))?;
                joints.push((n, leg, run, at));
                passed.push(n);
            }
        }
        for leg in 0..2 {
            for end in 0..2 {
                let run = if end == 0 { 0 } else { legs.runs.len() - 1 };
                let line = &legs.runs[run].1[leg];
                joints.push((stubs[end][leg], leg, run, if end == 0 { line[0] } else { *line.last().unwrap() }));
            }
        }
        let mut footholds: Vec<(usize, usize, u32, Pt)> = Vec::new();
        for &(n, leg, run, at) in &joints {
            let (layer, lines) = &legs.runs[run];
            let cell =
                self.foothold(n, stubs[0][1 - leg], at, *layer, &lines[1 - leg], pair.leg_width).ok_or_else(|| {
                    format!(
                        "{}: no free cell on a leg at {:?} for its stub {} (the legs are closer than the rules let \
                     a stub come, or fixed copper is in the way)",
                        pair.key, at, self.p.nets[n].key
                    )
                })?;
            footholds.push((n, *layer, cell, at));
        }
        self.unclaim(trunk);
        for leg in 0..2 {
            let holder = stubs[0][leg];
            self.unclaim(holder);
            let held = &mut self.laid.nets[holder];
            held.shapes = (legs.runs.iter())
                .flat_map(|(layer, lines)| lines[leg].windows(2).map(|w| (*layer, w[0], w[1], pair.leg_width / 2.0)))
                .collect();
            held.posts = legs.vias.iter().map(|vias| vias[leg]).collect();
            self.claim(holder);
        }
        for (n, layer, cell, at) in footholds {
            self.pairs.extra_terms[n] = vec![Term { cell, layers: 1 << layer }];
            self.exact[n].insert(cell, at);
        }
        self.pairs.legs[k] = Some(legs);
        Ok([stubs[0][0], stubs[0][1], stubs[1][0], stubs[1][1]].into_iter().chain(passed).collect())
    }

    /// Where the stub `n` comes to its leg at the point `at` of it on `layer`: the cell nearest that point
    /// from which a track of the stub still keeps its distance from the other leg (`other`, its line there; a
    /// stub of it is `of_other`). The stub joins the point itself from there. None if there is no such cell.
    fn foothold(&self, n: usize, of_other: usize, at: Pt, layer: usize, other: &[Pt], leg_width: f64) -> Option<u32> {
        let (me, them) = (&self.p.nets[n], &self.p.nets[of_other]);
        let rules = &self.p.rules;
        let apart = if me.quiet != them.quiet && rules.quiet_clearance_mm > 0.0 {
            rules.quiet_clearance_mm
        } else {
            rules.new_clearance_mm
        };
        let need = me.width / 2.0 + apart + leg_width / 2.0;
        let step = self.p.step;
        let mut best: Option<(f64, u32)> = None;
        for dj in -3i32..=3 {
            for di in -3i32..=3 {
                let q = [at[0] + di as f64 * step, at[1] + dj as f64 * step];
                let Some(cell) = self.plan.grid.cell(q) else { continue };
                if self.shared.fixed.mask(n, cell) & (1 << layer) != 0 {
                    continue;
                }
                let c = self.plan.grid.xy(cell);
                let clear = other.windows(2).map(|w| dist_segment(c, w[0], w[1])).fold(f64::INFINITY, f64::min);
                if clear < need + 1e-6 {
                    continue;
                }
                let far = dist(c, at);
                if best.map_or(true, |b| far < b.0 - 1e-9) {
                    best = Some((far, cell as u32));
                }
            }
        }
        best.map(|found| found.1)
    }

    /// The stub entries of pair `k`: those of the first end, those of the last, then those of the pads it passes.
    pub(super) fn stubs_of(&self, k: usize) -> Vec<usize> {
        self.p.pairs[k].stub_keys().filter_map(|(_, key)| self.index(key).ok()).collect()
    }

    /// Take pair `k` out after its trunk could not be split: the trunk and whatever the split had begun give
    /// up their ground, so that the nets laid after stand as if the pair were not there.
    pub(super) fn take_out(&mut self, k: usize) {
        if let Ok(trunk) = self.index(&self.p.pairs[k].trunk) {
            self.place(trunk, NetState::default());
        }
        for n in self.stubs_of(k) {
            self.unclaim(n);
            self.laid.nets[n] = NetState::default();
            self.pairs.extra_terms[n].clear();
        }
        self.pairs.legs[k] = None;
    }

    /// Lay the stubs of a pair (the four of its ends, then those of the pads it passes), one after the other,
    /// each fixed for the next. Those of the pads it passes go first: they are a step or two long and have
    /// one way to go. Which of the four goes first can decide whether all fit, so the orders are tried in turn
    /// until one does. If none does, they are laid in the order given as far as each goes, and what is missing
    /// is listed with each.
    pub(super) fn lay_stubs(&mut self, stubs: &[usize]) {
        let (four, passed) = stubs.split_at(4);
        for &n in passed {
            let state = self.lay_fixed(n);
            self.place(n, state);
        }
        for code in 0..24 {
            let mut ok = true;
            for n in nth_order(four, code) {
                match self.lay_whole(n) {
                    Ok(state) => self.place(n, state),
                    Err(_) => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                return;
            }
            for &n in four {
                self.place(n, NetState::default()); // the leg an entry holds stays with it
            }
        }
        for &n in four {
            let state = self.lay_fixed(n);
            self.place(n, state);
        }
    }
}

/// The two legs of a split pair.
#[derive(Clone)]
pub(super) struct Legs {
    /// For each layer the trunk ran on, in its order: the layer and the line of either leg there.
    pub(super) runs: Vec<(usize, [Vec<Pt>; 2])>,
    /// Between each of those and the next: the via of either leg.
    pub(super) vias: Vec<[Pt; 2]>,
}

/// What the legs of a pair are measured by: half widths and distances between centres.
struct Sizes {
    /// From leg to leg.
    pitch: f64,
    /// What the trunk is wider than its two legs side by side, on either side.
    spare: f64,
    /// Half the width of a leg.
    leg: f64,
    /// The radius of a leg's via.
    via: f64,
    /// The radius of the trunk's via.
    wide: f64,
    /// From via to via where the pair changes layer.
    via_pitch: f64,
    /// The least distance between the centres of two holes.
    hole: f64,
}

/// What two places worked out to a millionth of a millimetre each may be nearer than they are meant to be.
const ROUNDED: f64 = 3e-6;

/// The direction from `a` to `b`, as a vector of length 1.
fn unit(a: Pt, b: Pt) -> Pt {
    let (x, y) = (b[0] - a[0], b[1] - a[1]);
    let l = (x * x + y * y).sqrt();
    [x / l, y / l]
}

/// A line without the last `by` of its length at one end (`at_end`: its last point, else its first). None
/// if it is no longer than that.
fn shortened(line: &[Pt], at_end: bool, mut by: f64) -> Option<Vec<Pt>> {
    let mut points: Vec<Pt> = line.to_vec();
    if !at_end {
        points.reverse();
    }
    while points.len() >= 2 {
        let (p, q) = (points[points.len() - 1], points[points.len() - 2]);
        let length = dist(p, q);
        if length > by + 1e-9 {
            let t = by / length;
            let k = points.len() - 1;
            points[k] = [round6(p[0] + t * (q[0] - p[0])), round6(p[1] + t * (q[1] - p[1]))];
            if !at_end {
                points.reverse();
            }
            return Some(points);
        }
        by -= length;
        points.pop();
    }
    None
}

/// Take the legs through the trunk's layer changes: where its line on one layer (`centres`, in turn) ends and
/// the next begins, each leg gets a via of its own, the two `via_pitch` apart across the trunk's way there
/// and the leg on the left (`on_left`, per leg) to the left; and each leg leaves its line a little before
/// to come to its via at 45 degrees, and joins its line again as far behind. (The trunk runs straight
/// that far and more on either side of a layer change: see `Plan::runs`.)
fn through(centres: &[&[Pt]], legs: &mut Legs, on_left: [bool; 2], sizes: &Sizes) -> Result<(), String> {
    let aside = sizes.via_pitch / 2.0;
    let before = (aside - sizes.pitch / 2.0).max(0.0);
    for k in 1..centres.len() {
        let (came, goes) = (centres[k - 1], centres[k]);
        let at = goes[0];
        let (u, v) = (unit(came[came.len() - 2], came[came.len() - 1]), unit(goes[0], goes[1]));
        let way = [u[0] + v[0], u[1] + v[1]];
        let length = (way[0] * way[0] + way[1] * way[1]).sqrt();
        if length < 1.0 {
            return Err("the trunk turns too sharply where it changes layer".into());
        }
        let way = [way[0] / length, way[1] / length];
        let mut vias = [[0.0; 2]; 2];
        for leg in 0..2 {
            let by = if on_left[leg] { aside } else { -aside };
            let via = [round6(at[0] - way[1] * by), round6(at[1] + way[0] * by)];
            let short = || "the trunk has too little length beside a layer change for a leg to come to its via";
            let mut comes = shortened(&legs.runs[k - 1].1[leg], true, before).ok_or_else(short)?;
            let mut leaves = shortened(&legs.runs[k].1[leg], false, before).ok_or_else(short)?;
            push(&mut comes, via);
            if dist(leaves[0], via) > 1e-9 {
                leaves.insert(0, via);
            }
            (legs.runs[k - 1].1[leg], legs.runs[k].1[leg]) = (comes, leaves);
            vias[leg] = via;
        }
        legs.vias.push(vias);
    }
    Ok(())
}

/// Where a branch from `pad` comes to leg `leg`: the point of the leg nearest the pad, as (the run it is on,
/// the point). The leg gets a corner there, if it has none. None if the leg has no length.
fn branch_at(legs: &mut Legs, leg: usize, pad: Pt) -> Option<(usize, Pt)> {
    let mut best: Option<(f64, usize, usize, Pt)> = None;
    for (run, (_, lines)) in legs.runs.iter().enumerate() {
        for (k, w) in lines[leg].windows(2).enumerate() {
            let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
            let length = dx * dx + dy * dy;
            if length < 1e-18 {
                continue;
            }
            let t = (((pad[0] - w[0][0]) * dx + (pad[1] - w[0][1]) * dy) / length).clamp(0.0, 1.0);
            let q = [round6(w[0][0] + t * dx), round6(w[0][1] + t * dy)];
            if best.map_or(true, |b| dist(q, pad) < b.0 - 1e-9) {
                best = Some((dist(q, pad), run, k, q));
            }
        }
    }
    let (_, run, k, q) = best?;
    let line = &mut legs.runs[run].1[leg];
    if dist(line[k], q) > 1e-6 && dist(line[k + 1], q) > 1e-6 {
        line.insert(k + 1, q);
    }
    let q = if dist(line[k], q) <= 1e-6 {
        line[k]
    } else if dist(line[k + 1], q) <= 1e-6 {
        line[k + 1]
    } else {
        q
    };
    Some((run, q))
}

/// The line that runs `by` to the left of a line (to its right if negative), its corners where the stretches
/// beside two stretches meet. On the outside of a corner sharp enough for that meeting point to lie more
/// than `spare` further from the line than `by`, the corner is cut straight across instead: the leg stays
/// within the trunk it was split from. A stretch too short to have room on the inside of its corners is
/// left out: beside it the line would run backwards, towards the other leg of the pair.
fn beside(centre: &[Pt], by: f64, spare: f64) -> Vec<Pt> {
    let along = |a: Pt, b: Pt| {
        let (x, y) = (b[0] - a[0], b[1] - a[1]);
        let l = (x * x + y * y).sqrt();
        [x / l, y / l]
    };
    // each stretch moved aside: (where it begins, where it ends, its direction)
    let mut stretches: Vec<(Pt, Pt, Pt)> = centre
        .windows(2)
        .map(|w| {
            let u = along(w[0], w[1]);
            let aside = [-u[1] * by, u[0] * by];
            ([w[0][0] + aside[0], w[0][1] + aside[1]], [w[1][0] + aside[0], w[1][1] + aside[1]], u)
        })
        .collect();
    loop {
        // the stretches as they meet: each from its corner with the one before to its corner with the next
        let mut met: Vec<(Pt, Pt)> = stretches.iter().map(|&(a, b, _)| (a, b)).collect();
        for k in 1..stretches.len() {
            let ((a, _, u), (c, _, v)) = (stretches[k - 1], stretches[k]);
            let (cross, dot) = (u[0] * v[1] - u[1] * v[0], u[0] * v[0] + u[1] * v[1]);
            // (how much further from the line than `by` the two meet: by / cos of half the turn)
            let out = by.abs() * ((2.0 / (1.0 + dot)).sqrt() - 1.0);
            let outside = cross * by < 0.0;
            if cross.abs() < 1e-9 || (outside && out > spare) {
                continue; // in line, or cut across: each keeps its own end
            }
            let t = ((c[0] - a[0]) * v[1] - (c[1] - a[1]) * v[0]) / cross;
            let corner = [a[0] + t * u[0], a[1] + t * u[1]];
            (met[k - 1].1, met[k].0) = (corner, corner);
        }
        let backwards = |k: &usize| {
            let ((p, q), u) = (met[*k], stretches[*k].2);
            (q[0] - p[0]) * u[0] + (q[1] - p[1]) * u[1] <= 1e-9
        };
        match (0..stretches.len()).find(backwards).filter(|_| stretches.len() > 1) {
            Some(k) => {
                // (at an end, the line ends where its neighbour met it)
                if k == 0 {
                    stretches[1].0 = met[1].0;
                } else if k == stretches.len() - 1 {
                    stretches[k - 1].1 = met[k - 1].1;
                }
                stretches.remove(k);
            }
            None => {
                let mut line = Vec::new();
                for (from, to) in met {
                    push(&mut line, [round6(from[0]), round6(from[1])]);
                    push(&mut line, [round6(to[0]), round6(to[1])]);
                }
                return line;
            }
        }
    }
}

/// Draw the ends of the legs back where they lie nearer the other leg than the pitch. Where the trunk's line
/// turns within a step or two of its end (it often does, finding its way off the place it began at), the
/// leg on the inside of the turn ends under the other's way round it; a leg may end short of the trunk's
/// end, its stub joins it wherever it ends. What is too near elsewhere than at an end is left as it is,
/// for [`fit`] to refuse.
fn drawn_back(legs: &mut [Vec<Pt>; 2], pitch: f64) {
    let near = |a: Pt, b: Pt, other: &[Pt]| {
        other.windows(2).map(|w| dist_segments(a, b, w[0], w[1])).fold(f64::INFINITY, f64::min)
    };
    for _ in 0..64 {
        let mut drawn = false;
        for leg in 0..2 {
            for at_end in [false, true] {
                let (mine, other) = (&legs[leg], &legs[1 - leg]);
                if mine.len() < 2 || other.len() < 2 {
                    return;
                }
                // the last stretch at this end: from its end `p` back to `q`
                let k = if at_end { mine.len() - 1 } else { 0 };
                let (p, q) = (mine[k], mine[if at_end { k - 1 } else { 1 }]);
                if near(p, q, other) >= pitch - 1e-6 {
                    continue;
                }
                // (the other leg's last stretch may be the one to give way: the shorter of the two does)
                let theirs = |far: bool| {
                    let j = if far { other.len() - 1 } else { 0 };
                    (other[j], other[if far { j - 1 } else { 1 }])
                };
                let shorter = |far: bool| {
                    let (a, b) = theirs(far);
                    dist_segments(p, q, a, b) < pitch - 1e-6 && dist(a, b) < dist(p, q)
                };
                if shorter(false) || shorter(true) {
                    continue;
                }
                // back along the stretch to where it is clear of the other leg, or the whole stretch
                let length = dist(p, q);
                let pieces = (length / (pitch / 20.0)).ceil().max(1.0) as usize;
                let from = |i: usize| {
                    let t = i as f64 / pieces as f64;
                    [round6(p[0] + t * (q[0] - p[0])), round6(p[1] + t * (q[1] - p[1]))]
                };
                // (by no more than two pitches at a time: what is too near further in is not an end's doing)
                let clear = (1..pieces).find(|&i| near(from(i), q, other) >= pitch - 1e-6);
                match clear {
                    Some(i) if dist(p, from(i)) <= 2.0 * pitch => legs[leg][k] = from(i),
                    None if length <= 2.0 * pitch => {
                        legs[leg].remove(k);
                    }
                    _ => continue,
                }
                drawn = true;
            }
        }
        if !drawn {
            return;
        }
    }
}

/// Whether the two legs beside a trunk's lines (`centres`: on each layer in turn) are fit to be laid: each of
/// two points at least on every layer, no nearer each other than the pitch, no via of one nearer the other
/// (its track or its vias) than the legs are to each other, and nowhere further from the trunk's line than
/// half the pitch and what the trunk has to spare, but within the trunk's via where it changes layer (so that
/// what the trunk kept its distance from, the legs do too). Where the line turns twice in a row within less
/// room than a leg needs on the inside of a corner, the legs beside it do not come out so.
fn fit(centres: &[&[Pt]], legs: &Legs, sizes: &Sizes) -> Result<(), String> {
    let pitch = sizes.pitch;
    if legs.runs.iter().any(|(_, lines)| lines.iter().any(|line| line.len() < 2)) {
        return Err("the trunk is too short to be split into legs".into());
    }
    let between = |a: &[Pt], b: &[Pt]| {
        let pairs = a.windows(2).flat_map(|s| b.windows(2).map(move |t| dist_segments(s[0], s[1], t[0], t[1])));
        pairs.fold(f64::INFINITY, f64::min)
    };
    // (a trunk that comes back to a layer may pass itself there)
    let mut apart = f64::INFINITY;
    for (layer, first) in &legs.runs {
        for (_, second) in legs.runs.iter().filter(|(other, _)| other == layer) {
            apart = apart.min(between(&first[0], &second[1]));
        }
    }
    if apart < pitch - ROUNDED {
        return Err(format!("its legs would be {apart:.4} mm apart at a corner of the trunk, for a pitch of {pitch}"));
    }
    let gap = pitch - 2.0 * sizes.leg;
    for (k, vias) in legs.vias.iter().enumerate() {
        for leg in 0..2 {
            let via = vias[leg];
            let tracks = legs.runs.iter().flat_map(|(_, lines)| lines[1 - leg].windows(2));
            let track = tracks.map(|w| dist_segment(via, w[0], w[1])).fold(f64::INFINITY, f64::min);
            let other = legs.vias.iter().map(|vias| dist(via, vias[1 - leg])).fold(f64::INFINITY, f64::min);
            if track < sizes.via + gap + sizes.leg - ROUNDED
                || other < (2.0 * sizes.via + gap).max(sizes.hole) - ROUNDED
            {
                return Err("a via of one leg would be too near the other leg where the trunk changes layer".into());
            }
            // (where the trunk changes layer twice with a turn between, the two vias of the leg on the inside)
            let own = legs.vias.iter().enumerate().filter(|&(j, _)| j != k).map(|(_, vias)| dist(via, vias[leg]));
            if own.fold(f64::INFINITY, f64::min) < sizes.hole - ROUNDED {
                return Err("two vias of a leg would be nearer each other than two holes may be".into());
            }
            if dist(via, centres[k + 1][0]) > sizes.wide - sizes.via + ROUNDED {
                return Err("a leg's via would lie beyond the trunk's".into());
            }
        }
    }
    // (a leg as points along it, a tenth of the pitch apart at most)
    let along = |leg: &[Pt]| -> Vec<Pt> {
        let stretch = |w: &[Pt]| {
            let (a, b) = (w[0], w[1]);
            let pieces = (dist(a, b) / (pitch / 10.0)).ceil().max(1.0) as usize;
            (0..=pieces).map(move |k| {
                let t = k as f64 / pieces as f64;
                [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]
            })
        };
        leg.windows(2).flat_map(stretch).collect()
    };
    for (k, (_, lines)) in legs.runs.iter().enumerate() {
        let centre = centres[k];
        // (the trunk's vias at either end of this layer's line)
        let changes = [(k > 0).then(|| centre[0]), (k + 1 < centres.len()).then(|| centre[centre.len() - 1])];
        let off = |p: Pt| centre.windows(2).map(|w| dist_segment(p, w[0], w[1])).fold(f64::INFINITY, f64::min);
        let within = |p: Pt| changes.iter().flatten().any(|&at| dist(p, at) <= sizes.wide - sizes.leg + 1e-6);
        let beyond = |p: &Pt| !within(*p);
        let furthest = lines.iter().flat_map(|leg| along(leg)).filter(beyond).map(off).fold(0.0, f64::max);
        if furthest > pitch / 2.0 + sizes.spare + 1e-6 {
            return Err(format!("a leg would lie {furthest:.4} mm from the middle of the trunk, beyond its width"));
        }
    }
    Ok(())
}

/// The `code`-th order of four things (0 to 23); the 0th is the order given.
fn nth_order(four: &[usize], mut code: usize) -> Vec<usize> {
    let mut left: Vec<usize> = four.to_vec();
    let mut order = Vec::new();
    for size in (1..=4).rev() {
        order.push(left.remove(code % size));
        code /= size;
    }
    order
}

#[cfg(test)]
mod tests {
    use super::{beside, drawn_back, nth_order, through, Legs, Sizes};
    use crate::geom::{dist, dist_segments};

    /// The legs beside a line at a pitch, as `split` makes them.
    fn legs(centre: &[[f64; 2]], pitch: f64, spare: f64) -> [Vec<[f64; 2]>; 2] {
        let mut both = [beside(centre, pitch / 2.0, spare), beside(centre, -pitch / 2.0, spare)];
        drawn_back(&mut both, pitch);
        both
    }

    /// Whether two legs beside a line on one layer are fit to be laid (legs 0.2 wide at the least pitch).
    fn fit(centre: &[[f64; 2]], both: &[Vec<[f64; 2]>; 2], pitch: f64, spare: f64) -> Result<(), String> {
        let sizes = Sizes { pitch, spare, leg: 0.1, via: 0.3, wide: 0.7, via_pitch: 0.8, hole: 0.56 };
        super::fit(&[centre], &Legs { runs: vec![(0, both.clone())], vias: Vec::new() }, &sizes)
    }

    #[test]
    fn a_leg_that_ends_under_the_others_way_round_a_turn_is_drawn_back() {
        // two short stretches that turn the same way where the trunk begins
        let hook = [[0.0, 0.0], [0.05, 0.0], [0.1, -0.05], [0.1, -2.15]];
        let both = legs(&hook, 0.4, 0.02);
        assert_eq!(fit(&hook, &both, 0.4, 0.02), Ok(()));
        // one of them begins later than it did, and both still run to the trunk's end
        let before = [beside(&hook, 0.2, 0.02), beside(&hook, -0.2, 0.02)];
        assert!(both[0][0] != before[0][0] || both[1][0] != before[1][0]);
        assert!(both.iter().all(|leg| leg[leg.len() - 1][1] == -2.15));
        // a trunk of one step with a corner has nothing left to lay
        let stump = [[5.0, 3.0], [5.05, 3.0], [5.1, 3.05]];
        assert!(fit(&stump, &legs(&stump, 0.4, 0.02), 0.4, 0.02).is_err());
    }

    #[test]
    fn legs_that_are_not_what_they_are_meant_to_be_are_not_laid() {
        // (lines an independent review found: each gave legs too near each other, or beyond the trunk)
        // a jog shorter than a leg has room for, between two stretches side by side, at a pitch too wide to mitre
        let jog = [[0.0, 0.0], [2.0, 0.0], [2.05, 0.05], [4.0, 0.05]];
        assert!(fit(&jog, &legs(&jog, 0.5, 0.02), 0.5, 0.02).unwrap_err().contains("apart"));
        // two short stretches that turn the same way at an end, as `beside` leaves them
        let hook = [[0.0, 0.0], [0.05, 0.0], [0.1, -0.05], [0.1, -2.15]];
        assert!(fit(&hook, &[beside(&hook, 0.2, 0.02), beside(&hook, -0.2, 0.02)], 0.4, 0.02).is_err());
        // and what is as it should be passes: straight, one corner, a corner cut across
        for line in [
            vec![[0.0, 0.0], [3.0, 0.0]],
            vec![[0.0, 0.0], [2.0, 0.0], [3.0, 1.0], [3.0, 3.0]],
            vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]],
        ] {
            assert_eq!(fit(&line, &legs(&line, 0.4, 0.02), 0.4, 0.02), Ok(()), "{line:?}");
        }
    }

    #[test]
    fn whatever_the_line_legs_that_pass_keep_the_pitch_and_stay_within_the_trunk() {
        // lines as a search on the grid lays them: steps in the eight directions, many of them short
        let mut seed = 0x2545F4914F6CDD1Du64;
        let mut next = |most: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % most
        };
        let (mut passed, mut refused) = (0, 0);
        for _ in 0..4000 {
            let pitch = [0.3, 0.4, 0.5, 0.7][next(4) as usize];
            let spare = [0.02, 0.0412 * pitch][next(2) as usize];
            let mut line = vec![[0.0, 0.0]];
            let mut heading = next(8) as i64;
            for _ in 0..(2 + next(6)) {
                heading = (heading + [0, 1, 7, 1, 7, 2][next(6) as usize]) % 8;
                let length = [0.05, 0.05, 0.1, 0.3, 1.0, 2.5][next(6) as usize];
                let (dx, dy) = [(1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1)][heading as usize];
                let last = line[line.len() - 1];
                line.push([last[0] + length * dx as f64, last[1] + length * dy as f64]);
            }
            let mut tidy = Vec::new();
            line.iter().for_each(|&p| crate::geom::push(&mut tidy, p));
            if tidy.len() < 2 {
                continue;
            }
            let both = legs(&tidy, pitch, spare);
            match fit(&tidy, &both, pitch, spare) {
                Ok(()) => {
                    passed += 1;
                    let least = both[0]
                        .windows(2)
                        .flat_map(|s| both[1].windows(2).map(move |t| dist_segments(s[0], s[1], t[0], t[1])));
                    assert!(least.fold(f64::INFINITY, f64::min) >= pitch - super::ROUNDED, "{tidy:?} at {pitch}");
                }
                Err(_) => refused += 1,
            }
        }
        // (these lines are far more crooked than a trunk, which pays for its corners: most of them split all the
        // same; a pair that does not is done as two nets by whoever asked for it)
        assert!(passed > 3000 && refused > 100, "{passed} split, {refused} refused");
    }

    #[test]
    fn the_legs_beside_a_line_keep_their_distance_at_its_corners() {
        let centre = [[0.0, 0.0], [2.0, 0.0], [3.0, 1.0], [3.0, 3.0]];
        let (left, right) = (beside(&centre, 0.2, 0.02), beside(&centre, -0.2, 0.02));
        assert_eq!(left[0], [0.0, 0.2]);
        assert_eq!(right[3], [3.2, 3.0]);
        assert_eq!((left.len(), right.len()), (4, 4));
        let least = |a: &[[f64; 2]], b: &[[f64; 2]]| {
            let pairs = a.windows(2).flat_map(|s| b.windows(2).map(move |t| dist_segments(s[0], s[1], t[0], t[1])));
            pairs.fold(f64::INFINITY, f64::min)
        };
        assert!((least(&left, &right) - 0.4).abs() < 1e-6);
    }

    #[test]
    fn the_outside_of_a_sharp_corner_is_cut_across_so_that_the_leg_stays_within_the_trunk() {
        // a turn of 90 degrees: the legs would meet 0.083 mm further out than they run; the trunk has 0.02 to spare
        let centre = [[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]];
        let (inside, outside) = (beside(&centre, 0.2, 0.02), beside(&centre, -0.2, 0.02));
        assert_eq!(inside, [[0.0, 0.2], [1.8, 0.2], [1.8, 2.0]]);
        assert_eq!(outside, [[0.0, -0.2], [2.0, -0.2], [2.2, 0.0], [2.2, 2.0]]);
        // every point of the outer leg is within its distance of the line, and the two keep theirs from each other
        let from_line =
            |p: [f64; 2]| dist_segments(p, p, centre[0], centre[1]).min(dist_segments(p, p, centre[1], centre[2]));
        assert!(outside.iter().all(|&p| from_line(p) < 0.2 + 1e-9));
        let least =
            inside.windows(2).flat_map(|s| outside.windows(2).map(move |t| dist_segments(s[0], s[1], t[0], t[1])));
        assert!(least.fold(f64::INFINITY, f64::min) > 0.4 - 1e-9);
        // with room to spare the corner is a corner
        assert_eq!(beside(&centre, -0.2, 0.1), [[0.0, -0.2], [2.2, -0.2], [2.2, 2.0]]);
    }

    #[test]
    fn a_last_stretch_too_short_for_the_inside_of_its_corner_is_left_out() {
        // a line that ends with one slanted step of the grid: beside it on the inside there is no room for it
        let centre = [[0.0, 0.0], [2.0, 0.0], [2.05, 0.05]];
        let (inside, outside) = (beside(&centre, 0.2, 0.02), beside(&centre, -0.2, 0.02));
        assert_eq!(inside, [[0.0, 0.2], [1.917157, 0.2]]); // it ends where the slanted stretch would have met it
        assert_eq!(outside.len(), 3);
        let least = dist_segments(inside[0], inside[1], outside[1], outside[2])
            .min(dist_segments(inside[0], inside[1], outside[0], outside[1]));
        assert!(least > 0.4 - 1e-6, "{least}");
        // the same at the beginning, and for a short stretch between two corners
        let begun = beside(&[[2.05, 0.05], [2.0, 0.0], [0.0, 0.0]], -0.2, 0.02);
        assert_eq!(begun, [[1.917157, 0.2], [0.0, 0.2]]);
        let jog = beside(&[[0.0, 0.0], [2.0, 0.0], [2.05, 0.05], [2.05, 2.0]], 0.2, 0.02);
        assert_eq!(jog, [[0.0, 0.2], [1.85, 0.2], [1.85, 2.0]]);
    }

    #[test]
    fn where_the_trunk_changes_layer_each_leg_comes_aside_to_its_own_via() {
        let sizes = Sizes { pitch: 0.4, spare: 0.02, leg: 0.1, via: 0.3, wide: 0.7, via_pitch: 0.8, hole: 0.56 };
        let split = |lines: [&[[f64; 2]]; 2]| {
            let runs = lines.iter().enumerate().map(|(layer, line)| (layer, legs(line, 0.4, 0.02))).collect();
            let mut both = Legs { runs, vias: Vec::new() };
            through(&lines, &mut both, [true, false], &sizes)
                .and_then(|()| super::fit(&lines, &both, &sizes))
                .map(|()| both)
        };
        // straight through: the vias across the way, the left leg's to the left (y grows downwards on a board)
        let both = split([&[[0.0, 0.0], [2.0, 0.0]], &[[2.0, 0.0], [4.0, 0.0]]]).unwrap();
        assert_eq!(both.vias, [[[2.0, 0.4], [2.0, -0.4]]]);
        // each leg leaves its line 0.2 before, at 45 degrees, and is back on it 0.2 after
        assert_eq!(both.runs[0].1[0], [[0.0, 0.2], [1.8, 0.2], [2.0, 0.4]]);
        assert_eq!(both.runs[1].1[1], [[2.0, -0.4], [2.2, -0.2], [4.0, -0.2]]);
        assert!((dist(both.vias[0][0], both.vias[0][1]) - 0.8).abs() < 1e-9);
        // a trunk that turned at the layer change would bring a leg too near the other's via: refused
        assert!(split([&[[0.0, 0.0], [2.0, 0.0]], &[[2.0, 0.0], [3.0, 1.0]]]).is_err());
        // and so is one with no room to come aside
        assert!(split([&[[1.9, 0.0], [2.0, 0.0]], &[[2.0, 0.0], [4.0, 0.0]]]).is_err());
    }

    #[test]
    fn the_orders_of_four_are_all_there_once_and_begin_with_the_one_given() {
        let four = [7, 3, 9, 5];
        assert_eq!(nth_order(&four, 0), four);
        assert_eq!(nth_order(&four, 1), [3, 7, 9, 5]);
        let mut all: Vec<Vec<usize>> = (0..24).map(|code| nth_order(&four, code)).collect();
        for order in &all {
            let mut sorted = order.clone();
            sorted.sort();
            assert_eq!(sorted, [3, 5, 7, 9]);
        }
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 24);
    }
}
