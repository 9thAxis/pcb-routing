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

        if self.nets.len() >= MOST_NETS {
            return Err(invalid(
                "nets",
                format!("at most {} entries in one run, {} given", MOST_NETS - 1, self.nets.len()),
            ));
        }
        let stub_keys: HashSet<&str> =
            self.pairs.iter().flat_map(|p| p.stubs.iter().flatten().map(|s| s.as_str())).collect();
        let mut keys = HashSet::new();
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
                // a cost of zero would make every length on that layer equally good, and a negative one has no cheapest path
                positive(at(&format!("layer_cost.{l}")), *v)?;
            }
            let fewest = if stub_keys.contains(n.key.as_str()) { 1 } else { 2 };
            if n.terminals.len() < fewest {
                return Err(invalid(at("terminals"), format!("at least {fewest}, {} given", n.terminals.len())));
            }
            for (t, term) in n.terminals.iter().enumerate() {
                point(at(&format!("terminals[{t}].xy")), term.xy)?;
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
            if trunk.terminals.len() != 2 {
                return Err(invalid(
                    at("trunk"),
                    "the trunk entry has two terminals: where the pair gathers and where it parts",
                ));
            }
            if !trunk.no_vias {
                return Err(invalid(at("trunk"), "the trunk entry must have no_vias set: a pair stays on one layer"));
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
            for end in 0..2 {
                for leg in 0..2 {
                    let field = at(&format!("stubs[{end}][{leg}]"));
                    let stub = entry(field.clone(), &pair.stubs[end][leg])?;
                    if !used.insert(pair.stubs[end][leg].as_str()) {
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
                    if stub.terminals.len() != 1 {
                        return Err(invalid(field, "a stub entry has one terminal, its pad"));
                    }
                }
            }
        }
        Ok(())
    }
}
