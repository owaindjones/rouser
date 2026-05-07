//! Predictive cooldown system for adaptive sleep inhibition.
#![allow(dead_code)] // Public API items exercised only by unit tests in non-test builds.

/// History log — binary format, date-partitioned files with pruning.
mod history;
mod model;

pub use history::{fill_gaps, EntryDeltas, HistoryEntry, HistoryLog};
pub use model::{CooldownPrediction, PredictionModel};
