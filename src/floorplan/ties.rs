//! What ties things to each other, and the groups that are tied more within than without.

use std::collections::{BTreeMap, BTreeSet};

/// Groups of nodes after one round of local moves: each group, as one, goes to the neighbouring group that
/// raises the modularity most, until none moves. `ties`: the weight between two nodes, the lower first. The
/// groups that end together are one, in the order of the first.
pub fn gathered(groups: &[Vec<usize>], ties: &BTreeMap<(usize, usize), f64>) -> Vec<Vec<usize>> {
    let mut index: BTreeMap<usize, usize> = BTreeMap::new();
    for (g, members) in groups.iter().enumerate() {
        for &node in members {
            index.insert(node, g);
        }
    }
    let mut between: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for (&(a, b), &weight) in ties {
        if let (Some(&i), Some(&j)) = (index.get(&a), index.get(&b)) {
            *between.entry((i.min(j), i.max(j))).or_insert(0.0) += weight;
        }
    }
    let home = moved(groups.len(), &between);
    let mut merged: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (g, &to) in home.iter().enumerate() {
        merged.entry(to).or_default().push(g);
    }
    // (in the order of the first group of each)
    let mut lots: Vec<Vec<usize>> = merged.into_values().collect();
    lots.sort_by_key(|lot| lot[0]);
    lots.into_iter().map(|lot| lot.into_iter().flat_map(|g| groups[g].iter().copied()).collect()).collect()
}

/// Nodes in groups that are tied more within than without, the largest first: rounds of [`gathered`], each on
/// the groups of the round before, until a round changes nothing. The order of the nodes decides between
/// equals, so the same question has the same answer.
pub fn communities(nodes: &[usize], ties: &BTreeMap<(usize, usize), f64>) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = nodes.iter().map(|&node| vec![node]).collect();
    loop {
        let found = gathered(&groups, ties);
        if found.len() == groups.len() {
            let place = |node: usize| nodes.iter().position(|&n| n == node).unwrap_or(usize::MAX);
            groups.sort_by_key(|members| (usize::MAX - members.len(), place(members[0])));
            return groups;
        }
        groups = found;
    }
}

/// Which group each of `count` nodes ends in after local moves; `between`: the weight between two of them, the
/// lower first (a node with itself: what it, a group of an earlier round, has within).
pub(super) fn moved(count: usize, between: &BTreeMap<(usize, usize), f64>) -> Vec<usize> {
    let mut beside: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); count];
    let mut degree = vec![0.0f64; count];
    for (&(i, j), &weight) in between {
        if i == j {
            degree[i] += 2.0 * weight;
        } else {
            *beside[i].entry(j).or_insert(0.0) += weight;
            *beside[j].entry(i).or_insert(0.0) += weight;
            degree[i] += weight;
            degree[j] += weight;
        }
    }
    let total: f64 = degree.iter().sum();
    let mut home: Vec<usize> = (0..count).collect();
    if total <= 0.0 {
        return home;
    }
    let mut weight_of = degree.clone();
    for _ in 0..100 {
        let mut changed = false;
        for node in 0..count {
            let here = home[node];
            let mut to: BTreeMap<usize, f64> = BTreeMap::new();
            for (&other, &weight) in &beside[node] {
                *to.entry(home[other]).or_insert(0.0) += weight;
            }
            weight_of[here] -= degree[node];
            let gain = |group: usize| to.get(&group).copied().unwrap_or(0.0) - weight_of[group] * degree[node] / total;
            let mut best = (here, f64::NEG_INFINITY);
            for &group in to.keys() {
                if gain(group) > best.1 {
                    best = (group, gain(group));
                }
            }
            let to_group = if best.1 > gain(here) + 1e-12 { best.0 } else { here };
            weight_of[to_group] += degree[node];
            if to_group != here {
                home[node] = to_group;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    home
}

/// Ties between things from nets (each the parts it reaches; `of`: the thing a part is of, if any): a net of k
/// things ties each two of them by 1 / (k - 1), so that it counts once for each thing it reaches.
pub(super) fn tied(nets: &[Vec<usize>], of: &dyn Fn(usize) -> Option<usize>) -> BTreeMap<(usize, usize), f64> {
    let mut ties = BTreeMap::new();
    for parts in nets {
        let things: BTreeSet<usize> = parts.iter().filter_map(|&part| of(part)).collect();
        let things: Vec<usize> = things.into_iter().collect();
        for (i, &a) in things.iter().enumerate() {
            for &b in &things[i + 1..] {
                *ties.entry((a, b)).or_insert(0.0) += 1.0 / (things.len() - 1) as f64;
            }
        }
    }
    ties
}

/// Things in the order their ties alone give: each goes half way to the middle of those it is tied to, over
/// and over, with the middle of all taken out, which leaves those tied to each other side by side.
pub(super) fn in_line(count: usize, ties: &BTreeMap<(usize, usize), f64>) -> Vec<f64> {
    let mut line: Vec<f64> = (0..count).map(|z| z as f64).collect();
    for _ in 0..500 {
        let before = line.clone();
        for z in 0..count {
            let (mut sum, mut weights) = (0.0, 0.0);
            for (&(a, b), &weight) in ties.iter().filter(|(&(a, b), _)| a == z || b == z) {
                sum += weight * before[if a == z { b } else { a }];
                weights += weight;
            }
            if weights > 0.0 {
                line[z] = (before[z] + sum / weights) / 2.0;
            }
        }
        let mean = line.iter().sum::<f64>() / count.max(1) as f64;
        let size = line.iter().map(|v| (v - mean).abs()).fold(0.0, f64::max).max(1e-12);
        line.iter_mut().for_each(|v| *v = (*v - mean) / size);
    }
    line
}
