#![allow(unused_imports)] // CooldownPrediction re-exported as public API despite bin target not using it directly.
pub mod config;
pub mod inhibit;
pub mod metrics;
pub mod prediction;
pub mod service;
