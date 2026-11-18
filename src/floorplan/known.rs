//! What is known of a board before anything is laid out, and the small things every stage asks of it.

use super::constants::{BESIDE_PIN, LARGE, SWITCHED};
use super::ground::{taken_by, Work};
use super::power::{powered, Powered};
use super::settle::{area, overlap, quartered};
use super::types::{Board, Net, Part, Pin, Way};
use crate::error::{invalid, Error};
use crate::model::Pt;
use std::collections::{BTreeMap, BTreeSet};

/// The parts that are the board's own, and the board with their bodies shut to every other part, where
/// there are any.
///
/// A part on the board that is joined to no other part (no pad on a net, or only on nets of its own: a
/// mounting hole, a mark for the camera) is the board's own: it stays where it stands, and no other part
/// may stand on its body.
pub(super) fn standing(board: &Board) -> (Vec<bool>, Option<Board>) {
    let mut joins: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); board.nets.len()];
    for (p, part) in board.parts.iter().enumerate() {
        for &net in part.pads.iter().flatten().filter(|&&net| net < board.nets.len()) {
            joins[net].insert(p);
        }
    }
    let stays = |part: &Part| {
        part.placed && part.pads.iter().flatten().all(|&net| joins.get(net).map_or(true, |on| on.len() < 2))
    };
    let stays: Vec<bool> = board.parts.iter().map(stays).collect();
    // (and a part that is told to stay where it stands keeps what it is joined to, and its body is shut as theirs)
    let held = |part: &Part| part.fixed && part.placed;
    if !stays.iter().any(|&stays| stays) && !board.parts.iter().any(held) {
        return (stays, None);
    }
    let mut with = board.clone();
    // (but what has no pad at all is drawn on the board, not mounted on it: a mark, a logo. It is in no part's way)
    let shuts = |(part, &stays): &(&Part, &bool)| (stays || held(part)) && !part.pads.is_empty();
    for (part, _) in board.parts.iter().zip(&stays).filter(shuts) {
        let side = part.area.sqrt();
        let [w, h] = part.size.unwrap_or([side, side]);
        with.keepouts.push([part.at[0] - w / 2.0, part.at[1] - h / 2.0, part.at[0] + w / 2.0, part.at[1] + h / 2.0]);
    }
    (stays, Some(with))
}

/// What every stage of the floorplan reads: the board, its power, its signal nets, and what of its parts
/// disturbs or is disturbed. All by index into the parts and the nets of the board.
pub(super) struct Known<'a> {
    pub(super) board: &'a Board,
    pub(super) parts: &'a [Part],
    pub(super) all_nets: &'a [Net],
    /// The parts that are the board's own: they stay where they stand.
    pub(super) stays: Vec<bool>,
    /// The part each part mates with, if any.
    pub(super) mate_of: Vec<Option<usize>>,
    /// The power analysis.
    pub(super) found: Powered,
    /// The parts each net reaches.
    pub(super) reach: Vec<BTreeSet<usize>>,
    /// The signal nets (what is no ground and no supply) that reach more than one part; and each as its parts.
    pub(super) signal: Vec<usize>,
    pub(super) nets: Vec<Vec<usize>>,
    /// The domains, the one with the most parts first: each its ground and its parts.
    pub(super) domains: Vec<(usize, Vec<usize>)>,
    /// The parts on no ground.
    pub(super) adrift: Vec<usize>,
    /// The domain of each ground.
    pub(super) domain_at: BTreeMap<usize, usize>,
    /// The plane layers that supplies are poured on: every plane after the first, the ground's.
    pub(super) power_layers: Vec<String>,
    /// What disturbs, and what is disturbed.
    pub(super) aggressor: Vec<bool>,
    pub(super) victim: Vec<bool>,
}

impl<'a> Known<'a> {
    /// What is known of a board (`stays`: see [`standing`]). An error if a pad names a net that is not there,
    /// or the board has no size.
    pub(super) fn of(board: &'a Board, stays: Vec<bool>) -> Result<Known<'a>, Error> {
        let (parts, all_nets) = (&board.parts, &board.nets);
        let [x0, y0, x1, y1] = board.outline;
        if !(x1 > x0 && y1 > y0) {
            return Err(invalid("box", "has no size"));
        }
        for (p, part) in parts.iter().enumerate() {
            if part.pads.iter().flatten().any(|&net| net >= all_nets.len()) {
                return Err(invalid(format!("parts[{p}].pads"), format!("names a net of {}", all_nets.len())));
            }
            for (what, count) in [("offsets", part.offsets.len()), ("pins", part.pins.len())] {
                if count != 0 && count != part.pads.len() {
                    return Err(invalid(format!("parts[{p}].{what}"), format!("{count} for {} pads", part.pads.len())));
                }
            }
        }
        let mut mate_of: Vec<Option<usize>> = vec![None; parts.len()];
        for (p, part) in parts.iter().enumerate() {
            if let Some(mate) = &part.mate {
                let Some(other) = parts.iter().position(|other| &other.name == mate).filter(|&other| other != p) else {
                    return Err(invalid(format!("parts[{p}].mate"), format!("names no other part: {mate}")));
                };
                (mate_of[p], mate_of[other]) = (Some(other), Some(p));
            }
        }
        let found = powered(board);
        let is_chip = |q: usize| !parts[q].capacitor && !parts[q].edge && parts[q].pads.len() > 2;
        let net_name = |net: usize| all_nets[net].name.clone();

        // The signal nets: what is no ground and no supply, each as the parts it reaches.
        let mut reach: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); all_nets.len()];
        for (p, part) in parts.iter().enumerate() {
            for &net in part.pads.iter().flatten() {
                reach[net].insert(p);
            }
        }
        let is_signal = |net: usize| !found.grounds.contains(&net) && !found.rails.contains_key(&net);
        let signal: Vec<usize> = (0..all_nets.len()).filter(|&net| is_signal(net) && reach[net].len() > 1).collect();
        let nets: Vec<Vec<usize>> = signal.iter().map(|&net| reach[net].iter().copied().collect()).collect();

        let mut domains: Vec<(usize, Vec<usize>)> = found
            .grounds
            .iter()
            .map(|&g| (g, (0..parts.len()).filter(|&p| found.home[p] == Some(g)).collect::<Vec<usize>>()))
            .filter(|(_, own)| !own.is_empty())
            .collect();
        domains.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(net_name(a.0).cmp(&net_name(b.0))));
        let adrift: Vec<usize> = (0..parts.len()).filter(|&p| found.home[p].is_none() && !stays[p]).collect();
        let power_layers: Vec<String> = board
            .stack
            .iter()
            .filter(|layer| layer.way == Way::Plane)
            .skip(1)
            .map(|layer| layer.name.clone())
            .collect();
        // What disturbs, and what is disturbed. A part disturbs if that is said of it; and without being told, a
        // chip that switches an inductor (see the loops), with that inductor and what else is on the node. A part
        // is disturbed if that is said of it, or it has a pad on a sensitive net; but not one that disturbs.
        let mut aggressor: Vec<bool> = parts.iter().map(|part| part.noisy == Some(true)).collect();
        for node in (0..all_nets.len()).filter(|&net| is_signal(net) && reach[net].len() <= SWITCHED) {
            let on: Vec<usize> = reach[node].iter().copied().collect();
            let chips = on.iter().filter(|&&q| is_chip(q)).count();
            let coils = on.iter().filter(|&&q| parts[q].inductor).count();
            if chips == 1 && coils == 1 {
                for &q in on.iter().filter(|&&q| parts[q].noisy != Some(false) && !parts[q].capacitor) {
                    aggressor[q] = true;
                }
            }
        }
        let victim: Vec<bool> = (0..parts.len())
            .map(|p| {
                let on_sensitive = parts[p].pads.iter().flatten().any(|&net| all_nets[net].sensitive);
                !aggressor[p] && parts[p].quiet != Some(false) && (parts[p].quiet == Some(true) || on_sensitive)
            })
            .collect();
        let domain_at: BTreeMap<usize, usize> = domains.iter().enumerate().map(|(d, (g, _))| (*g, d)).collect();
        Ok(Known {
            board,
            parts,
            all_nets,
            stays,
            mate_of,
            found,
            reach,
            signal,
            nets,
            domains,
            adrift,
            domain_at,
            power_layers,
            aggressor,
            victim,
        })
    }
}

impl Known<'_> {
    /// Whether a part is what a capacitor may stand for: one of more than two pads that is no capacitor and
    /// no connector.
    pub(super) fn is_chip(&self, q: usize) -> bool {
        let parts = self.parts;
        !parts[q].capacitor && !parts[q].edge && parts[q].pads.len() > 2
    }

    /// Whether a part is a chip or a connector: what a small net is there for.
    pub(super) fn is_head(&self, q: usize) -> bool {
        self.is_chip(q) || self.parts[q].edge
    }

    pub(super) fn name(&self, p: usize) -> String {
        self.parts[p].name.clone()
    }

    /// The names of some parts, in order.
    pub(super) fn names(&self, list: &[usize]) -> Vec<String> {
        let mut out: Vec<String> = list.iter().map(|&p| self.name(p)).collect();
        out.sort();
        out
    }

    pub(super) fn net_name(&self, net: usize) -> String {
        self.all_nets[net].name.clone()
    }

    /// Whether a net is no ground and no supply.
    pub(super) fn is_signal(&self, net: usize) -> bool {
        !self.found.grounds.contains(&net) && !self.found.rails.contains_key(&net)
    }

    /// The sheet a part is drawn on.
    pub(super) fn sheet(&self, p: usize) -> String {
        let parts = self.parts;
        if parts[p].sheet.is_empty() {
            "(on no sheet)".to_string()
        } else {
            parts[p].sheet.clone()
        }
    }

    /// How many parts a sheet has.
    pub(super) fn sheet_size(&self, name: &str) -> usize {
        (0..self.parts.len()).filter(|&p| self.sheet(p) == name).count()
    }

    /// The kind of a part: its footprint and its value.
    pub(super) fn kind(&self, p: usize) -> String {
        format!("{}|{}", self.parts[p].footprint, self.parts[p].value)
    }

    /// The one chip or connector on a net, if there is one and no more.
    pub(super) fn only_head(&self, net: usize) -> Option<usize> {
        let mut heads = self.reach[net].iter().copied().filter(|&q| self.is_head(q));
        heads.next().filter(|_| heads.next().is_none())
    }

    /// What the schematic says a part's pad on a net is, where it says.
    pub(super) fn pin_on(&self, q: usize, net: usize) -> Pin {
        let parts = self.parts;
        let at = parts[q].pads.iter().position(|&of| of == Some(net));
        at.and_then(|i| parts[q].pins.get(i).copied()).unwrap_or_default()
    }

    /// The domain a net is of: its own if it is a ground, its ground's if a supply, else that of most of its parts.
    pub(super) fn net_domain(&self, net: usize) -> Option<usize> {
        let (found, reach, domain_at) = (&self.found, &self.reach, &self.domain_at);
        if let Some(&domain) = domain_at.get(&net) {
            return Some(domain);
        }
        if let Some(ground) = found.rails.get(&net).and_then(|rail| rail.0) {
            return domain_at.get(&ground).copied();
        }
        let mut count: BTreeMap<usize, usize> = BTreeMap::new();
        for domain in reach[net].iter().filter_map(|&q| found.home[q]).filter_map(|ground| domain_at.get(&ground)) {
            *count.entry(*domain).or_insert(0) += 1;
        }
        count.into_iter().max_by_key(|&(domain, n)| (n, usize::MAX - domain)).map(|best| best.0)
    }

    /// The room a piece of the board has for parts, in mm2: its ground less what is shut.
    pub(super) fn room(&self, r: [f64; 4]) -> f64 {
        (area(r) - self.board.keepouts.iter().map(|&k| overlap(r, k)).sum::<f64>()).max(1.0)
    }

    /// The ground a zone needs, in mm2.
    pub(super) fn need(&self, work: &Work) -> f64 {
        taken_by(self.board, &work.parts) + work.more
    }

    /// The body of a part as it is given: [wide, high].
    pub(super) fn body(&self, p: usize) -> [f64; 2] {
        let side = self.parts[p].area.sqrt();
        self.parts[p].size.unwrap_or([side, side])
    }

    /// The body of a part turned so many quarters.
    pub(super) fn lying(&self, p: usize, quarters: u8) -> [f64; 2] {
        let [w, h] = self.body(p);
        if quarters % 2 == 1 {
            [h, w]
        } else {
            [w, h]
        }
    }

    /// The ground a part takes with its middle at a place, turned so many quarters.
    pub(super) fn body_at(&self, p: usize, at: Pt, quarters: u8) -> [f64; 4] {
        let [w, h] = self.lying(p, quarters);
        [at[0] - w / 2.0, at[1] - h / 2.0, at[0] + w / 2.0, at[1] + h / 2.0]
    }

    /// The box of a part where it stands: its middle, and whether it is turned a quarter.
    pub(super) fn taken(&self, p: usize, (at, turn): (Pt, bool)) -> [f64; 4] {
        let [w, h] = self.body(p);
        let (w, h) = if turn { (h, w) } else { (w, h) };
        [at[0] - w / 2.0, at[1] - h / 2.0, at[0] + w / 2.0, at[1] + h / 2.0]
    }

    /// The ground a part takes where it stands: its box, or its pieces where it is given in pieces.
    pub(super) fn stands_on(&self, p: usize, (at, turn): (Pt, bool)) -> Vec<[f64; 4]> {
        let parts = self.parts;
        if parts[p].pieces.is_empty() {
            return vec![self.taken(p, (at, turn))];
        }
        let piece = |r: &[f64; 4]| {
            let (a, b) = (quartered(u8::from(turn), [r[0], r[1]]), quartered(u8::from(turn), [r[2], r[3]]));
            [at[0] + a[0].min(b[0]), at[1] + a[1].min(b[1]), at[0] + a[0].max(b[0]), at[1] + a[1].max(b[1])]
        };
        parts[p].pieces.iter().map(piece).collect()
    }

    /// Whether a part is held where it stands: one told to stay there (a connector the enclosure places, a
    /// part already laid out by hand) that stands on the board. It is given no other place, lies as it lies,
    /// and its body is shut to every other part; its zone is pulled to it.
    pub(super) fn held(&self, p: usize) -> bool {
        self.parts[p].fixed && self.parts[p].placed && !self.stays[p]
    }

    /// Whether a part is settled before the others: one with holes, one that disturbs, a large one.
    pub(super) fn goes_first(&self, p: usize) -> bool {
        self.parts[p].through || self.aggressor[p] || self.body(p)[0] * self.body(p)[1] >= LARGE
    }

    /// The sides of the board a part takes ground on: 1 the front, 2 the back, 3 both, for a part with holes.
    /// Two parts are in each other's way only if they have a side in common.
    pub(super) fn side_of(&self, p: usize) -> u8 {
        let parts = self.parts;
        if parts[p].through {
            3u8
        } else if parts[p].back {
            2
        } else {
            1
        }
    }

    /// How many quarters a part stands turned in the file today.
    pub(super) fn stands_turned(&self, p: usize) -> u8 {
        ((self.parts[p].angle / 90.0).round() as i64).rem_euclid(4) as u8
    }

    /// Where a pad of a part is from its middle as its footprint is drawn, not as it happens to stand in the
    /// file today: two blocks of one make-up are then laid out by the same steps, and come out the same.
    pub(super) fn drawn_pad(&self, p: usize, i: usize) -> Pt {
        quartered((4 - self.stands_turned(p)) % 4, self.parts[p].offsets[i])
    }

    /// The body of a part turned so many quarters from how it is drawn.
    pub(super) fn drawn_body(&self, p: usize, quarters: u8) -> [f64; 2] {
        let [w, h] = self.body(p);
        if (self.stands_turned(p) + quarters) % 2 == 1 {
            [h, w]
        } else {
            [w, h]
        }
    }

    /// The ground a part takes at a place, turned so many quarters from how it is drawn.
    pub(super) fn drawn_rect(&self, p: usize, at: Pt, quarters: u8) -> [f64; 4] {
        let [w, h] = self.drawn_body(p, quarters);
        [at[0] - w / 2.0, at[1] - h / 2.0, at[0] + w / 2.0, at[1] + h / 2.0]
    }

    /// How far from its pad a part may stand and be by it: within BESIDE_PIN; a large one within its own
    /// length and a millimetre (its pad cannot be nearer than its body allows).
    pub(super) fn by_pad(&self, c: usize) -> f64 {
        BESIDE_PIN.max(self.body(c)[0].max(self.body(c)[1]) + 1.0)
    }

    /// Whether a part is a capacitor with a pad on a supply.
    pub(super) fn of_supplies(&self, c: &usize) -> bool {
        let (parts, found) = (self.parts, &self.found);
        parts[*c].capacitor && parts[*c].pads.iter().flatten().any(|net| found.rails.contains_key(net))
    }
}
