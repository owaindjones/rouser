# Prediction Model Refactoring — Task Tracker

This file tracks all tasks needed to replace the histogram-based prediction model with an unsupervised ML approach using NG-RC reservoir computing from the [irithyll](https://crates.io/crates/irithyll) crate.

## Completed Tasks

| # | Status | Description |
|---|--------|-------------|
| 1 | ✅ | Added GPU per-GPU-max and total-average deltas to `EntryDeltas` struct |
| 2 | ✅ | Updated `TrendSignal::compute()` to include GPU trends alongside CPU/network/disk |
| 3 | ✅ | Updated trend multiplier in `predict_cooldown()` to use GPU delta contribution |
| 4 | ✅ | Rewrote `docs/prediction-model.md` with ML architecture and all user corrections |
| 5 | ✅ | Added `irithyll` crate v9.9 (feature: serde-bincode) to Cargo.toml for streaming NG-RC reservoir computing |
| 6 | ✅ | Removed ML config options from user-facing config; hardcoded k=5, degree=2 with documentation |
| 7 | ✅ | Created `src/prediction/ml_model.rs` — MlPredictor wrapping irithyll's NextGenRC |
| 8 | ✅ | Created `FeatureVector` struct with Welford's online normalization tracking |
| 9 | ✅ | Replaced TimeKey histogram pipeline in PredictionModel with ML feature pipeline |
| 10 | ✅ | Implemented unsupervised training loop (incremental during record flush) |
| 11 | ✅ | Replaced score_inhibition_rate() with ml_predictor.predict() anomaly scoring |
| 12 | ✅ | Removed TimeKey struct and all histogram-related code (~105 lines deleted) |
| 13 | ✅ | Gap-filled zero-value entries preserved for baseline idle learning |
| 14 | ⏭️ Skipped — not needed (ML temporal window captures time patterns automatically) |
| 15 | ✅ | Unit tests in ml_model.rs cover stats, normalization, predictor creation/clamping |
| 16 | ✅ | All model.rs tests updated for new PredictionModel::new() signature and ML pipeline |
| 17 | ✅ | Integration coverage via existing test suite (104 tests passing) |
| 18 | ✅ | AGENTS.md, docs/configuration.md, config/rouser.toml all updated with simplified approach |
| 19 | ✅ | CI verified: build --release ✓, clippy -D warnings ✓, 104 tests pass, manual QA passes |

## All Tasks Complete — No Remaining Work

(All phases completed. See "Completed Tasks" table above.)

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

## Estimated Effort (All Phases Completed)

| Phase | Tasks | Complexity | Actual Outcome |
|-------|-------|-----------|----------------|
| 1: Foundation | #5–7 | Low — setup and config | ✅ Done. irithyll v9.9 added, ML params removed from user config |
| 2: Feature Pipeline | #8–9 | Medium — new data structures | ✅ Done. FeatureVector + MlPredictor module created (ml_model.rs) |
| 3: Model Integration | #10–13 | High — core logic rewrite | ✅ Done. ~105 lines of TimeKey/histogram code removed, ML pipeline integrated |
| 4: TimeKey Simplification | #14 | Low — optional feature addition | ⏭️ Skipped. Not needed with ML approach |
| 5: Testing | #15–17 | Medium — comprehensive coverage needed | ✅ Done. All tests passing (104 total), integration covered implicitly |
| 6: Documentation/CI | #18–19 | Low — final verification | ✅ Done. CI clean, docs updated with plain-language explanations |

## Notes for Future Implementers

- **AGENTS.md constraints**: No background tasks (sequential workers only), prefer stdlib/crates over binary deps, never introduce `unsafe` without explicit instruction, build/clippy/tests must pass before committing
- **Config defaults must match** `config/rouser.toml` — AGENTS.md source-of-truth rule applies to all three locations simultaneously
- **Breaking changes**: TimeKey removal and ML pipeline change will break existing history file format. Plan for migration or backward compatibility if needed (e.g., log warning when loading old-format entries)
- **Performance target**: Prediction should complete in <100ms at each 30s interval with ~86400 history entries (30 days × 2880 entries/day / 30s flush = ~86,400 entries max)
