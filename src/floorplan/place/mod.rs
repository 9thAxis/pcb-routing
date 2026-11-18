//! A place for every part within its zone.

mod beside;
mod blocks;
mod facing;
mod gaps;
mod measures;
mod pads;
mod refine;
mod settling;
mod units;

use self::beside::beside;
use self::blocks::{alike, blocks};
use self::facing::{Facing, Pins};
use self::gaps::{on_gaps, slid};
use self::measures::{against, by_pads_told, chains, close, loops, places, repeats, sensitive, Against, Back, ByPad};
use self::pads::by_pads;
use self::refine::refined;
use self::settling::{grounds, lies, settled_first, settled_rest, Stood};
use self::units::{units, Units};
use super::boards::Boards;
use super::ground::Work;
use super::known::Known;
use super::types::{Bridge, Loop, Place, Repeat, Road};
use crate::model::Pt;
use std::collections::BTreeMap;

/// Which domain has a name.
pub(super) fn domain_named(top: &[Work], name: &str) -> Option<usize> {
    top.iter().position(|work| work.name == name)
}

/// What the placing of the parts reads at every step: what is known of the board, the layout chosen, and
/// what follows from them for each part.
pub(super) struct Site<'a> {
    pub(super) cx: &'a Known<'a>,
    /// The zones with their ground, a domain at the top of each tree.
    pub(super) top: &'a [Work],
    pub(super) roads: &'a [Road],
    pub(super) on: &'a Boards,
    /// The parts on the gaps: where the middle of each is, and how many quarters it is turned.
    pub(super) on_gap: BTreeMap<usize, Pt>,
    pub(super) gap_quarter: BTreeMap<usize, u8>,
    /// The smallest zones: the domain, the ground and the parts of each.
    pub(super) leaves: Vec<(usize, [f64; 4], Vec<usize>)>,
    /// The domain of each part that is of a zone.
    pub(super) part_domain: BTreeMap<usize, usize>,
    pub(super) domain_of_part: Vec<usize>,
    /// What is added to the place of each part to say where that is in the stack.
    pub(super) shift_of: Vec<Pt>,
}

impl<'a> Site<'a> {
    fn of(
        cx: &'a Known<'a>,
        top: &'a [Work],
        roads: &'a [Road],
        on: &'a Boards,
        on_gap: BTreeMap<usize, Pt>,
        gap_quarter: BTreeMap<usize, u8>,
    ) -> Site<'a> {
        let (parts, shifts, to) = (cx.parts, &on.shifts, &on.to);
        let mut fine_leaves: Vec<(usize, [f64; 4], Vec<usize>)> = Vec::new();
        fn parts_of_leaves(work: &Work, domain: usize, out: &mut Vec<(usize, [f64; 4], Vec<usize>)>) {
            if work.inside.is_empty() {
                out.push((domain, work.rect, work.parts.clone()));
            }
            work.inside.iter().for_each(|inner| parts_of_leaves(inner, domain, out));
        }
        top.iter().enumerate().for_each(|(domain, work)| parts_of_leaves(work, domain, &mut fine_leaves));
        let part_domain: BTreeMap<usize, usize> =
            fine_leaves.iter().flat_map(|(domain, _, own)| own.iter().map(move |&p| (p, *domain))).collect();
        let mut shift_of: Vec<Pt> = vec![[0.0, 0.0]; parts.len()];
        let mut domain_of_part: Vec<usize> = vec![0; parts.len()];
        for (domain, _, own) in &fine_leaves {
            for &p in own {
                (shift_of[p], domain_of_part[p]) = (shifts[to[*domain]], *domain);
            }
        }
        Site { cx, top, roads, on, on_gap, gap_quarter, leaves: fine_leaves, part_domain, domain_of_part, shift_of }
    }

    /// The ground that is shut to every part: what the board shuts, and the bodies on the gaps.
    pub(super) fn closed(&self, gaps: &BTreeMap<usize, [f64; 4]>) -> Vec<[f64; 4]> {
        self.cx.board.keepouts.iter().chain(gaps.values()).copied().collect()
    }
}

/// What the placing of the parts comes to, as the plan tells it.
pub(super) struct Placed {
    pub(super) blocks: [usize; 2],
    pub(super) alike: Vec<(usize, usize, usize, Vec<String>)>,
    pub(super) disturbing: [usize; 2],
    pub(super) close: Vec<(String, String, f64)>,
    pub(super) sensitive: (usize, f64, f64, String),
    pub(super) repeats: Vec<Repeat>,
    pub(super) served: [usize; 2],
    pub(super) chains: usize,
    pub(super) back: Vec<Back>,
    pub(super) loops: Vec<Loop>,
    pub(super) places: Vec<Place>,
    pub(super) parts_now_mm: f64,
    pub(super) parts_then_mm: f64,
    pub(super) against: Vec<Against>,
    pub(super) pads_first_mm: f64,
    pub(super) pads_mm: f64,
    pub(super) bypass: [usize; 2],
    pub(super) capacitors: Vec<ByPad>,
}

/// Where the parts stand, as they are placed.
pub(super) struct Standing {
    /// Where each body came to stand: its middle and whether it is turned; None where there was no room.
    pub(super) stood: BTreeMap<usize, Option<(Pt, bool)>>,
    /// Where the middle of each part is, as its board is drawn; and how many quarters it is turned.
    pub(super) centre: Vec<Option<Pt>>,
    pub(super) quarter: Vec<u8>,
}

/// A place for every part within its zone, and what that comes to. The bridges are told where the parts on
/// the gaps came to stand.
pub(super) fn placed(cx: &Known, top: &[Work], roads: &[Road], on: &Boards, bridges: &mut [Bridge]) -> Placed {
    let gaps = on_gaps(cx, top, on, bridges);
    let mut gap_rects = gaps.rects;
    let site = &Site::of(cx, top, roads, on, gaps.on, gaps.quarter);
    let beside = beside(site);
    let grounds = grounds(site, &beside.fine_nets);
    let mut laid_blocks = blocks(site, &beside.order, &grounds);
    let alike = alike(cx, &laid_blocks);
    let blocks = [laid_blocks.len(), laid_blocks.values().map(|block| block.at.len()).sum()];
    let Units { groups, alternatives } = units(site, &grounds, &mut laid_blocks);
    // (what of a block's parts found no room in it is no part of it, and is placed with the rest)
    let in_block: BTreeMap<usize, usize> =
        laid_blocks.iter().flat_map(|(&head, block)| block.at.iter().map(move |&(p, _, _)| (p, head))).collect();
    let mut on = settled_first(site, &grounds, &gap_rects, &laid_blocks, &alternatives, &in_block);
    settled_rest(site, &grounds, &gap_rects, &in_block, &mut on);
    let (centre, quarter) = lies(site, &grounds, &on);
    let Stood { stood, block_quarter, halos, in_rows } = on;
    let facing = Facing::of(site, &block_quarter);
    let mut standing = Standing { stood, centre, quarter };
    let pads_first_mm = facing.all_reach(&standing.centre, &standing.quarter);
    facing.face(&standing.centre, &mut standing.quarter, &Pins::new());
    let by = by_pads(site, &facing, &beside, &gap_rects, &laid_blocks, &mut standing);
    facing.face(&standing.centre, &mut standing.quarter, &by.pins);
    slid(site, &mut gap_rects, bridges, &mut standing);
    refined(site, &facing, &beside, &by, &gap_rects, &halos, &mut standing);
    facing.face(&standing.centre, &mut standing.quarter, &by.pins);
    let (centre, quarter) = (&standing.centre[..], &standing.quarter[..]);
    let (aggressor, victim) = (&cx.aggressor, &cx.victim);
    let repeats = repeats(cx, &facing, &groups, &in_rows, centre, quarter);
    let close = close(site, centre, quarter);
    let disturbing = [aggressor.iter().filter(|&&is| is).count(), victim.iter().filter(|&&is| is).count()];
    let sensitive = sensitive(cx, &facing, centre, quarter);
    let pads_mm = facing.all_reach(centre, quarter);
    let loops = loops(cx, &facing, &beside.switchers, centre, quarter);
    let (chains, back) = chains(cx, &facing, centre, quarter);
    let (bypass, served, capacitors) = by_pads_told(cx, &facing, &by, &beside.served_parts, centre, quarter);
    let (places, spot) = places(site, &grounds, &gap_rects, &standing);
    let (parts_now_mm, parts_then_mm, against) = against(site, &spot);
    Placed {
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
    }
}
