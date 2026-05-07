# Prediction Model Refactoring — Task Tracker

This file tracks all tasks needed to replace the histogram-based prediction model with an unsupervised ML approach using NG-RC reservoir computing from the [irithyll](https://crates.io/crates/irithyll) crate.

## Completed Tasks

| # | Status | Description |
|---|--------|-------------|
| 1 | ✅ | Added GPU per-GPU-max and total-average deltas to `EntryDeltas` struct |
| 2 | ✅ | Updated `TrendSignal::compute()` to include GPU trends alongside CPU/network/disk |
| 3 | ✅ | Updated trend multiplier in `predict_cooldown()` to use GPU delta contribution |
| 4 | ✅ | Rewrote `docs/prediction-model.md` with ML architecture and all user corrections |

## Remaining Tasks — In Priority Order

### Phase 1: Foundation (Must complete before any model work)

| # | Task | Details | Files | Dependencies |
|---|------|---------|-------|-------------|
| 5 | Add `irithyll` crate to Cargo.toml | Version `9.9.x`, feature flags: `serde_support`. Justify as lightweight streaming ML with NG-RC reservoir computing for temporal pattern learning, zero unsafe blocks, O(1) per-sample memory | `Cargo.toml` | — |
| 6 | Add ML config options to `PredictionConfig` | New fields: `hidden_dim: usize (default 16)`, `delay_buffer_size: usize (default 8)`. Keep existing `update_interval`, `history_length`, `max_extension_time`. Update `Default` impl. Update `config/rouser.toml` with new defaults. Sync all three locations per AGENTS.md rules | `src/config.rs`, `config/rouser.toml`, `docs/configuration.md` | — |
| 7 | Create `src/prediction/ml_model.rs` | New module for ML predictor wrapper: `MlPredictor` struct wrapping irithyll's NG-RC. Methods: `new(config)`, `train(features, target)`, `predict(features) -> f64`, `save(path)`, `load(path)` | `src/prediction/ml_model.rs` (new), `src/prediction/mod.rs` (add module) | Task 5, 6 |

### Phase 2: Feature Pipeline

| # | Task | Details | Files | Dependencies |
|---|------|---------|-------|-------------|
| 8 | Create `FeatureVector` struct | Fixed-size array of 6 normalized f64 values (cpu_max, cpu_avg, gpu_max, gpu_avg, network, disk). Implement conversion from `HistoryEntry`. Include normalization statistics tracking (running mean/std) for consistent scaling across time | `src/prediction/ml_model.rs` | Task 7 |
| 9 | Replace TimeKey histogram with feature pipeline in `PredictionModel` | Remove `inhibited_timekeys: HashMap<TimeKey, u64>`. Add `ml_predictor: MlPredictor`, `normalization_stats: NormalizationStats { mean[6], std[6] }`. Update `new()` to load history and initialize stats. Update `record()` to build feature vectors | `src/prediction/model.rs` | Task 7, 8 |

### Phase 3: Model Integration

| # | Task | Details | Files | Dependencies |
|---|------|---------|-------|-------------|
| 10 | Implement unsupervised training loop in `predict_cooldown()` | When called (at each prediction update_interval), iterate recent entries, build feature vectors, train model incrementally. Use reconstruction error as anomaly score instead of histogram inhibition rate | `src/prediction/model.rs` | Task 9 |
| 11 | Replace `score_inhibition_rate()` with ML scoring | Remove TimeKey-based lookup and fallback matching. New method: `ml_predictor.score(&features) -> f64` returning normalized anomaly score (0–1). Map to cooldown extension via same interpolation logic as before | `src/prediction/model.rs` | Task 9, 10 |
| 12 | Remove TimeKey struct and all histogram-related code | Delete `TimeKey::from_timestamp_ns()`, `TimeKey::display()`, `TimeKey::hour_of_day()`, `score_from_count()`, linear day computation. Update debug logging to remove "time=year=X week=Y sec=Z" from output | `src/prediction/model.rs` | Task 10, 11 |
| 13 | Fix gap-filled entry handling | Remove filter-out of zero-value entries before feature vector construction (user: '"All metrics at 0 with no inhibition" is a valid state'). Keep them in history for baseline learning. Only exclude from training if they represent extended shutdown periods (>24h) | `src/prediction/model.rs` | Task 10 |

### Phase 4: TimeKey Simplification (Optional — only if partial time info useful)

| # | Task | Details | Files | Dependencies |
|---|------|---------|-------|-------------|
| 14 | Evaluate if `week_of_year + minutes_into_week` should be added as features | User suggested `(week_of_year, minutes_into_week)` for efficiency. In ML context this could be two additional features (week: 0–52, minutes: 0–10079) to encode temporal position without bucketing. Decide based on model performance experiments | `src/prediction/ml_model.rs` | Task 8, 10 |

### Phase 5: Testing and Verification

| # | Task | Details | Files | Dependencies |
|---|------|---------|-------|-------------|
| 15 | Add unit tests for `FeatureVector::from_entry()` | Test normalization with known values. Edge cases: all-zero entries, single-GPU systems, no GPUs (all zero) | `src/prediction/ml_model.rs` | Task 8 |
| 16 | Update existing prediction model tests | All tests in `model.rs #[cfg(test)] mod tests` need updating to work with ML pipeline instead of histogram. Test training → scoring → extension flow end-to-end | `src/prediction/model.rs` (tests) | Task 10, 11 |
| 17 | Add integration test for full prediction cycle | Spin up PredictionModel, feed synthetic history entries at known intervals, verify that anomalous patterns produce expected extensions | New file or existing tests | All previous tasks |

### Phase 6: Documentation and CI

| # | Task | Details | Files | Dependencies |
|---|------|---------|-------|-------------|
| 18 | Update AGENTS.md with new architecture section | Document ML-based prediction, TimeKey deprecation, irithyll dependency policy. Add "Prediction Model Refactoring" to Lessons Learned if relevant patterns emerge | `AGENTS.md` | All code tasks complete |
| 19 | Run full CI: build + clippy + test on final branch | Verify all changes pass before merging | — | All previous tasks |

## Architecture Decision Record

### Why NG-RC Reservoir Computing (irithyll)?

**Requirements:**
- Unsupervised learning (no labeled "inhibited" data for training)
- Online/iterative weight updates at each 30s prediction interval
- Small memory footprint (<1MB total model state)
- No external binary dependencies, pure Rust preferred
- Temporal awareness (learn patterns over time series)

**Alternatives considered:**
| Approach | Pros | Cons for this use case |
|----------|------|------------------------|
| NG-RC (irithyll) | Streaming O(1) memory per sample, temporal via delay buffers, concept drift adaptation, pure Rust zero unsafe | Requires one new crate dep |
| Isolation Forest (`extended-isolation-forest`) | Simple anomaly scoring, no training needed | Batch-only, no online updates, must reload on every prediction |
| Random Cut Forest (`anomstream`) | Streaming anomaly detection, low memory | No temporal awareness, less suited for time-series patterns |
| Autoencoder (xneuron) | Unsupervised reconstruction error as score | Fixed-point arithmetic only, minimal feature set, no online learning yet |
| LightRiver | Fast online ML, TinyML optimized | Primarily focused on anomaly detection algorithms (Hoeffding Trees), not neural networks for regression |

**Decision**: NG-RC from irithyll provides the best combination of temporal awareness, streaming updates, small memory footprint, and pure-Rust implementation with zero unsafe blocks.

### TimeKey Deprecation Rationale

The current `TimeKey` struct `(year, week_of_year, seconds_into_week)` has fundamental issues:
1. **Year is monotonically increasing** — it provides no pattern-matching value, only timestamp reconstruction capability
2. **604800 buckets/week is wasteful** — most buckets have zero or one entries even after years of data
3. **Exact-match fallback is brittle** — sparse data means frequent misses requiring hour-of-day fallback which loses precision

The ML approach eliminates bucketing entirely: each history entry becomes a feature vector, and the model learns temporal patterns through delay embeddings in the reservoir computing architecture. This removes all histogram-related complexity while improving generalization across time periods.

## Estimated Effort

| Phase | Tasks | Est. Complexity |
|-------|-------|-----------------|
| 1: Foundation | #5–7 | Low — setup and config |
| 2: Feature Pipeline | #8–9 | Medium — new data structures |
| 3: Model Integration | #10–13 | High — core logic rewrite |
| 4: TimeKey Simplification | #14 | Low — optional feature addition |
| 5: Testing | #15–17 | Medium — comprehensive coverage needed |
| 6: Documentation/CI | #18–19 | Low — final verification |

## Notes for Implementers

- **AGENTS.md constraints**: No background tasks (sequential workers only), prefer stdlib/crates over binary deps, never introduce `unsafe` without explicit instruction, build/clippy/tests must pass before committing
- **Config defaults must match** `config/rouser.toml` — AGENTS.md source-of-truth rule applies to all three locations simultaneously
- **Breaking changes**: TimeKey removal and ML pipeline change will break existing history file format. Plan for migration or backward compatibility if needed (e.g., log warning when loading old-format entries)
- **Performance target**: Prediction should complete in <100ms at each 30s interval with ~86400 history entries (30 days × 2880 entries/day / 30s flush = ~86,400 entries max)
