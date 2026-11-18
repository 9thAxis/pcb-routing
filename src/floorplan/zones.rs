//! The zones of a board: the groups within each sheet, gathered level by level, under each domain.

use super::constants::{BESIDE_CHIP, GATHERED};
use super::ground::Work;
use super::known::Known;
use super::ties::{communities, gathered, tied};
use super::types::Apart;
use std::collections::{BTreeMap, BTreeSet};

/// The zones within a lot of parts: the groups within each sheet, gathered level by level. `between_parts`:
/// what ties two parts by the signal nets they share. A sheet whose groups are not gathered together at the
/// first level is told in `apart`.
fn within(
    cx: &Known,
    between_parts: &BTreeMap<(usize, usize), f64>,
    lot: &[usize],
    sheeted: f64,
    apart: &mut Vec<Apart>,
) -> Vec<Work> {
    let (parts, found, reach, nets, power_layers) = (cx.parts, &cx.found, &cx.reach, &cx.nets, &cx.power_layers);
    let mine: BTreeSet<usize> = lot.iter().copied().collect();
    let mut sheets: Vec<String> = mine.iter().map(|&p| cx.sheet(p)).collect::<BTreeSet<_>>().into_iter().collect();
    sheets.sort();
    let mut atoms: Vec<(String, Vec<usize>)> = Vec::new();
    let mut atom_of: BTreeMap<usize, usize> = BTreeMap::new();
    for of_sheet in &sheets {
        let mut own: Vec<usize> = mine.iter().copied().filter(|&p| &cx.sheet(p) == of_sheet).collect();
        own.sort_by_key(|&p| cx.name(p));
        let here: BTreeSet<usize> = own.iter().copied().collect();
        let mut ties: BTreeMap<(usize, usize), f64> = between_parts
            .iter()
            .filter(|(&(a, b), _)| here.contains(&a) && here.contains(&b))
            .map(|(&k, &w)| (k, w))
            .collect();
        // (a capacitor from a supply to a ground is tied to a chip of the sheet on that supply, the chips
        // taking them in turn: it is there for the chip, though no signal says so)
        let mut has: BTreeMap<(usize, usize), usize> = BTreeMap::new();
        for &c in own.iter().filter(|&&c| parts[c].capacitor && parts[c].pads.len() == 2) {
            for &rail in parts[c].pads.iter().flatten().filter(|net| found.rails.contains_key(net)) {
                let chips = own.iter().filter(|&&q| cx.is_chip(q) && parts[q].pads.contains(&Some(rail)));
                if let Some(&chip) = chips.min_by_key(|&&q| (has.get(&(rail, q)).copied().unwrap_or(0), q)) {
                    *has.entry((rail, chip)).or_insert(0) += 1;
                    *ties.entry((c.min(chip), c.max(chip))).or_insert(0.0) += BESIDE_CHIP;
                }
            }
        }
        let is_tied: BTreeSet<usize> = ties.keys().flat_map(|&(a, b)| [a, b]).collect();
        let nodes: Vec<usize> = own.iter().copied().filter(|p| is_tied.contains(p)).collect();
        let mut groups = communities(&nodes, &ties);
        if groups.is_empty() {
            groups.push(Vec::new());
        }
        groups[0].extend(own.iter().copied().filter(|p| !is_tied.contains(p)));
        for mut group in groups {
            group.sort_by_key(|&p| cx.name(p));
            for &p in &group {
                atom_of.insert(p, atoms.len());
            }
            atoms.push((of_sheet.clone(), group));
        }
    }
    // (a zone by the sheet most of its parts are drawn on; and by its first parts, if it is not all of it)
    let named = |members: &[usize]| -> String {
        let mut count: BTreeMap<&str, usize> = BTreeMap::new();
        for &atom in members {
            *count.entry(atoms[atom].0.as_str()).or_insert(0) += atoms[atom].1.len();
        }
        let mut most: (&str, usize) = ("", 0);
        for (&of_sheet, &n) in &count {
            if n > most.1 {
                most = (of_sheet, n);
            }
        }
        let mut of_it: Vec<String> = members
            .iter()
            .filter(|&&atom| atoms[atom].0 == most.0)
            .flat_map(|&atom| cx.names(&atoms[atom].1))
            .collect();
        of_it.sort();
        let some = if most.1 == cx.sheet_size(most.0) {
            String::new()
        } else {
            format!(" ({}{})", of_it[..of_it.len().min(3)].join(", "), if of_it.len() > 3 { " ..." } else { "" })
        };
        let more = count.len() - 1;
        format!("{}{}{}", most.0, some, if more > 0 { format!(" and {more} more") } else { String::new() })
    };
    let local: Vec<Vec<usize>> =
        nets.iter().map(|net| net.iter().copied().filter(|p| mine.contains(p)).collect()).collect();
    let mut ties = tied(&local, &|p| atom_of.get(&p).copied());
    // (a capacitor from a supply to a ground with no chip of its own sheet on that supply is for a chip
    // of another: its group is tied to that chip's, the chips taking them in turn)
    let mut has: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for &c in mine.iter().filter(|&&c| parts[c].capacitor && parts[c].pads.len() == 2) {
        for &rail in parts[c].pads.iter().flatten().filter(|net| found.rails.contains_key(net)) {
            let chips = || mine.iter().filter(|&&q| cx.is_chip(q) && parts[q].pads.contains(&Some(rail)));
            if chips().any(|&q| cx.sheet(q) == cx.sheet(c)) {
                continue;
            }
            if let Some(&chip) = chips().min_by_key(|&&q| (has.get(&(rail, q)).copied().unwrap_or(0), q)) {
                *has.entry((rail, chip)).or_insert(0) += 1;
                if let (Some(&a), Some(&b)) = (atom_of.get(&c), atom_of.get(&chip)) {
                    if a != b {
                        *ties.entry((a.min(b), a.max(b))).or_insert(0.0) += 1.0;
                    }
                }
            }
        }
    }
    // (and by the supply each would have of the plane: the groups most of whose parts are on one supply
    // are tied by it, GATHERED nets for each, that what is fed alike be gathered together)
    let mut asking: Vec<Vec<(usize, usize, usize)>> = vec![Vec::new(); atoms.len()];
    for &net in found.rails.keys() {
        let mut count: BTreeMap<usize, usize> = BTreeMap::new();
        reach[net].iter().filter_map(|p| atom_of.get(p)).for_each(|&a| *count.entry(a).or_insert(0) += 1);
        if count.len() > 1 {
            count.iter().for_each(|(&a, &n)| asking[a].push((n, count.len(), net)));
        }
    }
    let mut fed_by: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (a, asks) in asking.iter_mut().enumerate() {
        asks.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
        asks.iter().take(power_layers.len()).for_each(|ask| fed_by.entry(ask.2).or_default().push(a));
    }
    for alike in fed_by.values().filter(|alike| alike.len() > 1) {
        for (i, &a) in alike.iter().enumerate() {
            for &b in &alike[i + 1..] {
                *ties.entry((a.min(b), a.max(b))).or_insert(0.0) += GATHERED / (alike.len() - 1) as f64;
            }
        }
    }
    // (and by the sheet they are drawn on: the groups of one sheet are tied to each other, `sheeted` nets for
    // each. Its designer drew them as one thing; the few nets that lead from one stage of it to the next say
    // less of that than the supplies and the control lines its stages share with the rest of the board)
    let mut drawn: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (a, atom) in atoms.iter().enumerate().filter(|(_, atom)| !atom.1.is_empty() && !atom.0.is_empty()) {
        drawn.entry(atom.0.as_str()).or_default().push(a);
    }
    for together in drawn.values().filter(|together| together.len() > 1 && sheeted > 0.0) {
        for (i, &a) in together.iter().enumerate() {
            for &b in &together[i + 1..] {
                *ties.entry((a.min(b), a.max(b))).or_insert(0.0) += sheeted / (together.len() - 1) as f64;
            }
        }
    }
    let ties = ties;
    let mut levels: Vec<Vec<Vec<usize>>> =
        vec![(0..atoms.len()).filter(|&a| !atoms[a].1.is_empty()).map(|a| vec![a]).collect()];
    loop {
        let last = &levels[levels.len() - 1];
        let next = gathered(last, &ties);
        if next.len() == last.len() || next.len() < 2 {
            break;
        }
        levels.push(next);
    }
    let parts_of =
        |members: &[usize]| -> Vec<usize> { members.iter().flat_map(|&atom| atoms[atom].1.iter().copied()).collect() };
    let blank = [0.0; 4];
    let mut kept: Vec<(Vec<usize>, Work)> = levels[0]
        .iter()
        .map(|members| {
            (
                members.clone(),
                Work { name: named(members), parts: parts_of(members), inside: Vec::new(), rect: blank, more: 0.0 },
            )
        })
        .collect();
    for level in &levels[1..] {
        let mut made: Vec<(Vec<usize>, Work)> = Vec::new();
        for members in level {
            let of: BTreeSet<usize> = members.iter().copied().collect();
            let (mut inside, rest): (Vec<_>, Vec<_>) =
                kept.into_iter().partition(|(under, _)| under.iter().all(|a| of.contains(a)));
            kept = rest;
            if inside.len() == 1 {
                made.push((members.clone(), inside.remove(0).1));
            } else {
                let mut inner: Vec<Work> = inside.into_iter().map(|(_, work)| work).collect();
                inner.sort_by(|a, b| b.parts.len().cmp(&a.parts.len()).then(a.name.cmp(&b.name)));
                made.push((
                    members.clone(),
                    Work { name: named(members), parts: parts_of(members), inside: inner, rect: blank, more: 0.0 },
                ));
            }
        }
        kept = made;
    }
    // A sheet whose groups are not gathered together at the first level: drawn as one, tied apart.
    let first = if levels.len() > 1 { &levels[1] } else { &levels[0] };
    let mut first_of: BTreeMap<usize, usize> = BTreeMap::new();
    for (g, members) in first.iter().enumerate() {
        for &atom in members {
            first_of.insert(atom, g);
        }
    }
    for of_sheet in &sheets {
        let own: Vec<usize> = (0..atoms.len()).filter(|&a| &atoms[a].0 == of_sheet && !atoms[a].1.is_empty()).collect();
        let into: BTreeSet<usize> = own.iter().map(|a| first_of[a]).collect();
        if into.len() > 1 {
            apart.push((
                of_sheet.clone(),
                own.iter().map(|&a| (cx.names(&atoms[a].1), named(&first[first_of[&a]]))).collect(),
            ));
        }
    }
    let mut zones: Vec<Work> = kept.into_iter().map(|(_, work)| work).collect();
    zones.sort_by(|a, b| b.parts.len().cmp(&a.parts.len()).then(a.name.cmp(&b.name)));
    zones
}

/// The zones of the board, a domain at the top of each tree (and one for the parts on no ground), with the
/// sheets that are drawn as one and tied apart.
pub(super) fn zone_tree(cx: &Known, sheeted: f64) -> (Vec<Work>, Vec<Apart>) {
    let (parts, found, domains, adrift) = (cx.parts, &cx.found, &cx.domains, &cx.adrift);
    let between_parts = tied(&cx.nets, &Some);
    let mut apart: Vec<Apart> = Vec::new();
    let mut top: Vec<Work> = Vec::new();
    // (a part that is held where it stands is of no zone: the zones are made of what may be moved, and are
    // pulled to it by what joins them to it)
    let free = |lot: &Vec<usize>| -> Vec<usize> { lot.iter().copied().filter(|&p| !cx.held(p)).collect() };
    for (g, own) in domains {
        let own = &free(own);
        let inside = within(cx, &between_parts, own, sheeted, &mut apart);
        // (a part counted with another domain that stands on this ground too takes half its ground here)
        let half = |p: usize| {
            if found.home[p] != Some(*g) && found.stands[p].contains(g) && !cx.held(p) {
                parts[p].area / 2.0
            } else {
                0.0
            }
        };
        let more = (0..parts.len()).map(half).sum();
        top.push(Work { name: cx.net_name(*g), parts: own.clone(), inside, rect: [0.0; 4], more });
    }
    if !adrift.is_empty() {
        let adrift = &free(adrift);
        let inside = within(cx, &between_parts, adrift, sheeted, &mut apart);
        top.push(Work { name: "(on no ground)".to_string(), parts: adrift.clone(), inside, rect: [0.0; 4], more: 0.0 });
    }
    (top, apart)
}
