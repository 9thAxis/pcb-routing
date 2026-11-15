//! Index arithmetic: states and cells, terminals, and the eight steps.

use super::DIRS;
use crate::grid::Grid;

/// How the states of a problem are numbered: a state is (layer, cell), layer by layer; a cell is (row,
/// column), row by row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Space {
    /// Columns of the grid.
    pub(crate) w: usize,
    /// Cells of one layer.
    pub(crate) hw: usize,
    /// Routing layers.
    pub(crate) nl: usize,
}

impl Space {
    pub(crate) fn new(grid: &Grid, layers: usize) -> Space {
        Space { w: grid.w, hw: grid.cells(), nl: layers }
    }

    /// Number of (layer, cell) states.
    #[inline]
    pub(crate) fn states(&self) -> usize {
        self.nl * self.hw
    }

    #[inline]
    pub(crate) fn state(&self, layer: usize, cell: usize) -> usize {
        layer * self.hw + cell
    }

    /// (layer, cell) of a state.
    #[inline]
    pub(crate) fn split(&self, state: usize) -> (usize, usize) {
        (state / self.hw, state % self.hw)
    }

    /// (row, column) of a cell.
    #[inline]
    pub(crate) fn rc(&self, cell: usize) -> (usize, usize) {
        (cell / self.w, cell % self.w)
    }
}

/// Where a net is to be reached: a cell, and the layers on which it may be, a bit each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Term {
    pub(crate) cell: u32,
    pub(crate) layers: u8,
}

impl Term {
    #[inline]
    pub(crate) fn on(&self, layer: usize) -> bool {
        self.layers & (1 << layer) != 0
    }

    /// Its states, lowest layer first.
    pub(crate) fn states(self, space: Space) -> impl Iterator<Item = u32> {
        (0..space.nl).filter(move |&l| self.on(l)).map(move |l| (l * space.hw) as u32 + self.cell)
    }
}

/// The eight steps from a cell, in the order of `DIRS`: what each adds to the cell's number, and its length in
/// grid steps.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Steps {
    pub(crate) off: [isize; 8],
    pub(crate) len: [f64; 8],
}

impl Steps {
    pub(crate) fn new(w: usize) -> Steps {
        let w = w as i32;
        Steps {
            off: DIRS.map(|(dj, di)| (dj * w + di) as isize),
            len: DIRS.map(|(dj, di)| if dj != 0 && di != 0 { std::f64::consts::SQRT_2 } else { 1.0 }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn space() -> Space {
        Space::new(&Grid::new([0.0, 0.0, 0.6, 0.4], 0.1), 3)
    }

    #[test]
    fn states_and_cells_come_back_as_they_went_in() {
        let s = space();
        assert_eq!((s.w, s.hw, s.nl, s.states()), (7, 35, 3, 105));
        for layer in 0..s.nl {
            for cell in 0..s.hw {
                let state = s.state(layer, cell);
                assert_eq!(s.split(state), (layer, cell));
                let (row, column) = s.rc(cell);
                assert_eq!(row * s.w + column, cell);
                assert!(column < s.w);
            }
        }
        assert_eq!(s.state(2, 34), s.states() - 1);
        assert_eq!(s.rc(34), (4, 6));
    }

    #[test]
    fn a_terminal_has_a_state_per_layer_it_is_on() {
        let s = space();
        let t = Term { cell: 9, layers: 0b101 };
        assert!(t.on(0) && !t.on(1) && t.on(2));
        assert_eq!(t.states(s).collect::<Vec<_>>(), vec![9, 79]);
        assert_eq!(Term { cell: 9, layers: 0 }.states(s).count(), 0);
        // layers the problem does not have are not states
        assert_eq!(Term { cell: 0, layers: 0xFF }.states(s).collect::<Vec<_>>(), vec![0, 35, 70]);
    }

    #[test]
    fn steps_follow_the_directions() {
        let steps = Steps::new(7);
        for (d, &(dj, di)) in DIRS.iter().enumerate() {
            assert_eq!(steps.off[d], (dj * 7 + di) as isize);
            assert_eq!(steps.len[d] > 1.0, dj != 0 && di != 0);
        }
        // the fixed look-ahead keeps its lengths in single precision
        assert_eq!(std::f64::consts::SQRT_2 as f32, std::f32::consts::SQRT_2);
    }
}
