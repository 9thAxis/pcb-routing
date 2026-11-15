//! Running a problem: the attempts side by side, and the choice among them.

use crate::engine::shared::{Diagnostics, Shared};
use crate::engine::Engine;
use crate::error::{invalid, Error};
use crate::model::*;

/// Something that happened during a run, for [`Router::on_event`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Event {
    /// A round of negotiation has ended.
    Round {
        /// The attempt it belongs to.
        attempt: usize,
        /// The round, from 1.
        round: usize,
        /// Nets laid again in this round.
        relaid: usize,
        /// Nets still too close to another after it.
        in_conflict: usize,
        /// The cost factor per other net claiming a cell in this round.
        present: f64,
    },
    /// With [`Router::verify_search`]: a search whose two-pass cost differs from its one-pass cost by more than
    /// a quarter of a grid step. Costs are in grid steps; `None` means no path was found.
    Mismatch {
        /// Key of the net being laid.
        net: String,
        /// What the two-pass search found.
        two_pass: Option<f64>,
        /// What the one-pass search found.
        one_pass: Option<f64>,
    },
}

/// A run of one problem, with what is to be watched or checked on the way.
///
/// ```
/// # use pcb_routing::{Problem, Router};
/// # let problem: Problem = serde_json::from_str(r#"{"window": [0, 0, 4, 2], "step": 0.1, "layers": ["F"],
/// #   "nets": [{"key": "a", "net": "A", "width": 0.15, "via": [0.45, 0.2], "layers": ["F"], "clearance": {"default": 0.17},
/// #             "terminals": [{"xy": [1, 1], "layers": ["F"]}, {"xy": [3, 1], "layers": ["F"]}]}]}"#).unwrap();
/// let solution = Router::new(&problem)
///     .on_event(|event| eprintln!("{event:?}"))
///     .run()?;
/// # Ok::<(), pcb_routing::Error>(())
/// ```
pub struct Router<'a> {
    problem: &'a Problem,
    diagnostics: Diagnostics,
    only: Option<usize>,
}

impl<'a> Router<'a> {
    /// A run of `problem` as its own options say.
    pub fn new(problem: &'a Problem) -> Router<'a> {
        Router { problem, diagnostics: Diagnostics::default(), only: None }
    }

    /// Run every two-pass search a second time in one pass and compare the costs. Differences are counted in
    /// [`Stats::mismatches`] and reported as [`Event::Mismatch`]. Several times slower; for testing the router.
    pub fn verify_search(mut self, on: bool) -> Self {
        self.diagnostics.verify = on;
        self
    }

    /// Be told of each round as it ends. The function is called from the threads the attempts run on.
    pub fn on_event(mut self, f: impl Fn(&Event) + Send + Sync + 'static) -> Self {
        self.diagnostics.on_event = Some(Box::new(f));
        self
    }

    /// Run one attempt of [`Options::variants`] alone (to look at one attempt of a larger run).
    pub fn only_attempt(mut self, variant: usize) -> Self {
        self.only = Some(variant);
        self
    }

    /// Route the problem: one attempt per variant (the first in the given order, the others shuffled), side by
    /// side; the best complete one is returned, with every attempt listed in brief.
    ///
    /// An `Err` means the problem was not run. A problem that was run and could not be completed comes back
    /// as `Ok` with [`Status::Conflicts`] or [`Status::Unroutable`].
    pub fn run(self) -> Result<Solution, Error> {
        let p = self.problem;
        p.validate()?;
        let variants = p.options.variants.max(1);
        if let Some(v) = self.only {
            if v >= variants {
                return Err(invalid("only_attempt", format!("attempt {v} of {variants}")));
            }
        }
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(p.options.threads)
            .build()
            .map_err(|e| Error::Threads(e.to_string()))?;
        let only = self.only;
        let diagnostics = self.diagnostics;
        Ok(pool.install(move || attempts(p, variants, only, diagnostics)))
    }
}

/// Route a problem with nothing watched: `Router::new(problem).run()`.
pub fn route(problem: &Problem) -> Result<Solution, Error> {
    Router::new(problem).run()
}

/// The order in which attempt `v` lays the nets: as given for the first, shuffled (always the same way) for the others.
fn order(nets: usize, v: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..nets).collect();
    let mut seed = 0x9E3779B97F4A7C15u64.wrapping_mul(v as u64 + 1);
    if v > 0 {
        for k in (1..order.len()).rev() {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            order.swap(k, ((seed >> 33) as usize) % (k + 1));
        }
    }
    order
}

/// How good an ending is, lower being better: all laid and clear, then all laid but some too close, then some
/// not laid.
fn rank(status: Status) -> u8 {
    match status {
        Status::Routed => 0,
        Status::Conflicts => 1,
        Status::Unroutable => 2,
    }
}

/// Whether attempt `a` ended better than `b`: by how it ended, then the fewer connections not made and places
/// too close, then the lower measure (see `Engine::report`).
fn better(a: &(Solution, f64), b: &(Solution, f64)) -> bool {
    let left = |s: &Solution| (rank(s.status), s.unrouted.len() + s.conflicts.len());
    match left(&a.0).cmp(&left(&b.0)) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => a.1 < b.1,
    }
}

fn attempts(p: &Problem, variants: usize, only: Option<usize>, diagnostics: Diagnostics) -> Solution {
    use rayon::prelude::*;
    let shared = Shared::new(p, diagnostics);
    let run = |v: usize| Engine::new(p, &shared).attempt(&order(p.nets.len(), v), v);
    let ended: Vec<(Solution, f64)> = match only {
        Some(v) => vec![run(v)],
        None if variants == 1 => vec![run(0)],
        None => (0..variants).into_par_iter().map(run).collect(),
    };
    // of equals the earlier attempt
    let mut best = 0;
    for k in 1..ended.len() {
        if better(&ended[k], &ended[best]) {
            best = k;
        }
    }
    let results: Vec<Solution> = ended.into_iter().map(|(solution, _)| solution).collect();
    let brief: Vec<Attempt> = results
        .iter()
        .map(|s| Attempt {
            variant: s.variant,
            status: s.status,
            score: s.score,
            rounds: s.iterations,
            lengths_mm: s.nets.iter().map(|n| n.length_mm).collect(),
            vias: s.nets.iter().map(|n| n.vias.len()).sum(),
        })
        .collect();
    let mut chosen = results.into_iter().nth(best).unwrap();
    chosen.attempts = brief;
    chosen.stats = shared.stats();
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_better_ending_ranks_lower() {
        assert!(rank(Status::Routed) < rank(Status::Conflicts));
        assert!(rank(Status::Conflicts) < rank(Status::Unroutable));
    }

    #[test]
    fn attempts_that_end_alike_are_ranked_by_what_is_missing_and_then_by_the_measure() {
        let ended = |status: Status, missing: usize, measure: f64| {
            let mut s: Solution = serde_json::from_value(serde_json::json!({
                "status": "routed", "iterations": 1, "variant": 0, "score": null, "nets": [], "conflicts": [], "message": ""
            }))
            .unwrap();
            s.status = status;
            s.unrouted = vec![String::new(); missing];
            (s, measure)
        };
        // complete beats incomplete whatever the measure
        assert!(better(&ended(Status::Routed, 0, 900.0), &ended(Status::Unroutable, 1, 10.0)));
        // of two incomplete ones, the one with less missing; then the lower measure, though neither has a score
        assert!(better(&ended(Status::Unroutable, 1, 900.0), &ended(Status::Unroutable, 2, 10.0)));
        assert!(better(&ended(Status::Unroutable, 2, 10.0), &ended(Status::Unroutable, 2, 11.0)));
        assert!(!better(&ended(Status::Unroutable, 2, 10.0), &ended(Status::Unroutable, 2, 10.0)));
    }

    #[test]
    fn the_first_attempt_keeps_the_given_order_and_the_others_are_shuffled_the_same_way_each_time() {
        assert_eq!(order(5, 0), vec![0, 1, 2, 3, 4]);
        for v in 1..4 {
            let mut shuffled = order(5, v);
            assert_eq!(shuffled, order(5, v));
            shuffled.sort();
            assert_eq!(shuffled, vec![0, 1, 2, 3, 4]);
        }
    }
}
