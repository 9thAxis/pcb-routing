//! The boards of a box: which domain is on which.

use super::constants::{ANCHOR, BOARD_FULL, BOARD_MOST};
use super::ground::Work;
use super::known::Known;
use super::settle::overlap;
use super::ties::{in_line, tied};
use super::types::{Region, Road};
use crate::model::Pt;
use std::collections::BTreeMap;

/// The boards of the box, in the order they are drawn, and the domains on each.
pub(super) struct Boards {
    /// The ground of each board, as it is drawn.
    pub(super) boards: Vec<[f64; 4]>,
    /// What is added to a place on each to say where that is in the stack.
    pub(super) shifts: Vec<Pt>,
    /// The board each domain is on.
    pub(super) to: Vec<usize>,
    /// The domains each board holds, by name.
    pub(super) held: Vec<Vec<String>>,
}

/// Where the box holds several boards, the domains are parted between them first: in the order their ties
/// give, each board with as many as its room is of all the room (a domain is on one board).
pub(super) fn on_boards(cx: &Known, top: &[Work]) -> Boards {
    let (board, parts, all_nets, found, nets) = (cx.board, cx.parts, cx.all_nets, &cx.found, &cx.nets);
    let mut with_shift: Vec<([f64; 4], Pt)> = if board.regions.is_empty() {
        vec![(board.outline, [0.0, 0.0])]
    } else {
        (0..board.regions.len())
            .map(|k| (board.regions[k], board.shifts.get(k).copied().unwrap_or([0.0, 0.0])))
            .collect()
    };
    with_shift.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]).then(a.0[1].total_cmp(&b.0[1])));
    let boards: Vec<[f64; 4]> = with_shift.iter().map(|board| board.0).collect();
    let shifts: Vec<Pt> = with_shift.iter().map(|board| board.1).collect();
    let mut to: Vec<usize> = vec![0; top.len()];
    if boards.len() > 1 && top.len() > 1 {
        let mut of_top: BTreeMap<usize, usize> = BTreeMap::new();
        for (z, work) in top.iter().enumerate() {
            of_top.extend(work.parts.iter().map(|&p| (p, z)));
        }
        let line = in_line(top.len(), &tied(nets, &|p| of_top.get(&p).copied()));
        let mut order: Vec<usize> = (0..top.len()).collect();
        order.sort_by(|&a, &b| line[a].total_cmp(&line[b]).then(a.cmp(&b)));
        let all_room: f64 = boards.iter().map(|&r| cx.room(r)).sum();
        let all_need: f64 = top.iter().map(|work| cx.need(work)).sum::<f64>().max(1.0);
        // (the order, or the order turned round: whichever leaves each board nearer what its room is of all)
        let parted = |order: &[usize]| -> (f64, Vec<usize>) {
            let (mut to, mut at_board, mut taken, mut off) = (vec![0usize; top.len()], 0usize, 0.0f64, 0.0f64);
            for (k, &z) in order.iter().enumerate() {
                let share = cx.room(boards[at_board]) / all_room * all_need;
                let left = order.len() - k;
                let must_move = left <= boards.len() - 1 - at_board;
                let over = taken + cx.need(&top[z]) - share;
                if at_board + 1 < boards.len() && taken > 0.0 && (must_move || over > share - taken) {
                    off += (taken - share).abs();
                    at_board += 1;
                    taken = 0.0;
                }
                to[z] = at_board;
                taken += cx.need(&top[z]);
            }
            off += (taken - cx.room(boards[at_board]) / all_room * all_need).abs();
            (off, to)
        };
        let turned: Vec<usize> = order.iter().rev().copied().collect();
        let (straight, back) = (parted(&order), parted(&turned));
        to = if straight.0 <= back.0 { straight.1 } else { back.1 };
        // What goes from one board to another goes through a connector between them: where the domains are
        // few enough to try every way, they are parted so that as little as may be is tied across boards,
        // each board holding no more than BOARD_FULL of its room; of ways as good, the most even.
        let ways = (boards.len() as f64).powi(top.len() as i32);
        if ways <= 20_000.0 {
            let mut between = tied(nets, &|p| of_top.get(&p).copied());
            // (a part on two grounds stands on the gap between its two domains: they are on one board)
            for p in (0..parts.len()).filter(|&p| found.stands[p].len() > 1) {
                let on: Vec<usize> = top
                    .iter()
                    .enumerate()
                    .filter(|(_, work)| found.stands[p].iter().any(|&g| all_nets[g].name == work.name))
                    .map(|(z, _)| z)
                    .collect();
                for (i, &a) in on.iter().enumerate() {
                    for &b in &on[i + 1..] {
                        *between.entry((a.min(b), a.max(b))).or_insert(0.0) += ANCHOR;
                    }
                }
            }
            let mut best: Option<(f64, f64, Vec<usize>)> = None;
            // (no board fuller than BOARD_FULL where that can be; else than BOARD_MOST; else as it may)
            for most in [BOARD_FULL, BOARD_MOST, f64::INFINITY] {
                if best.is_some() {
                    break;
                }
                for code in 0..ways as usize {
                    let (mut way, mut rest) = (Vec::with_capacity(top.len()), code);
                    for _ in 0..top.len() {
                        way.push(rest % boards.len());
                        rest /= boards.len();
                    }
                    let fill: Vec<f64> = (0..boards.len())
                        .map(|b| {
                            (0..top.len()).filter(|&z| way[z] == b).map(|z| cx.need(&top[z])).sum::<f64>()
                                / cx.room(boards[b])
                        })
                        .collect();
                    if fill.iter().any(|&f| f > most || f == 0.0) {
                        continue;
                    }
                    let across: f64 = between.iter().filter(|(&(a, b), _)| way[a] != way[b]).map(|(_, w)| *w).sum();
                    let uneven =
                        fill.iter().fold(0.0f64, |m, &f| m.max(f)) - fill.iter().fold(f64::INFINITY, |m, &f| m.min(f));
                    if best.as_ref().map_or(true, |b| (across, uneven) < (b.0, b.1)) {
                        best = Some((across, uneven, way));
                    }
                }
            }
            if let Some((_, _, way)) = best {
                to = way;
            }
        }
    }
    let held: Vec<Vec<String>> = (0..boards.len())
        .map(|b| (0..top.len()).filter(|&z| to[z] == b).map(|z| top[z].name.clone()).collect())
        .collect();
    Boards { boards, shifts, to, held }
}

/// The boards, as they are told: the room of each, what its domains need, and what its roads take.
pub(super) fn regions(cx: &Known, on: &Boards, top: &[Work], roads: &[Road]) -> Vec<Region> {
    let (boards, held) = (&on.boards, &on.held);
    boards
        .iter()
        .zip(held)
        .map(|(&rect, names)| Region {
            rect,
            room: cx.room(rect).round(),
            needed: top.iter().filter(|work| names.contains(&work.name)).map(|work| cx.need(work)).sum::<f64>().round(),
            roads: roads.iter().map(|road| overlap(road.rect, rect)).sum::<f64>().round(),
            zones: names.clone(),
        })
        .collect()
}
