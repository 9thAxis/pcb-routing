//! Ground that is open but costs more (see [`crate::model::Dear`]), drawn onto the grid once.

use super::space::Space;
use crate::grid::Grid;
use crate::mask::{layer_bits, stamp_polygon, Rect};
use crate::model::Problem;
use std::collections::HashMap;

/// Where the dear ground of a problem is, and what each net pays there.
///
/// Pieces of dear ground overlap (a zone, and the spokes of a pad in it; two zones of different nets), and
/// what a net pays in a cell is the greatest factor among the pieces there that are not its own. So a cell
/// holds the set of kinds of ground that cover it, a kind being a factor and the net it belongs to; the sets
/// that occur are few, and each net has its factor for every one of them.
pub(crate) struct DearGround {
    space: Space,
    /// Per (layer, cell): 0 in ordinary ground, else which set of kinds covers it. Empty for a problem
    /// without dear ground.
    set: Vec<u16>,
    /// Per net and set (0: ordinary ground): the factor on a unit of track length.
    factor: Vec<Vec<f32>>,
    /// Per net: the diameter of its via in grid steps, the length a via pays for.
    via_span: Vec<f64>,
}

/// The sets of kinds met while the ground is drawn: each a sorted list of kinds, set 0 the empty one.
struct Sets {
    kinds: Vec<Vec<u16>>,
    known: HashMap<Vec<u16>, u16>,
    /// (a set, a kind) -> the set with that kind added.
    with: HashMap<(u16, u16), u16>,
}

impl Sets {
    fn new() -> Sets {
        Sets { kinds: vec![Vec::new()], known: HashMap::from([(Vec::new(), 0)]), with: HashMap::new() }
    }

    /// The set `set` with `kind` added. With every number taken (65,535 different sets) the set stays as it is.
    fn add(&mut self, set: u16, kind: u16) -> u16 {
        if let Some(&to) = self.with.get(&(set, kind)) {
            return to;
        }
        let mut kinds = self.kinds[set as usize].clone();
        if let Err(at) = kinds.binary_search(&kind) {
            kinds.insert(at, kind);
        }
        let to = match self.known.get(&kinds) {
            Some(&to) => to,
            None if self.kinds.len() > u16::MAX as usize => set,
            None => {
                self.kinds.push(kinds.clone());
                self.known.insert(kinds, (self.kinds.len() - 1) as u16);
                (self.kinds.len() - 1) as u16
            }
        };
        self.with.insert((set, kind), to);
        to
    }
}

/// The cells a polygon can cover: the rectangle of the grid round its points, None if it lies off the grid.
fn cells_round(grid: &Grid, rings: &[Vec<[f64; 2]>]) -> Option<Rect> {
    let points = || rings.iter().flatten();
    let low = |k: usize| points().map(|p| p[k]).fold(f64::INFINITY, f64::min);
    let high = |k: usize| points().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
    let first = |v: f64, origin: f64| (((v - origin) / grid.step).floor() - 1.0).max(0.0) as usize;
    let last =
        |v: f64, origin: f64, most: usize| ((((v - origin) / grid.step).ceil() + 1.0).max(0.0) as usize).min(most - 1);
    if points().next().is_none() || high(0) < grid.x0 || high(1) < grid.y0 {
        return None;
    }
    let (i0, j0) = (first(low(0), grid.x0), first(low(1), grid.y0));
    let (i1, j1) = (last(high(0), grid.x0, grid.w), last(high(1), grid.y0, grid.h));
    (i0 <= i1 && j0 <= j1).then(|| Rect { i0, j0, w: i1 - i0 + 1, h: j1 - j0 + 1 })
}

impl DearGround {
    pub(super) fn new(p: &Problem, grid: &Grid, space: Space) -> DearGround {
        let mut ground = DearGround {
            space,
            set: Vec::new(),
            factor: vec![vec![1.0]; p.nets.len()],
            via_span: p.nets.iter().map(|n| n.via[0] / p.step).collect(),
        };
        if p.dear.is_empty() {
            return ground;
        }
        // the kinds: a factor, the net it belongs to, and the one net that pays for it
        let mut kinds: Vec<(f64, Option<&str>, Option<&str>)> = Vec::new();
        let mut sets = Sets::new();
        ground.set = vec![0u16; space.states()];
        let mut drawn: Vec<u8> = Vec::new();
        for d in &p.dear {
            let layers = d.layers.as_ref().map_or(u8::MAX, |names| layer_bits(&p.layers, names));
            let Some(rect) = cells_round(grid, &d.rings) else { continue };
            if d.factor <= 1.0 || kinds.len() > u16::MAX as usize {
                continue;
            }
            let this = (d.factor, d.net.as_deref(), d.only.as_deref());
            let kind = kinds.iter().position(|&k| k == this).unwrap_or_else(|| {
                kinds.push(this);
                kinds.len() - 1
            }) as u16;
            drawn.clear();
            drawn.resize(rect.w * rect.h, 0);
            stamp_polygon(grid, rect, &mut drawn, 1, &d.rings, 0.0);
            for (at, _) in drawn.iter().enumerate().filter(|(_, mark)| **mark != 0) {
                let cell = (rect.j0 + at / rect.w) * grid.w + rect.i0 + at % rect.w;
                for layer in (0..space.nl).filter(|l| layers & (1 << l) != 0) {
                    let state = space.state(layer, cell);
                    ground.set[state] = sets.add(ground.set[state], kind);
                }
            }
        }
        // what a net pays in a set: the greatest factor among the kinds that are not its own, and are for it
        let pays = |net: &str, set: &Vec<u16>| -> f32 {
            let kinds = set.iter().map(|&k| kinds[k as usize]);
            let for_it = kinds.filter(|k| k.1 != Some(net) && k.2.map_or(true, |only| only == net));
            for_it.map(|k| k.0 as f32).fold(1.0, f32::max)
        };
        ground.factor = p.nets.iter().map(|n| sets.kinds.iter().map(|set| pays(&n.net, set)).collect()).collect();
        ground
    }

    /// What a step of net `n` into a state costs more for the ground it is in, as a factor: 1 in ordinary ground.
    #[inline]
    pub(super) fn track(&self, n: usize, state: usize) -> f32 {
        if self.set.is_empty() {
            1.0
        } else {
            self.factor[n][self.set[state] as usize]
        }
    }

    /// What a new via of net `n` at a cell costs more for the ground it passes, in grid steps: what a track
    /// of the via's diameter would on the dearest of its layers.
    #[inline]
    pub(super) fn via(&self, n: usize, cell: usize) -> f64 {
        if self.set.is_empty() {
            return 0.0;
        }
        let dearest = (0..self.space.nl).map(|l| self.track(n, self.space.state(l, cell))).fold(1.0f32, f32::max);
        (dearest as f64 - 1.0) * self.via_span[n]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// Nets "a" (net A) and "b" (net B) on a grid of 0.1 mm over 10 mm by 4 mm, two layers, with this dear ground.
    fn ground(dear: Value) -> (DearGround, Grid, Space) {
        let net = |key: &str, name: &str| {
            json!({"key": key, "net": name, "width": 0.2, "via": [0.6, 0.3], "layers": ["F", "B"], "clearance": {"default": 0.2},
                   "terminals": [{"xy": [1.0, 1.0], "layers": ["F"]}, {"xy": [9.0, 1.0], "layers": ["F"]}]})
        };
        let p: Problem = serde_json::from_value(json!({
            "window": [0.0, 0.0, 10.0, 4.0], "step": 0.1, "layers": ["F", "B"], "dear": dear,
            "nets": [net("a", "A"), net("b", "B")]
        }))
        .unwrap();
        let grid = Grid::new(p.window, p.step);
        let space = Space::new(&grid, 2);
        (DearGround::new(&p, &grid, space), grid, space)
    }

    fn square(x: [f64; 2], factor: f64, net: Option<&str>, layers: Option<&[&str]>) -> Value {
        json!({"rings": [[[x[0], 1.0], [x[1], 1.0], [x[1], 3.0], [x[0], 3.0]]], "factor": factor, "net": net, "layers": layers})
    }

    fn at(ground: &(DearGround, Grid, Space), n: usize, layer: usize, xy: [f64; 2]) -> f32 {
        ground.0.track(n, ground.2.state(layer, ground.1.cell(xy).unwrap()))
    }

    #[test]
    fn a_net_pays_the_dearest_ground_that_is_not_its_own() {
        // from x 2 to 6 ground of A at 5, from 4 to 8 ground of nobody at 3: both from 4 to 6
        let g = ground(json!([square([2.0, 6.0], 5.0, Some("A"), None), square([4.0, 8.0], 3.0, None, None)]));
        let pays = |n: usize, x: f64| at(&g, n, 0, [x, 2.0]);
        assert_eq!([pays(0, 1.0), pays(0, 3.0), pays(0, 5.0), pays(0, 7.0), pays(0, 9.0)], [1.0, 1.0, 3.0, 3.0, 1.0]);
        assert_eq!([pays(1, 1.0), pays(1, 3.0), pays(1, 5.0), pays(1, 7.0), pays(1, 9.0)], [1.0, 5.0, 5.0, 3.0, 1.0]);
    }

    #[test]
    fn ground_dear_to_one_net_alone_is_ordinary_ground_to_the_others() {
        let mut beside = square([2.0, 6.0], 5.0, None, None);
        beside["only"] = json!("A");
        let g = ground(json!([beside, square([4.0, 8.0], 3.0, None, None)]));
        let pays = |n: usize, x: f64| at(&g, n, 0, [x, 2.0]);
        assert_eq!([pays(0, 3.0), pays(0, 5.0), pays(0, 7.0)], [5.0, 5.0, 3.0]);
        assert_eq!([pays(1, 3.0), pays(1, 5.0), pays(1, 7.0)], [1.0, 3.0, 3.0]);
    }

    #[test]
    fn the_order_and_the_names_of_the_nets_change_nothing() {
        for (first, second) in [("A", "B"), ("B", "A")] {
            let both = [square([2.0, 6.0], 3.0, Some(first), None), square([2.0, 6.0], 3.0, Some(second), None)];
            for dear in [json!([both[0], both[1]]), json!([both[1], both[0]])] {
                let g = ground(dear);
                assert_eq!([at(&g, 0, 0, [4.0, 2.0]), at(&g, 1, 0, [4.0, 2.0])], [3.0, 3.0]);
            }
        }
    }

    #[test]
    fn ground_is_dear_on_its_layers_only_and_a_via_pays_for_the_dearest() {
        let g = ground(json!([square([2.0, 6.0], 4.0, None, Some(&["B"]))]));
        assert_eq!([at(&g, 0, 0, [4.0, 2.0]), at(&g, 0, 1, [4.0, 2.0])], [1.0, 4.0]);
        let cell = |xy: [f64; 2]| g.1.cell(xy).unwrap();
        // (4 - 1) times a via of 0.6 mm in steps of 0.1
        assert!((g.0.via(0, cell([4.0, 2.0])) - 3.0 * 6.0).abs() < 1e-9);
        assert_eq!(g.0.via(0, cell([8.0, 2.0])), 0.0);
    }

    #[test]
    fn ground_off_the_grid_or_at_its_edge_is_drawn_as_far_as_it_lies_on_it() {
        let g = ground(json!([
            square([-5.0, 0.5], 2.0, None, None),
            square([20.0, 30.0], 2.0, None, None),
            square([9.5, 30.0], 7.0, None, None)
        ]));
        assert_eq!([at(&g, 0, 0, [0.0, 2.0]), at(&g, 0, 0, [0.6, 2.0]), at(&g, 0, 0, [10.0, 2.0])], [2.0, 1.0, 7.0]);
    }

    #[test]
    fn no_dear_ground_costs_nothing() {
        let g = ground(json!([]));
        assert_eq!((at(&g, 0, 0, [4.0, 2.0]), g.0.via(0, 5)), (1.0, 0.0));
    }
}
