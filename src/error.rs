//! Why a problem was not run.

/// Why a problem was not run. A problem that was run and could not be completed is not an error: it comes back
/// as a [`Solution`](crate::Solution) whose status says so.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// The problem does not make sense as given.
    Invalid {
        /// What is wrong, as a path into the problem (`nets[3].width`, `options.bend_mm`).
        field: String,
        /// Why.
        reason: String,
    },
    /// The grid is larger than the run may be.
    TooLarge {
        /// (layer, cell) states of the grid.
        states: usize,
        /// The most accepted: [`Options::max_states`](crate::Options::max_states), or what the search's
        /// 32-bit state numbers can hold.
        limit: usize,
    },
    /// The threads could not be started.
    Threads(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Invalid { field, reason } => write!(f, "invalid problem: {field}: {reason}"),
            Error::TooLarge { states, limit } => write!(f, "the grid has {states} (layer, cell) states, more than the {limit} allowed (a coarser step, a smaller window, or a higher options.max_states)"),
            Error::Threads(e) => write!(f, "cannot start threads: {e}"),
        }
    }
}

impl std::error::Error for Error {}

pub(crate) fn invalid(field: impl Into<String>, reason: impl Into<String>) -> Error {
    Error::Invalid { field: field.into(), reason: reason.into() }
}
