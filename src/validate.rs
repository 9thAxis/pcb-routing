//! Checks on a problem before it is run: everything that would otherwise loop, overflow or give a misleading
//! answer is refused here by name.

use crate::error::{invalid, Error};
use crate::grid::Grid;
use crate::model::*;
use std::collections::HashSet;

/// The search numbers its states (direction, layer, cell) in 32 bits, nine per (layer, cell).
const STATE_LIMIT: usize = (u32::MAX / 9) as usize;
/// Claims are counted per cell in a byte.
const MOST_NETS: usize = 250;

/// The greatest factor on the cost of dear ground: beyond it the ground is as good as closed.
const MOST_DEAR: f64 = 100.0;

fn positive(field: String, v: f64) -> Result<(), Error> {
    if v.is_finite() && v > 0.0 {
        Ok(())
    } else {
        Err(invalid(field, format!("must be a positive number, is {v}")))
    }
}

fn not_negative(field: String, v: f64) -> Result<(), Error> {
    if v.is_finite() && v >= 0.0 {
        Ok(())
    } else {
        Err(invalid(field, format!("must be zero or more, is {v}")))
    }
}

fn point(field: String, p: Pt) -> Result<(), Error> {
    if p[0].is_finite() && p[1].is_finite() {
        Ok(())
    } else {
        Err(invalid(field, "must be finite"))
    }
}

fn rings(field: &str, rings: &[Vec<Pt>]) -> Result<(), Error> {
    for (r, ring) in rings.iter().enumerate() {
        for (k, p) in ring.iter().enumerate() {
            point(format!("{field}[{r}][{k}]"), *p)?;
        }
    }
    Ok(())
}

impl Problem {
    /// Check that the problem can be run: sizes and costs that make sense, names that exist, a grid that fits.
    /// [`route`](crate::route) does this first; it is here for callers who want the answer without a run.
    pub fn validate(&self) -> Result<(), Error> {
        let o = &self.options;
        positive("step".into(), self.step)?;
        for (k, v) in self.window.iter().enumerate() {
            if !v.is_finite() {
                return Err(invalid(format!("window[{k}]"), "must be finite"));
            }
        }
        if self.window[2] < self.window[0] || self.window[3] < self.window[1] {
            return Err(invalid("window", "is [x0, y0, x1, y1] with x1 >= x0 and y1 >= y0"));
        }
        if self.layers.is_empty() || self.layers.len() > 7 {
            return Err(invalid(
                "layers",
                format!("between one and seven routing layers, {} given", self.layers.len()),
            ));
        }
        let mut names = HashSet::new();
        for (k, l) in self.layers.iter().enumerate() {
            if !names.insert(l.as_str()) {
                return Err(invalid(format!("layers[{k}]"), format!("layer {l} is named twice")));
            }
        }
        // the cell count in floating point first: a tiny step must not overflow the count itself
        let across = |a: f64, b: f64| ((b + self.step / 2.0 - a) / self.step).ceil();
        let states =
            across(self.window[0], self.window[2]) * across(self.window[1], self.window[3]) * self.layers.len() as f64;
        let limit = o.max_states.min(STATE_LIMIT);
        if states > limit as f64 {
            return Err(Error::TooLarge { states: states.min(usize::MAX as f64) as usize, limit });
        }
        debug_assert!(Grid::new(self.window, self.step).cells() * self.layers.len() <= limit);

        let r = &self.rules;
        not_negative("rules.hole_centre_mm".into(), r.hole_centre_mm)?;
        not_negative("rules.pad_drill_margin_mm".into(), r.pad_drill_margin_mm)?;
        not_negative("rules.region_margin_mm".into(), r.region_margin_mm)?;
        not_negative("rules.new_clearance_mm".into(), r.new_clearance_mm)?;
        not_negative("rules.quiet_clearance_mm".into(), r.quiet_clearance_mm)?;

        not_negative("options.bend_mm".into(), o.bend_mm)?;
        not_negative("options.present_start".into(), o.present_start)?;
        not_negative("options.present_max".into(), o.present_max)?;
        not_negative("options.history_step".into(), o.history_step)?;
        not_negative("options.sweep_margin".into(), o.sweep_margin)?;
        not_negative("options.bus_pull".into(), o.bus_pull)?;
        not_negative("options.bus_cross".into(), o.bus_cross)?;
        if o.bus_cross > 100.0 {
            return Err(invalid("options.bus_cross", format!("is {}: at most 100", o.bus_cross)));
        }
        if o.bus_pull > 10.0 {
            return Err(invalid(
                "options.bus_pull",
                format!("is {}: at most 10 (a net ten times as long to stay beside its bus)", o.bus_pull),
            ));
        }
        if !(o.present_growth.is_finite() && o.present_growth >= 1.0) {
            return Err(invalid("options.present_growth", format!("must be 1 or more, is {}", o.present_growth)));
        }

        for (k, c) in self.copper.iter().enumerate() {
            match &c.shape {
                Shape::Seg { a, b, w } => {
                    point(format!("copper[{k}].a"), *a)?;
                    point(format!("copper[{k}].b"), *b)?;
                    not_negative(format!("copper[{k}].w"), *w)?;
                }
                Shape::Circle { c, r } => {
                    point(format!("copper[{k}].c"), *c)?;
                    not_negative(format!("copper[{k}].r"), *r)?;
                }
                Shape::Poly { rings: rs } => rings(&format!("copper[{k}].rings"), rs)?,
            }
        }
        for (k, h) in self.holes.iter().enumerate() {
            point(format!("holes[{k}].xy"), h.xy)?;
        }
        let region = |field: String, g: &Region| -> Result<(), Error> {
            rings(&format!("{field}.rings"), &g.rings)?;
            for l in g.layers.iter().flatten() {
                if !names.contains(l.as_str()) {
                    return Err(invalid(format!("{field}.layers"), format!("{l} is not a routing layer")));
                }
            }
            Ok(())
        };
        for (k, g) in self.regions.iter().enumerate() {
            region(format!("regions[{k}]"), g)?;
        }
        for (k, d) in self.dear.iter().enumerate() {
            rings(&format!("dear[{k}].rings"), &d.rings)?;
            for l in d.layers.iter().flatten() {
                if !names.contains(l.as_str()) {
                    return Err(invalid(format!("dear[{k}].layers"), format!("{l} is not a routing layer")));
                }
            }
            if !(d.factor.is_finite() && (1.0..=MOST_DEAR).contains(&d.factor)) {
                return Err(invalid(format!("dear[{k}].factor"), format!("is {}: from 1 to {MOST_DEAR}", d.factor)));
            }
        }

        if self.nets.len() >= MOST_NETS {
            return Err(invalid(
                "nets",
                format!("at most {} entries in one run, {} given", MOST_NETS - 1, self.nets.len()),
            ));
        }
        let stub_keys: HashSet<&str> = self.pairs.iter().flat_map(|p| p.stub_keys().map(|(_, key)| key)).collect();
        let mut keys = HashSet::new();
        let widest = (self.window[2] - self.window[0]).max(self.window[3] - self.window[1]).max(self.step);
        for (field, v) in
            [("new_clearance_mm", self.rules.new_clearance_mm), ("quiet_clearance_mm", self.rules.quiet_clearance_mm)]
        {
            if v > widest {
                return Err(invalid(format!("rules.{field}"), format!("is {v} mm: more than the window is across")));
            }
        }
        for (k, n) in self.nets.iter().enumerate() {
            let at = |f: &str| format!("nets[{k}] ({}).{f}", n.key);
            if !keys.insert(n.key.as_str()) {
                return Err(invalid(at("key"), "is used by an earlier entry too"));
            }
            positive(at("width"), n.width)?;
            positive(at("via[0]"), n.via[0])?;
            positive(at("via[1]"), n.via[1])?;
            not_negative(at("via_penalty_mm"), n.via_penalty_mm)?;
            not_negative(at("yield"), n.yield_)?;
            not_negative(at("weight"), n.weight)?;
            not_negative(at("clearance.default"), n.clearance.default)?;
            for (name, v) in &n.clearance.nets {
                not_negative(at(&format!("clearance.nets.{name}")), *v)?;
            }
            // (a slip of a unit, 250 for 0.25, would have the router draw up discs of millions of cells)
            let sizes = [("width", n.width), ("via[0]", n.via[0]), ("clearance.default", n.clearance.default)];
            let named = n.clearance.nets.iter().map(|(name, v)| (format!("clearance.nets.{name}"), *v));
            for (field, v) in sizes.into_iter().map(|(field, v)| (field.to_string(), v)).chain(named) {
                if v > widest {
                    return Err(invalid(at(&field), format!("is {v} mm: more than the window is across")));
                }
            }
            if let Some(m) = n.max_mm {
                not_negative(at("max_mm"), m)?;
            }
            if n.layers.is_empty() {
                return Err(invalid(at("layers"), "names no layer"));
            }
            for l in &n.layers {
                if !names.contains(l.as_str()) {
                    return Err(invalid(at("layers"), format!("{l} is not a routing layer")));
                }
            }
            for (l, v) in &n.layer_cost {
                if !names.contains(l.as_str()) {
                    return Err(invalid(at("layer_cost"), format!("{l} is not a routing layer")));
                }
                // a cost of zero would make every length on that layer equally good, and a negative one has no
                // cheapest path
                positive(at(&format!("layer_cost.{l}")), *v)?;
            }
            let fewest = if stub_keys.contains(n.key.as_str()) { 1 } else { 2 };
            let pieces: HashSet<u32> = n.terminals.iter().filter_map(|t| t.piece).collect();
            let apart = pieces.len() + n.terminals.iter().filter(|t| t.piece.is_none()).count();
            if apart < fewest {
                return Err(invalid(at("terminals"), format!("at least {fewest} not yet joined, {apart} given")));
            }
            // (an entry that passes terminals is one line: its first and its last piece are its ends)
            let mut line: Vec<(Option<u32>, bool)> = Vec::new();
            for term in &n.terminals {
                match line.iter_mut().find(|(piece, _)| term.piece.is_some() && *piece == term.piece) {
                    Some((_, pass)) if *pass != term.pass => {
                        return Err(invalid(at("terminals"), "the terminals of one piece are all passed or none is"));
                    }
                    Some(_) => {}
                    None => line.push((term.piece, term.pass)),
                }
            }
            if line.iter().any(|&(_, pass)| pass) {
                // (each hand lays the whole line: it has terminals of every piece)
                let mut hands: Vec<u8> = n.terminals.iter().map(|t| t.hand).filter(|&hand| hand != 0).collect();
                hands.sort_unstable();
                hands.dedup();
                for hand in hands {
                    let of_hand = |piece: Option<u32>| {
                        piece.is_none()
                            || n.terminals.iter().any(|t| t.piece == piece && (t.hand == 0 || t.hand == hand))
                    };
                    if !line.iter().all(|&(piece, _)| of_hand(piece)) {
                        return Err(invalid(
                            at("terminals"),
                            format!("hand {hand} has no terminal of one of the pieces: every hand lays the whole line"),
                        ));
                    }
                }
            }
            if line.first().is_some_and(|first| first.1) || line.last().is_some_and(|last| last.1) {
                return Err(invalid(
                    at("terminals"),
                    "a terminal that is passed is of neither the first piece nor the last: those are the ends",
                ));
            }
            for (t, term) in n.terminals.iter().enumerate() {
                point(at(&format!("terminals[{t}].xy")), term.xy)?;
                if term.pass && term.heading.is_none() {
                    return Err(invalid(
                        at(&format!("terminals[{t}].pass")),
                        "a terminal that is passed has a heading",
                    ));
                }
                if let Some(way) = term.heading {
                    if !(way[0].is_finite() && way[1].is_finite() && way[0].hypot(way[1]) > 1e-9) {
                        return Err(invalid(at(&format!("terminals[{t}].heading")), "is no direction"));
                    }
                }
                if term.layers.is_empty() {
                    return Err(invalid(at(&format!("terminals[{t}].layers")), "names no layer"));
                }
                for l in &term.layers {
                    if !names.contains(l.as_str()) {
                        return Err(invalid(
                            at(&format!("terminals[{t}].layers")),
                            format!("{l} is not a routing layer"),
                        ));
                    }
                    if !n.layers.contains(l) && term.layers.len() == 1 {
                        return Err(invalid(at(&format!("terminals[{t}].layers")), format!("the net may not use {l}")));
                    }
                }
            }
            for (g, reg) in n.regions.iter().enumerate() {
                region(at(&format!("regions[{g}]")), reg)?;
            }
        }

        let mut pair_keys = HashSet::new();
        let mut used = HashSet::new();
        for (k, pair) in self.pairs.iter().enumerate() {
            let at = |f: &str| format!("pairs[{k}] ({}).{f}", pair.key);
            if !pair_keys.insert(pair.key.as_str()) {
                return Err(invalid(at("key"), "is used by an earlier pair too"));
            }
            positive(at("leg_width"), pair.leg_width)?;
            positive(at("pitch"), pair.pitch)?;
            if pair.pitch <= pair.leg_width {
                return Err(invalid(at("pitch"), "must be more than leg_width (it is centre to centre)"));
            }
            let entry = |field: String, key: &str| {
                self.nets
                    .iter()
                    .find(|n| n.key == key)
                    .ok_or_else(|| invalid(field, format!("no entry of nets has the key {key}")))
            };
            let trunk = entry(at("trunk"), &pair.trunk)?;
            // (two terminals, or more that stand for two pieces: see `Terminal::piece`)
            let mut ends: Vec<Option<u32>> = Vec::new();
            for terminal in &trunk.terminals {
                if terminal.piece.is_none() || !ends.contains(&terminal.piece) {
                    ends.push(terminal.piece);
                }
            }
            if ends.len() != 2 + pair.passes.len() {
                return Err(invalid(
                    at("trunk"),
                    "the trunk entry has two terminals, or the terminals of two pieces: where the pair gathers and \
                     where it parts; and between them one piece for each of `passes`, of terminals with `pass` set",
                ));
            }
            let passed = trunk.terminals.iter().filter(|t| t.pass).map(|t| t.piece).collect::<HashSet<_>>().len();
            if passed != pair.passes.len() {
                return Err(invalid(
                    at("passes"),
                    format!("{} given, and the trunk entry passes {passed} of its pieces", pair.passes.len()),
                ));
            }
            if !(pair.via_pitch >= 0.0 && pair.via_pitch.is_finite()) {
                return Err(invalid(at("via_pitch"), "is a distance, or 0 for a pair that stays on one layer"));
            }
            if pair.via_pitch > 0.0 && pair.via_pitch < self.rules.hole_centre_mm - 1e-9 {
                return Err(invalid(
                    at("via_pitch"),
                    "is less than rules.hole_centre_mm: two holes may not be so near",
                ));
            }
            if pair.via_pitch == 0.0 && !trunk.no_vias {
                return Err(invalid(
                    at("trunk"),
                    "the trunk entry must have no_vias set, or the pair a via_pitch: where it changes layer its \
                     two vias stand that far apart",
                ));
            }
            if trunk.width < pair.pitch + pair.leg_width {
                return Err(invalid(
                    at("trunk"),
                    format!("the trunk entry's width {} is less than pitch + leg_width", trunk.width),
                ));
            }
            if !used.insert(pair.trunk.as_str()) {
                return Err(invalid(at("trunk"), "is part of another pair too"));
            }
            let at_ends = (0..2).map(|end| (format!("stubs[{end}]"), &pair.stubs[end]));
            let passed = (0..pair.passes.len()).map(|pass| (format!("passes[{pass}]"), &pair.passes[pass]));
            for (list, two) in at_ends.chain(passed) {
                for leg in 0..2 {
                    let field = at(&format!("{list}[{leg}]"));
                    let stub = entry(field.clone(), &two[leg])?;
                    if !used.insert(two[leg].as_str()) {
                        return Err(invalid(field, "is used twice in the pairs"));
                    }
                    if !stub.last {
                        return Err(invalid(field, "a stub entry must be marked last"));
                    }
                    if stub.net != pair.legs[leg] {
                        return Err(invalid(
                            field,
                            format!("its net is {}, the leg's is {}", stub.net, pair.legs[leg]),
                        ));
                    }
                    let one = stub.terminals.first().and_then(|t| t.piece);
                    let alike =
                        stub.terminals.len() > 1 && one.is_some() && stub.terminals.iter().all(|t| t.piece == one);
                    if stub.terminals.len() != 1 && !alike {
                        return Err(invalid(
                            field,
                            "a stub entry has one terminal, its pad, or the terminals of one piece: pads that are \
                             joined already, the first of them the one the pair comes to",
                        ));
                    }
                    if pair.via_pitch > 0.0 && pair.via_pitch < stub.via[0] + pair.pitch - pair.leg_width - 1e-9 {
                        return Err(invalid(
                            at("via_pitch"),
                            format!(
                                "two vias of {} so near would be nearer each other than the legs are (pitch - leg_width)",
                                stub.via[0]
                            ),
                        ));
                    }
                    if pair.via_pitch > 0.0 && trunk.via[1] < pair.via_pitch + stub.via[1] - 1e-9 {
                        return Err(invalid(
                            at("trunk"),
                            format!(
                                "the trunk entry's drill of {} is narrower than two of {} side by side, {} apart",
                                trunk.via[1], stub.via[1], pair.via_pitch
                            ),
                        ));
                    }
                    // (a via of another net keeps the copper clearance from the trunk's: that must hold the
                    // hole spacing between it and either of the two)
                    let others = self.nets.iter().filter(|n| n.key != trunk.key && !n.no_vias).map(|n| n.via[0] / 2.0);
                    let ring = trunk.via[0] / 2.0 - pair.via_pitch / 2.0 + self.rules.new_clearance_mm;
                    let least = others.fold(f64::INFINITY, f64::min);
                    if pair.via_pitch > 0.0 && least.is_finite() && ring + least < self.rules.hole_centre_mm - 1e-9 {
                        return Err(invalid(
                            at("trunk"),
                            "the trunk entry's via is too narrow for a via of another net that keeps its distance \
                             from it to keep rules.hole_centre_mm from either of the pair's two",
                        ));
                    }
                    if pair.via_pitch > 0.0 && trunk.via[0] < pair.via_pitch + stub.via[0] - 1e-9 {
                        return Err(invalid(
                            at("trunk"),
                            format!(
                                "the trunk entry's via of {} is narrower than two vias of {} side by side, {} apart",
                                trunk.via[0], stub.via[0], pair.via_pitch
                            ),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}
