//! A chip and the parts by its pads as one body.

use super::beside::By;
use super::Site;
use crate::floorplan::constants::SETTLE;
use crate::floorplan::known::Known;
use crate::floorplan::settle::{quartered, settled};
use crate::geom::dist;
use crate::model::Pt;
use std::collections::BTreeMap;

/// Parts laid out together and placed as one body.
#[derive(Clone)]
pub(super) struct Block {
    /// Each of its parts: where its middle is from the block's, and how many quarters it is turned.
    pub(super) at: Vec<(usize, Pt, u8)>,
    /// The same as the chip is drawn: where each is from the chip, and its quarters from its own drawing.
    pub(super) drawn: Vec<(usize, Pt, u8)>,
    pub(super) through: bool,
    /// For each part that stands by a pad: (the part, (its pad, the part it stands by, that one's pad)).
    pub(super) pins: Vec<(usize, (usize, usize, usize))>,
}

/// A chip and the parts that stand by its pads, laid out by themselves on empty ground: the chip in the
/// middle, each part beside the pad it is for, in the order they are taken.
fn lay_block(cx: &Known, head: usize, list: &[By]) -> Block {
    let parts = cx.parts;
    // (where each part is from the chip, and how many quarters it is turned from how it is drawn)
    let mut pos: BTreeMap<usize, (Pt, u8)> = [(head, ([0.0, 0.0], 0u8))].into_iter().collect();
    let [hw, hh] = cx.body(head);
    let reach = hw.max(hh) / 2.0 + cx.board.tune.block_reach_mm;
    let ground = [-reach, -reach, reach, reach];
    // (what lies already, with the sides of the board it takes ground on)
    let mut shut: Vec<([f64; 4], u8)> = vec![(cx.drawn_rect(head, [0.0, 0.0], 0), cx.side_of(head))];
    let mut at_pad: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let mut pins: Vec<(usize, (usize, usize, usize))> = Vec::new();
    for &(c, by, net) in list {
        let Some(&(by_at, by_q)) = pos.get(&by) else { continue };
        let pad_of = |i: usize| {
            let from = quartered(by_q, cx.drawn_pad(by, i));
            [by_at[0] + from[0], by_at[1] + from[1]]
        };
        let on_net = (0..parts[by].pads.len()).filter(|&i| parts[by].pads[i] == Some(net));
        // (a part whose other pad is on a net of the same part stands between the two pads: of the pads
        // on this net, the one nearest a pad on that other net; else the pads in turn)
        let partners: Vec<Pt> = parts[c]
            .pads
            .iter()
            .flatten()
            .filter(|&&other| other != net && cx.is_signal(other))
            .flat_map(|&other| (0..parts[by].pads.len()).filter(move |&i| parts[by].pads[i] == Some(other)))
            .map(pad_of)
            .collect();
        let across = |i: usize| {
            let nearest = partners.iter().map(|&there| dist(pad_of(i), there)).fold(f64::INFINITY, f64::min);
            if nearest.is_finite() {
                (nearest / SETTLE).round() as i64
            } else {
                0
            }
        };
        let Some(pin) = on_net.min_by(|&a, &b| {
            let has = |i: usize| at_pad.get(&(by, i)).copied().unwrap_or(0);
            across(a).cmp(&across(b)).then(has(a).cmp(&has(b))).then(a.cmp(&b))
        }) else {
            continue;
        };
        // (just off the body of what it stands by, on the side the pad is nearest, lying away from it)
        let pad = pad_of(pin);
        let by_body = cx.drawn_rect(by, by_at, by_q);
        let [cw, ch] = cx.drawn_body(c, 0);
        let (long, short) = (cw.max(ch), cw.min(ch));
        let half = long / 2.0;
        let sides = [
            (pad[0] - by_body[0], [by_body[0] - half, pad[1]], true),
            (by_body[2] - pad[0], [by_body[2] + half, pad[1]], true),
            (pad[1] - by_body[1], [pad[0], by_body[1] - half], false),
            (by_body[3] - pad[1], [pad[0], by_body[3] + half], false),
        ];
        let (_, want, along_x) = sides.iter().fold(sides[0], |best, side| if side.0 < best.0 { *side } else { best });
        // (a part on the other side of the board from what it stands by stands under the pad itself, as
        // its footprint is drawn or a quarter from that: there is nothing of the other in its way)
        let under = cx.side_of(c) & cx.side_of(by) == 0;
        let (want, lying) =
            if under { (pad, [cw, ch]) } else { (want, if along_x { [long, short] } else { [short, long] }) };
        let quarter_more = (lying[0] - cw).abs() > 1e-9;
        let in_its_way: Vec<[f64; 4]> =
            shut.iter().filter(|lies| lies.1 & cx.side_of(c) != 0).map(|lies| lies.0).collect();
        let Some((at, turn)) = settled(ground, &[(lying, want)], &in_its_way)[0] else { continue };
        if dist(at, pad) > cx.board.tune.block_reach_mm {
            continue;
        }
        // (of the two ways it may lie there, the one with its own pad on the net toward the pad)
        let mut q = u8::from(turn != quarter_more);
        if let Some(mine) = parts[c].pads.iter().position(|&of| of == Some(net)) {
            if mine < parts[c].offsets.len() {
                let far = |q: u8| {
                    let from = quartered(q, cx.drawn_pad(c, mine));
                    dist([at[0] + from[0], at[1] + from[1]], pad)
                };
                if far(q + 2) < far(q) - 1e-9 {
                    q += 2;
                }
            }
            pins.push((c, (mine, by, pin)));
        }
        *at_pad.entry((by, pin)).or_insert(0) += 1;
        pos.insert(c, (at, q));
        shut.push((cx.drawn_rect(c, at, q), cx.side_of(c)));
    }
    let low = |k: usize| shut.iter().map(|lies| lies.0[k]).fold(f64::INFINITY, f64::min);
    let high = |k: usize| shut.iter().map(|lies| lies.0[k + 2]).fold(f64::NEG_INFINITY, f64::max);
    let mid = [(low(0) + high(0)) / 2.0, (low(1) + high(1)) / 2.0];
    Block {
        // (told as each part stands in the file: turned from that by what it is turned from its drawing,
        // less what it stands turned already)
        at: pos
            .iter()
            .map(|(&p, &(at, q))| (p, [at[0] - mid[0], at[1] - mid[1]], (q + 4 - cx.stands_turned(p)) % 4))
            .collect(),
        drawn: pos.iter().map(|(&p, &(at, q))| (p, at, q)).collect(),
        through: pos.keys().any(|&p| parts[p].through),
        pins,
    }
}

/// The blocks, by their chips.
///
/// A chip (or a connector) and the parts that stand by its pads are a block: laid out once, by themselves
/// on empty ground (the chip in the middle, each part beside the pad it is for, in the order they are
/// taken), and then placed as one body. Where each is taken after its neighbours, the ground by a chip's
/// pads is gone before its own parts come to it. Two blocks of the same make-up come out alike: the
/// steps that lay them out are the same.
pub(super) fn blocks(site: &Site, order: &[By], grounds: &BTreeMap<usize, [f64; 4]>) -> BTreeMap<usize, Block> {
    let (cx, parts, part_domain) = (site.cx, site.cx.parts, &site.part_domain);
    let mut block_head: BTreeMap<usize, usize> = BTreeMap::new();
    let mut members_of: BTreeMap<usize, Vec<(usize, usize, usize)>> = BTreeMap::new();
    for &(c, by, net) in order {
        let head = block_head.get(&by).copied().unwrap_or(by);
        let free = c != head && !block_head.contains_key(&c) && !members_of.contains_key(&c);
        let together = grounds.contains_key(&head)
            && grounds.contains_key(&c)
            && part_domain.get(&c) == part_domain.get(&head)
            // (a part in pieces is no head of a block: what stands by it may stand between its pieces)
            && parts[head].pieces.is_empty();
        if free && together && !parts[by].offsets.is_empty() && !parts[head].offsets.is_empty() {
            block_head.insert(c, head);
            members_of.entry(head).or_default().push((c, by, net));
        }
    }
    members_of
        .iter()
        .map(|(&head, list)| (head, lay_block(cx, head, list)))
        .filter(|(_, block)| block.at.len() > 1)
        .collect()
}

/// The block a part stands in, by its chip.
pub(super) fn in_blocks(p: usize, blocks: &BTreeMap<usize, Block>) -> Option<usize> {
    blocks.iter().find(|(_, block)| block.at.iter().any(|placed| placed.0 == p)).map(|(&head, _)| head)
}

/// The blocks of one make-up, and whether they are laid out alike. A make-up is the kind of the chip (its
/// footprint and its value) and, for each part by it, its kind and the pads it stands between; a layout is
/// where each such part lies from the chip and how it is turned.
pub(super) fn alike(cx: &Known, laid_blocks: &BTreeMap<usize, Block>) -> Vec<(usize, usize, usize, Vec<String>)> {
    type Made = BTreeMap<(String, Vec<String>), Vec<(usize, Vec<String>)>>; // make-up -> [(chip, layout)]
    let mut made: Made = BTreeMap::new();
    for (&head, block) in laid_blocks {
        let by_pad: BTreeMap<usize, (usize, usize, usize)> = block.pins.iter().copied().collect();
        let mut make: Vec<String> = Vec::new();
        let mut lies: Vec<String> = Vec::new();
        for &(m, at, q) in block.drawn.iter().filter(|placed| placed.0 != head) {
            let pads = by_pad.get(&m).map_or(String::new(), |&(mine, by, pin)| format!("{mine}>{}:{pin}", cx.kind(by)));
            make.push(format!("{} {pads}", cx.kind(m)));
            lies.push(format!("{} {pads} {:.2} {:.2} {q}", cx.kind(m), at[0] + 0.0, at[1] + 0.0));
        }
        make.sort();
        lies.sort();
        made.entry((cx.kind(head), make)).or_default().push((head, lies));
    }
    let mut alike: Vec<(usize, usize, usize, Vec<String>)> = made
        .values()
        .filter(|group| group.len() > 1)
        .map(|group| {
            let mut count: BTreeMap<&Vec<String>, usize> = BTreeMap::new();
            group.iter().for_each(|(_, lies)| *count.entry(lies).or_insert(0) += 1);
            let most = count.values().copied().max().unwrap_or(0);
            (group.len(), most, group[0].1.len() + 1, cx.names(&group.iter().map(|of| of.0).collect::<Vec<_>>()))
        })
        .collect();
    alike.sort_by(|a, b| (b.0 * b.2).cmp(&(a.0 * a.2)).then(a.3.cmp(&b.3)));
    alike
}
