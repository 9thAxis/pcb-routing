//! The routing grid.

use crate::model::Pt;

/// A cell exactly at the limit distance is closed (as the project's older router has it); this much further still counts as at the limit.
pub(crate) const EPS: f64 = 1e-6;

/// The grid of a problem: cells `step` apart from the lower corner of its window, numbered row by row.
#[derive(Clone, Debug)]
pub struct Grid {
    /// x of column 0.
    pub x0: f64,
    /// y of row 0.
    pub y0: f64,
    /// Distance between neighbouring cells.
    pub step: f64,
    /// Columns.
    pub w: usize,
    /// Rows.
    pub h: usize,
}

impl Grid {
    /// The grid over `window` (`[x0, y0, x1, y1]`), both edges included.
    pub fn new(window: [f64; 4], step: f64) -> Grid {
        // the same count as numpy.arange(start, stop + step / 2, step)
        let n = |a: f64, b: f64| ((b + step / 2.0 - a) / step).ceil() as usize;
        Grid { x0: window[0], y0: window[1], step, w: n(window[0], window[2]), h: n(window[1], window[3]) }
    }
    /// Number of cells.
    pub fn cells(&self) -> usize {
        self.w * self.h
    }
    /// Centre of a cell (cell = row * w + column).
    pub fn xy(&self, cell: usize) -> Pt {
        let (j, i) = (cell / self.w, cell % self.w);
        [round6(self.x0 + i as f64 * self.step), round6(self.y0 + j as f64 * self.step)]
    }
    /// Nearest cell, or None outside the window. A point midway between two cells goes to the even one, as Python's round() does.
    pub fn cell(&self, p: Pt) -> Option<usize> {
        let i = ((p[0] - self.x0) / self.step).round_ties_even();
        let j = ((p[1] - self.y0) / self.step).round_ties_even();
        if i < 0.0 || j < 0.0 || i >= self.w as f64 || j >= self.h as f64 {
            return None;
        }
        Some(j as usize * self.w + i as usize)
    }
    /// Offsets (dj, di) of the cells whose centres lie within `r` of a cell centre.
    pub(crate) fn disc(&self, r: f64) -> Vec<(i32, i32)> {
        let n = (r / self.step + 1e-6).floor() as i32;
        let mut out = Vec::new();
        for dj in -n..=n {
            for di in -n..=n {
                let d2 = ((di * di + dj * dj) as f64) * self.step * self.step;
                if d2 <= (r + EPS) * (r + EPS) {
                    out.push((dj, di));
                }
            }
        }
        out
    }
}

pub(crate) fn round6(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}
