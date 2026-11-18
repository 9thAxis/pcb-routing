//! The floorplan of a board, from its power down to a place for every part.

use super::boards::{on_boards, regions};
use super::known::{standing, Known};
use super::layout::laid_out;
use super::place::{placed, Placed};
use super::planes::{planes, pours};
use super::power::power_told;
use super::report::{flows, gap_parts, junctions, zones_told};
use super::types::{Board, Floorplan};
use super::zones::zone_tree;
use crate::error::Error;

/// The floorplan of a board (see the module). An error if a pad names a net that is not there, or the board
/// has no size.
pub fn floorplan(board: &Board) -> Result<Floorplan, Error> {
    let (stays, held) = standing(board);
    let board = held.as_ref().unwrap_or(board);
    let cx = &Known::of(board, stays)?;
    let power = power_told(cx);
    let (top, apart) = zone_tree(cx);
    let planes = planes(cx, &top);
    let on = on_boards(cx, &top);
    let (best, first) = laid_out(cx, &top, &planes, &on);
    let (laid, mut roads, travel, tall, mut bridges) = best;
    let top = laid;
    let junctions = junctions(board, &travel);
    let Placed {
        blocks,
        alike,
        disturbing,
        close,
        sensitive,
        repeats,
        served,
        chains,
        back,
        loops,
        places,
        parts_now_mm,
        parts_then_mm,
        against,
        pads_first_mm,
        pads_mm,
        bypass,
        capacitors,
    } = placed(cx, &top, &roads, &on, &mut bridges);
    let pours = pours(cx, &planes, &top);
    gap_parts(cx, &mut roads);
    let regions = regions(cx, &on, &top, &roads);
    roads.sort_by(|a, b| a.level.cmp(&b.level).then(b.width.total_cmp(&a.width)));
    let flows = flows(cx, &top);
    let (zones, now_mm, then_mm) = zones_told(cx, &top);
    Ok(Floorplan {
        power,
        zones,
        flows,
        apart,
        now_mm: now_mm.round(),
        then_mm: then_mm.round(),
        regions,
        stranded: travel.left.iter().map(|&k| cx.net_name(cx.signal[k])).collect(),
        tall,
        travel,
        first,
        bridges,
        blocks,
        alike,
        disturbing,
        close,
        sensitive,
        repeats,
        served,
        chains,
        back,
        loops,
        places,
        parts_now_mm: parts_now_mm.round(),
        parts_then_mm: parts_then_mm.round(),
        against: against.into_iter().map(|(kind, count, now, then)| (kind, count, now.round(), then.round())).collect(),
        pads_first_mm: pads_first_mm.round(),
        pads_mm: pads_mm.round(),
        bypass,
        capacitors,
        junctions,
        pours,
        roads,
    })
}
