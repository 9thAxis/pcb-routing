//! The small parts gone over to shorten their nets.

use super::beside::Beside;
use super::facing::Facing;
use super::pads::ByPads;
use super::{Site, Standing};
use crate::floorplan::constants::{FEW, OWN_PAD_MM, REFINES, REFINE_MM, SETTLE};
use crate::floorplan::settle::{median, settled};
use crate::floorplan::types::Pin;
use crate::geom::dist;
use crate::model::Pt;
use std::collections::BTreeMap;

/// The small parts are gone over, a few times: each is tried at the middle of what its nets join it to (as
/// near that as there is room, within REFINE_MM of it), and stays there if its nets are the shorter for it
/// by the boxes of their parts. Not what was settled first, nor a capacitor by its pad.
pub(super) fn refined(
    site: &Site,
    facing: &Facing,
    beside: &Beside,
    by: &ByPads,
    gap_rects: &BTreeMap<usize, [f64; 4]>,
    halos: &[[f64; 4]],
    standing: &mut Standing,
) {
    let (cx, top, roads, on_gap) = (site.cx, site.top, site.roads, &site.on_gap);
    let (parts, victim, shift_of, domain_of_part) = (cx.parts, &cx.victim, &site.shift_of, &site.domain_of_part);
    let (fine_nets, pads_on) = (&beside.fine_nets, &facing.pads_on);
    let (by_its_pad, with_capacitors) = (&by.by_its_pad, &by.with_capacitors);
    let Standing { stood, centre, quarter } = standing;
    let mut nets_of: Vec<Vec<usize>> = vec![Vec::new(); parts.len()];
    for (n, net) in fine_nets.iter().enumerate() {
        net.iter().for_each(|&p| nets_of[p].push(n));
    }
    let boxes = |p: usize, at: Pt, centre: &[Option<Pt>]| -> f64 {
        let mut all = 0.0;
        for &n in &nets_of[p] {
            let (mut low, mut high) = (at, at);
            for there in fine_nets[n].iter().filter(|&&q| q != p).filter_map(|&q| facing.in_stack(q, centre)) {
                low = [low[0].min(there[0]), low[1].min(there[1])];
                high = [high[0].max(there[0]), high[1].max(there[1])];
            }
            all += high[0] - low[0] + high[1] - low[1];
        }
        all
    };
    for _ in 0..REFINES {
        let mut better = false;
        for p in 0..parts.len() {
            let settled_here = !on_gap.contains_key(&p) && stood.get(&p).is_some_and(|at| at.is_some());
            let free = settled_here && !cx.goes_first(p) && !by_its_pad.contains(&p) && !with_capacitors.contains(&p);
            let Some(was) = centre[p].filter(|_| free) else { continue };
            let shift = shift_of[p];
            let mates: Vec<Pt> = nets_of[p]
                .iter()
                .flat_map(|&n| fine_nets[n].iter().filter(|&&q| q != p))
                .filter_map(|&q| facing.in_stack(q, centre))
                .map(|at| [at[0] - shift[0], at[1] - shift[1]])
                .collect();
            // (a part of two pads with one of them on a net of its own with one pad of another part is there
            // for that pad, a resistor at its driver, a protector at its connector: it wants to be by it, and
            // goes there if its nets are no longer for it)
            let own_pad: Option<Pt> = if parts[p].pads.len() == 2 && !parts[p].capacitor {
                let alone: Vec<(usize, usize)> = parts[p]
                    .pads
                    .iter()
                    .flatten()
                    .filter(|&&net| pads_on[net].len() == 2)
                    .filter_map(|&net| pads_on[net].iter().copied().find(|other| other.0 != p))
                    .collect();
                // (with a net of its own to a pad at each end, it is for the one that drives, if the schematic
                // says which that is)
                let drives =
                    |&(q, j): &(usize, usize)| matches!(parts[q].pins.get(j), Some(Pin::Output) | Some(Pin::PowerOut));
                let chosen = match alone[..] {
                    [one] => Some(one),
                    [a, b] if drives(&a) != drives(&b) => Some(if drives(&a) { a } else { b }),
                    _ => None,
                };
                chosen
                    .and_then(|(q, j)| facing.pad_at(q, j, quarter[q], centre))
                    .map(|at| [at[0] - shift[0], at[1] - shift[1]])
            } else {
                None
            };
            // Where it may want to be: by the pad it is for, if it is for one; else at the middle of what it is
            // joined to, and (a part joined to few, which is as well anywhere between them) at their mean and
            // by each of them. Room is looked for near each, then further (twice and four times as far); the
            // place that leaves its nets the shortest is the one tried.
            let mut wants: Vec<Pt> = Vec::new();
            match own_pad {
                Some(pad) => wants.push(pad),
                None => {
                    wants.extend(median(&mates));
                    if !mates.is_empty() && mates.len() <= FEW {
                        let n = mates.len() as f64;
                        wants.push([
                            mates.iter().map(|m| m[0]).sum::<f64>() / n,
                            mates.iter().map(|m| m[1]).sum::<f64>() / n,
                        ]);
                        wants.extend(mates.iter().copied());
                    }
                }
            }
            if wants.is_empty() || dist(wants[0], was) < SETTLE {
                continue;
            }
            let domain = domain_of_part[p];
            let ground = top[domain].rect;
            let mut shut: Vec<[f64; 4]> = site.closed(gap_rects);
            shut.extend(roads.iter().filter(|road| road.gap).map(|road| road.rect));
            let in_the_way = |q: &&usize| **q != p && !on_gap.contains_key(q) && cx.side_of(**q) & cx.side_of(p) != 0;
            // (whatever domain it is of: a part that found no room on its own ground may stand on this one)
            for q in (0..parts.len()).filter(|q| in_the_way(&q)) {
                if let Some(at) = centre[q].filter(|_| stood.get(&q).is_some_and(|at| at.is_some())) {
                    shut.push(cx.taken(q, (at, quarter[q] % 2 == 1)));
                }
            }
            // (what is disturbed is not moved onto the ground near what disturbs)
            if victim[p] {
                shut.extend(halos.iter().copied());
            }
            let [w, h] = cx.body(p);
            let lying = if quarter[p] % 2 == 1 { [h, w] } else { [w, h] };
            let stack = |at: Pt| [at[0] + shift[0], at[1] + shift[1]];
            let now = boxes(p, stack(was), centre);
            let mut tried: Option<(f64, Pt, bool)> = None;
            for want in wants {
                let window = |reach: f64| {
                    [
                        ground[0].max(want[0] - reach),
                        ground[1].max(want[1] - reach),
                        ground[2].min(want[0] + reach),
                        ground[3].min(want[1] + reach),
                    ]
                };
                let room = [1.0, 2.0, 4.0].iter().find_map(|&times| {
                    let within = window(times * REFINE_MM);
                    let has_size = within[2] - within[0] >= SETTLE && within[3] - within[1] >= SETTLE;
                    if has_size {
                        settled(within, &[(lying, want)], &shut)[0]
                    } else {
                        None
                    }
                });
                if let Some((at, turn)) = room {
                    let then = boxes(p, stack(at), centre);
                    if tried.map_or(true, |best| then < best.0 - 1e-6) {
                        tried = Some((then, at, turn));
                    }
                }
            }
            let Some((then, at, turn)) = tried else { continue };
            let nearer = own_pad.is_some_and(|pad| dist(at, pad) < dist(was, pad) - SETTLE && then <= now + OWN_PAD_MM);
            if then < now - 1e-6 || nearer {
                let turn = turn != (quarter[p] % 2 == 1);
                (centre[p], quarter[p]) = (Some(at), u8::from(turn));
                stood.insert(p, Some((at, turn)));
                better = true;
            }
        }
        if !better {
            break;
        }
    }
}
