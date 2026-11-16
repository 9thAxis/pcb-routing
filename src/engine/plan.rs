//! What follows from the problem alone and does not change during a run: worked out once, used by every attempt.

use super::dear::DearGround;
use super::fixed::Fixed;
use super::space::{Space, Steps, Term};
use crate::geom::{dist, inside};
use crate::grid::Grid;
use crate::mask::{layer_bits, VIA_BIT};
use crate::model::*;
use std::collections::HashMap;

pub(crate) struct Plan {
    pub(super) grid: Grid,
    pub(super) space: Space,
    pub(super) steps: Steps,
    /// Per net: which of the sizes in `tr` its track has, and which of those in `vr` its via.
    pub(super) tclass: Vec<usize>,
    pub(super) vclass: Vec<usize>,
    /// The sizes of track and via a net may have, as (half width, quiet or not): one set of claims is kept per size.
    pub(super) tr: Vec<(f64, bool)>,
    pub(super) vr: Vec<(f64, bool)>,
    /// Per net and routing layer: the cost of a grid step there, as a factor.
    pub(super) layer_cost: Vec<Vec<f64>>,
    /// Per net, sorted: the cells where it has a hole from the start (a hole of its electrical net, or a
    /// terminal on more than one layer), so that a layer change there needs no new via.
    pub(super) fixed_free: Vec<Vec<u32>>,
    /// Other entries of the same electrical net: their copper is no obstacle and their vias count as holes of the net.
    pub(super) siblings: Vec<Vec<usize>>,
    /// Per net: the entries of other nets of its bus (see `Net::bus`).
    pub(super) mates: Vec<Vec<usize>>,
    /// Per net and terminal: its cell and layers, or why it cannot be reached at all.
    pub(super) terms: Vec<Vec<Result<Term, String>>>,
    /// Per net and terminal: the piece of copper it stands on (see `Terminal::piece`), a number of its own
    /// for a terminal that stands alone.
    pub(super) pieces: Vec<Vec<u64>>,
    /// Per net: the exact place of what stands in a cell (a hole, a terminal), where a track ending in that
    /// cell is to end. As it is before any pair is split: each attempt works on a copy.
    pub(super) exact: Vec<HashMap<u32, Pt>>,
    /// Per net: its terminals that are left one way only (see `Terminal::heading`).
    pub(super) heads: Vec<Vec<Head>>,
    /// Per net that is the trunk of a pair: for each of the eight directions, how many steps it runs straight
    /// before a layer change and as many after it. Its two vias stand side by side across its way (see
    /// `Pair::via_pitch`), and each leg needs that much of a straight run to come to its own and back.
    pub(super) runs: Vec<Option<[usize; 8]>>,
    /// The offsets of the cells too close to a hole for another hole.
    pub(super) shut: Vec<(i32, i32)>,
    /// And those too close to a hole that stands anywhere within a cell (the via of a pair's leg).
    pub(super) shut_off_grid: Vec<(i32, i32)>,
    /// The ground that costs more without being closed.
    pub(super) dear: DearGround,
}

/// How many steps in each direction the trunk of a pair runs straight on either side of a layer change: what
/// a leg takes to come aside to its via at 45 degrees, and what the leg on the inside of a corner is shorter
/// than the trunk.
fn run_of(p: &Problem, pair: &Pair) -> [usize; 8] {
    let length = (pair.via_pitch / 2.0).max(pair.pitch);
    super::DIRS.map(|(rows, columns)| {
        (length / (p.step * ((rows * rows + columns * columns) as f64).sqrt()) - 1e-9).ceil() as usize
    })
}

fn class_of(list: &mut Vec<(f64, bool)>, v: (f64, bool)) -> usize {
    match list.iter().position(|x| (x.0 - v.0).abs() < 1e-9 && x.1 == v.1) {
        Some(i) => i,
        None => {
            list.push(v);
            list.len() - 1
        }
    }
}

impl Plan {
    pub(super) fn new(p: &Problem, grid: Grid, fixed: &Fixed) -> Plan {
        let space = Space::new(&grid, p.layers.len());
        let (mut tr, mut vr) = (Vec::new(), Vec::new());
        let tclass: Vec<usize> = p.nets.iter().map(|n| class_of(&mut tr, (n.width / 2.0, n.quiet))).collect();
        let vclass: Vec<usize> = p.nets.iter().map(|n| class_of(&mut vr, (n.via[0] / 2.0, n.quiet))).collect();
        let layer_cost = p
            .nets
            .iter()
            .map(|n| p.layers.iter().map(|l| n.layer_cost.get(l).copied().unwrap_or(1.0)).collect())
            .collect();
        // the exact place of every hole and terminal of a net
        let mut exact: Vec<HashMap<u32, Pt>> = Vec::new();
        for n in &p.nets {
            let holes = p.holes.iter().filter(|h| h.net.as_deref() == Some(n.net.as_str())).map(|h| h.xy);
            let terminals = n.terminals.iter().map(|t| t.xy);
            exact.push(holes.chain(terminals).filter_map(|xy| Some((grid.cell(xy)? as u32, xy))).collect());
        }
        let fixed_free: Vec<Vec<u32>> = (0..p.nets.len()).map(|n| fixed.free(n).to_vec()).collect();
        let mut terms = Vec::new();
        let mut heads: Vec<Vec<Head>> = Vec::new();
        for (n, net) in p.nets.iter().enumerate() {
            let mut list = Vec::new();
            heads.push(Vec::new());
            for t in &net.terminals {
                let at = relocate(p, &grid, &|cell| fixed.mask(n, cell), net, t);
                if let (Ok(term), Some(way)) = (&at, t.heading) {
                    let way = super::nearest_dir(way) as u8;
                    heads[n].push(Head { cell: term.cell, way, hand: t.hand, pass: t.pass });
                }
                // a track that ends on another cell than the terminal's own does not end at the terminal's point
                if let (Ok(term), Some(own)) = (&at, grid.cell(t.xy)) {
                    if term.cell != own as u32 {
                        exact[n].remove(&(own as u32));
                    }
                }
                list.push(at);
            }
            terms.push(list);
        }
        // a track ends at the exact place only where the piece that leads there from the cell is clear
        for (n, places) in exact.iter_mut().enumerate() {
            places.retain(|&cell, xy| reaches_clear(fixed, &grid, n, cell as usize, *xy));
        }
        Plan {
            steps: Steps::new(grid.w),
            dear: DearGround::new(p, &grid, space),
            shut: grid.disc(p.rules.hole_centre_mm),
            shut_off_grid: grid.disc(p.rules.hole_centre_mm + p.step),
            siblings: (0..p.nets.len())
                .map(|n| (0..p.nets.len()).filter(|&m| m != n && p.nets[m].net == p.nets[n].net).collect())
                .collect(),
            mates: (0..p.nets.len())
                .map(|n| {
                    let same = |m: &usize| p.nets[*m].net != p.nets[n].net && p.nets[*m].bus == p.nets[n].bus;
                    (0..p.nets.len()).filter(|m| p.nets[n].bus.is_some() && same(m)).collect()
                })
                .collect(),
            grid,
            space,
            tclass,
            vclass,
            tr,
            vr,
            layer_cost,
            fixed_free,
            terms,
            pieces: p.nets.iter().map(|n| (0..n.terminals.len()).map(|t| piece_of(n, t)).collect()).collect(),
            exact,
            heads,
            runs: p
                .nets
                .iter()
                .map(|n| {
                    let changes = |pair: &&Pair| pair.trunk == n.key && pair.via_pitch > 0.0 && !n.no_vias;
                    p.pairs.iter().find(changes).map(|pair| run_of(p, pair))
                })
                .collect(),
        }
    }
}

/// The piece of copper terminal `t` of a net stands on: the number it is given, or one of its own (beyond
/// any that can be given) for a terminal that stands alone. `t` may count on past the net's terminals, for
/// those an attempt adds.
pub(super) fn piece_of(net: &Net, t: usize) -> u64 {
    net.terminals.get(t).and_then(|terminal| terminal.piece).map_or((1u64 << 32) + t as u64, u64::from)
}

/// Whether a track of net `n` may run from the centre of `cell` to the point `xy` (the exact place of a pad or
/// hole in that cell): on every layer open to it at the cell, it must keep its distances all the way. The
/// cells are tested at their centres only, and the last piece of a track, off the grid, can come nearer a
/// neighbouring pad than the centre does (a track wider than its own pad, on pads at a fine pitch).
fn reaches_clear(fixed: &Fixed, grid: &Grid, n: usize, cell: usize, xy: Pt) -> bool {
    fixed.open_along(n, grid.xy(cell), xy, !fixed.mask(n, cell) & !VIA_BIT)
}

/// A terminal that is left one way only.
#[derive(Clone, Copy, Debug)]
pub(super) struct Head {
    pub(super) cell: u32,
    /// The way it is left, as an index into the eight directions.
    pub(super) way: u8,
    /// The hand it is on (see `Terminal::hand`), 0 for none.
    pub(super) hand: u8,
    /// Whether the net runs on through it (see `Terminal::pass`): it is then come to that way too.
    pub(super) pass: bool,
}

/// Where a terminal of `net` is reached, or why it cannot be. Its own cell if a track of the net may end there
/// (`mask` gives the net's fixed mask at a cell); else the nearest open cell on the same piece of the net's copper
/// (a pad too near the board edge or another pad is entered where there is room), and the track then ends on that
/// cell, not at the terminal's own point.
pub(super) fn relocate(
    p: &Problem,
    grid: &Grid,
    mask: &dyn Fn(usize) -> u8,
    net: &Net,
    t: &Terminal,
) -> Result<Term, String> {
    let wanted = layer_bits(&p.layers, &t.layers);
    let Some(cell) = grid.cell(t.xy) else {
        return Err("is outside the window".to_string());
    };
    let bits = wanted & !mask(cell);
    if bits != 0 {
        return Ok(Term { cell: cell as u32, layers: bits });
    }
    let mut best: Option<(f64, Term)> = None;
    let reach = (2.0 / p.step).ceil() as i32;
    for c in grid.around(cell, &Grid::square(reach)) {
        let open = wanted & !mask(c);
        if open == 0 {
            continue;
        }
        let q = grid.xy(c);
        let far = dist(q, t.xy);
        if best.is_some_and(|b| far >= b.0) {
            continue;
        }
        // on a layer where one piece of the net's copper holds both the terminal and this cell
        let mut on = 0u8;
        for piece in &p.copper {
            if piece.net.as_deref() == Some(net.net.as_str()) && inside(&piece.shape, t.xy) && inside(&piece.shape, q) {
                on |= layer_bits(&p.layers, &piece.layers);
            }
        }
        if open & on != 0 {
            best = Some((far, Term { cell: c as u32, layers: open & on }));
        }
    }
    match best {
        Some((_, term)) => Ok(term),
        None => Err(format!("is closed by fixed copper or a closed region on {:?}", t.layers)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// One net "A" with a terminal at (1, 1) on the given layers, on a grid of 41 columns by 21 rows.
    fn problem(layers: &[&str], copper: Vec<Value>) -> Problem {
        serde_json::from_value(json!({
            "window": [0.0, 0.0, 4.0, 2.0], "step": 0.1, "layers": ["F", "B"], "copper": copper,
            "nets": [{"key": "a", "net": "A", "width": 0.15, "via": [0.45, 0.2], "layers": ["F", "B"],
                      "clearance": {"default": 0.17},
                      "terminals": [{"xy": [1.0, 1.0], "layers": layers}, {"xy": [3.0, 1.0], "layers": ["F"]}]}]
        }))
        .unwrap()
    }

    fn pad(net: &str, a: [f64; 2], b: [f64; 2]) -> Value {
        json!({"t": "seg", "a": a, "b": b, "w": 0.3, "net": net, "layers": ["F"]})
    }

    /// A mask with layer F closed in columns 0..=11 of rows 0..=13: round the terminal, which is in column 10 of
    /// row 10.
    fn closed_round_the_terminal(grid: &Grid) -> Vec<u8> {
        let mut mask = vec![0u8; grid.cells()];
        for row in 0..=13 {
            for column in 0..=11 {
                mask[row * grid.w + column] = 1;
            }
        }
        mask
    }

    fn first(p: &Problem, mask: &[u8]) -> Result<Term, String> {
        let grid = Grid::new(p.window, p.step);
        relocate(p, &grid, &|cell| mask[cell], &p.nets[0], &p.nets[0].terminals[0])
    }

    #[test]
    fn an_open_terminal_stays_on_its_cell_on_its_open_layers() {
        let p = problem(&["F", "B"], vec![]);
        let grid = Grid::new(p.window, p.step);
        assert_eq!((grid.w, grid.h), (41, 21));
        let own = (10 * 41 + 10) as u32;
        assert_eq!(first(&p, &vec![0u8; grid.cells()]), Ok(Term { cell: own, layers: 0b11 }));
        assert_eq!(first(&p, &closed_round_the_terminal(&grid)), Ok(Term { cell: own, layers: 0b10 }));
    }

    #[test]
    fn a_closed_terminal_ends_on_the_nearest_open_cell_of_its_copper() {
        // The nearest open cell is two columns to the right, off the pad; the pad runs upwards, and its first
        // open cell is four rows up.
        let p = problem(&["F"], vec![pad("A", [1.0, 1.0], [1.0, 1.8])]);
        let grid = Grid::new(p.window, p.step);
        let mask = closed_round_the_terminal(&grid);
        assert_eq!(first(&p, &mask), Ok(Term { cell: (14 * 41 + 10) as u32, layers: 0b01 }));
    }

    #[test]
    fn a_closed_terminal_with_no_copper_to_an_open_cell_cannot_be_reached() {
        let grid = Grid::new([0.0, 0.0, 4.0, 2.0], 0.1);
        let mask = closed_round_the_terminal(&grid);
        let cases = [
            vec![],                                 // no copper at all
            vec![pad("B", [1.0, 1.0], [1.0, 1.8])], // another net's
            vec![pad("A", [1.0, 1.0], [1.0, 1.1])], // its own, but all of it closed
            // two pieces: none holds both points
            vec![pad("A", [1.0, 1.0], [1.0, 1.1]), pad("A", [1.0, 1.3], [1.0, 1.8])],
        ];
        for copper in cases {
            let p = problem(&["F"], copper);
            let why = first(&p, &mask).unwrap_err();
            assert!(why.starts_with("is closed by fixed copper"), "{why}");
        }
    }

    #[test]
    fn a_terminal_outside_the_window_cannot_be_reached() {
        let mut p = problem(&["F"], vec![]);
        p.nets[0].terminals[0].xy = [5.0, 1.0];
        let grid = Grid::new(p.window, p.step);
        assert_eq!(first(&p, &vec![0u8; grid.cells()]), Err("is outside the window".to_string()));
    }

    #[test]
    fn the_plan_forgets_the_point_of_a_terminal_that_is_reached_elsewhere() {
        let p = problem(&["F"], vec![pad("A", [1.0, 1.0], [1.0, 1.8])]);
        let grid = Grid::new(p.window, p.step);
        // closed to every net up to column 11 and row 13, as in `closed_round_the_terminal`
        let mut p = p;
        p.regions = vec![serde_json::from_value(
            json!({"rings": [[[-1.0, -1.0], [1.04, -1.0], [1.04, 1.24], [-1.0, 1.24]]], "layers": ["F"], "vias": false}),
        )
        .unwrap()];
        let fixed = Fixed::new(&p, &grid, Default::default());
        let plan = Plan::new(&p, grid, &fixed);
        assert_eq!(plan.terms[0][0], Ok(Term { cell: 584, layers: 1 }));
        assert_eq!(plan.terms[0][1], Ok(Term { cell: 440, layers: 1 }));
        assert!(!plan.exact[0].contains_key(&420));
        assert_eq!(plan.exact[0].get(&440), Some(&[3.0, 1.0]));
    }
}
