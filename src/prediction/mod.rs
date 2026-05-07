//! Predictive cooldown system for adaptive sleep inhibition.
#![allow(dead_code)] // Public API items exercised only by unit tests in non-test builds.

/// History log — binary format, date-partitioned files with pruning.
mod history;
mod ml_model;
mod model;

pub use history::{fill_gaps, EntryDeltas, HistoryEntry, HistoryLog};
pub use ml_model::MlPredictor;
pub use model::{CooldownPrediction, PredictionModel};
