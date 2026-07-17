# Active Wallet Rotation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the fixed three-wallet watchlist with automatic public leaderboard discovery, deterministic historical replay, forward-paper quarantine, restart-safe qualification, and automatic rotation among qualified wallets while preserving the $95 account's 5% risk controls.

**Architecture:** Add public leaderboard and market-resolution clients, a deterministic replay evaluator using the existing strategy engines, a persisted wallet registry, and an active-set manager. The runtime maintains separate paper and live watcher sets: all quarantined wallets are observed in paper, while only paper-qualified wallets may enter the live active set. Discovery runs outside the low-latency polling loop and publishes rotation proposals that the main loop applies only while no position or order is open.

**Tech Stack:** Rust 1.88, Tokio, Reqwest/Rustls, Serde/JSON, rust_decimal, Wiremock, Proptest, existing Polymarket V2 Rust SDK for the separately gated live executor.

## Global Constraints

- Discovery uses only public Polymarket APIs and never receives signing material.
- Top 50 Crypto leaderboard entries are fetched for DAY, WEEK, and MONTH every 15 minutes.
- At least two leaderboard periods must succeed before a discovery cycle may mutate candidate state.
- Existing `$25` first-large-buy and `$100` confirmed-flow thresholds remain unchanged.
- Replay applies documented taker fees and one-cent adverse slippage; two-cent resilience is reported.
- New wallets are paper-only until at least five resolved forward paper positions, 60 minutes, positive net P&L, maximum drawdown no greater than 5% of bankroll, no more than three consecutive losses, and at least 80% processing health.
- Starting bankroll is `$95`; maximum risk fraction remains `0.05`; maximum modeled position cost is `$4.75`; initial daily capital-at-risk limit is `$9.50`.
- One market position maximum; agreement never multiplies exposure; conflicting outcomes are skipped.
- Rotation is forbidden while a position or submission is open and requires a successfully persisted state snapshot.
- Tests and CI never instantiate or call the live executor and never contain signing material.

---

### Task 1: Public leaderboard and market-resolution clients

**Files:**
- Create: `src/discovery_api.rs`
- Modify: `src/lib.rs`
- Test: `tests/discovery_api.rs`

**Interfaces:**
- Produces `LeaderboardPeriod`, `LeaderboardRow`, `LeaderboardSnapshot`, `MarketResolution`, and `DiscoveryApiClient`.
- `DiscoveryApiClient::fetch_leaderboard(period, limit)` calls `/v1/leaderboard` with `category=CRYPTO`, `orderBy=PNL`, `offset=0`.
- `DiscoveryApiClient::fetch_resolutions(condition_ids)` calls Gamma `/markets` in bounded batches and parses stringified `outcomes`/`outcomePrices` arrays.

- [ ] Write failing Wiremock tests for exact leaderboard query parameters, address normalization, decimal parsing, non-200 preservation, and resolved/unresolved market parsing.
- [ ] Run `cargo test --test discovery_api`; expect unresolved imports.
- [ ] Implement the models and HTTP methods with deterministic sorting and validation.
- [ ] Run `cargo test --test discovery_api`; expect all tests to pass.

### Task 2: Deterministic chronological replay evaluator

**Files:**
- Create: `src/replay.rs`
- Modify: `src/lib.rs`
- Test: `tests/replay.rs`

**Interfaces:**
- Produces `StrategyFamily`, `ReplaySignal`, `ReplayMetrics`, `CandidateEvaluation`, and `ReplayEvaluator`.
- `ReplayEvaluator::evaluate(wallet, trades, resolutions, now)` replays both fixed strategy families, selects the stronger eligible family, and never tunes thresholds per wallet.
- Metrics include signal count, recent counts, win/loss counts, fee/slippage P&L, turnover, ROI, max loss streak, max drawdown, median price, median lead, two-cent-positive percentage, latest qualifying timestamp, and deterministic score.

- [ ] Write failing tests for supported-market filtering, first-large-buy replay, confirmed-flow replay, unresolved-market exclusion, one-cent and two-cent accounting, activity/recency gates, hedging rejection, stable tie-breaking, and non-positive-edge rejection.
- [ ] Run `cargo test --test replay`; expect unresolved imports.
- [ ] Implement replay using `StrategyEngine`, `crypto_taker_fee`, fixed constants, chronological ordering, and deterministic integer/decimal scoring.
- [ ] Run `cargo test --test replay`; expect all tests to pass.

### Task 3: Wallet lifecycle registry and atomic persistence

**Files:**
- Create: `src/registry.rs`
- Modify: `src/lib.rs`
- Test: `tests/registry.rs`

**Interfaces:**
- Produces `WalletLifecycle`, `WalletRecord`, `RegistryState`, `WalletRegistry`, `PaperOutcome`, and `SuspensionReason`.
- `WalletRegistry::load_or_new(path, bankroll)` validates schema/version and fails closed on corrupt state.
- `WalletRegistry::save_atomic()` writes a sibling temporary file, flushes, and renames.
- Historical replay never increments forward-paper counters.

- [ ] Write failing tests for every lifecycle transition, retry-after behavior, paper qualification gates, suspension/demotion, corrupt-state fail-closed behavior, atomic replacement, restart restoration, and unchanged `$95` risk metadata.
- [ ] Run `cargo test --test registry`; expect unresolved imports.
- [ ] Implement serializable state, validation, transition methods, and atomic persistence.
- [ ] Run `cargo test --test registry`; expect all tests to pass.

### Task 4: Correlation analysis and active-set rotation

**Files:**
- Create: `src/rotation.rs`
- Modify: `src/lib.rs`, `src/coordinator.rs`
- Test: `tests/rotation.rs`

**Interfaces:**
- Produces `CorrelationEvidence`, `RotationContext`, `ActiveSetProposal`, `CorrelationAnalyzer`, and `ActiveSetManager`.
- `ActiveSetManager::propose(registry, traces, current, context)` returns at most three uncorrelated wallets, preserves healthy incumbents, requires a 10% challenger advantage, and limits ordinary replacement to one per cycle.
- `select_signal_with_priority(signals, priorities)` consolidates agreement without changing position size.

- [ ] Write failing tests for correlation boundaries, deterministic candidate order, three-wallet maximum, 10% hysteresis, one replacement per cycle, no rotation during open position/submission, persistence prerequisite, conflict skipping, and agreement not increasing capital.
- [ ] Run `cargo test --test rotation`; expect unresolved imports.
- [ ] Implement correlation and active-set selection with explicit tie-breaking.
- [ ] Run `cargo test --test rotation`; expect all tests to pass.

### Task 5: Dynamic watcher set and restart priming

**Files:**
- Create: `src/dynamic_watchers.rs`
- Modify: `src/lib.rs`, `src/watcher.rs`
- Test: `tests/dynamic_watchers.rs`

**Interfaces:**
- Produces `DynamicWatcherSet` with `synchronize`, `process_snapshot`, `is_primed`, and `wallets`.
- New/restored watchers always prime from a successful snapshot before emitting.
- Strategy changes replace and re-prime the watcher atomically.

- [ ] Write failing tests for add/remove, strategy replacement, first-snapshot priming, restart priming, and absence of historical signal emission.
- [ ] Run `cargo test --test dynamic_watchers`; expect unresolved imports.
- [ ] Implement the bounded watcher map and strategy fingerprinting.
- [ ] Run `cargo test --test dynamic_watchers`; expect all tests to pass.

### Task 6: Discovery coordinator and fail-closed update channel

**Files:**
- Create: `src/discovery.rs`
- Modify: `src/lib.rs`
- Test: `tests/discovery.rs`

**Interfaces:**
- Produces `DiscoveryConfig`, `DiscoveryCycleResult`, `RegistryUpdate`, and `DiscoveryCoordinator`.
- A cycle fetches three periods concurrently, requires two successes, deduplicates candidates, respects retry-after timestamps, evaluates at most four candidates concurrently, and leaves the active set unchanged on failure.

- [ ] Write failing async tests for full success, one-period failure, two-period failure, duplicate wallets, bounded concurrency, candidate failure isolation, and incumbent preservation.
- [ ] Run `cargo test --test discovery`; expect unresolved imports.
- [ ] Implement the coordinator using Tokio tasks/semaphores and a bounded channel.
- [ ] Run `cargo test --test discovery`; expect all tests to pass.

### Task 7: Paper resolution accounting and launch controller

**Files:**
- Create: `src/launch.rs`
- Modify: `src/lib.rs`, `src/model.rs`
- Test: `tests/launch.rs`

**Interfaces:**
- Produces `LaunchMode`, `PaperPosition`, `LaunchGateSnapshot`, `LaunchController`, and `PreflightStatus`.
- The controller resolves paper positions from public market outcomes, records per-wallet outcomes in the registry, and permits armed-live status only after both global and wallet-specific gates pass.
- It never creates an `ExecutionRequest`; live submission remains in the existing executor path.

- [ ] Write failing tests for one-hour minimum, five-resolved minimum, positive-P&L requirement, drawdown limit, processing-health gate, unresolved positions, wallet qualification, restart persistence, and fail-closed preflight results.
- [ ] Run `cargo test --test launch`; expect unresolved imports.
- [ ] Implement deterministic accounting and gate decisions.
- [ ] Run `cargo test --test launch`; expect all tests to pass.

### Task 8: Runtime integration, CLI, journaling, and operational documentation

**Files:**
- Modify: `src/main.rs`, `src/journal.rs`, `src/data_api.rs`, `README.md`, `.env.example`
- Create: `RUNBOOK_LOCAL_AGENT.md`
- Test: `tests/runtime_rotation.rs`, `tests/journal_rotation.rs`

**Interfaces:**
- Adds CLI options `--registry`, `--discovery-ms`, `--leaderboard-limit`, `--gamma-api-base`, `--clob-public-base`, `--rotation-observe-only`, and `--auto-live`.
- Defaults remain paper-safe: `--auto-live=false`; dynamic discovery starts observation-only unless explicitly enabled.
- Runtime applies registry updates only at safe points, polls quarantined wallets in paper, routes live requests only from qualified active wallets, preserves global risk state across rotation, and journals every lifecycle decision.

- [ ] Write failing runtime tests using recorded/mock public data to prove inactive incumbents are replaced in paper, quarantine is forward-only, rotation waits for an empty position, state survives restart, and no live executor method is called.
- [ ] Run the focused runtime tests; expect failures for missing integration.
- [ ] Integrate the coordinator, registry, dynamic watchers, active-set manager, resolution polling, and launch controller into `main.rs` without blocking the hot polling loop.
- [ ] Extend structured journal records and write the local-agent runbook with exact safe commands.
- [ ] Run all focused tests; expect pass.

### Task 9: Full verification and public smoke evidence

**Files:**
- Modify: `.github/workflows/copybot-ci.yml`

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo test --all-targets`.
- [ ] Run `cargo test --all-targets --all-features`.
- [ ] Run `cargo clippy --all-targets -- -D warnings`.
- [ ] Run `cargo clippy --all-targets --all-features -- -D warnings`.
- [ ] Run `cargo build --release`.
- [ ] Run `cargo build --release --all-features`.
- [ ] Run a public leaderboard discovery smoke cycle in observation-only paper mode.
- [ ] Run a public wallet polling smoke cycle.
- [ ] Run a restart/priming smoke cycle against a temporary registry.
- [ ] Scan tracked files for credential patterns.
- [ ] Assert from the mock server and logs that no live order endpoint was called.
- [ ] Archive source, verification logs, discovered-candidate summary, and the local-agent runbook as CI artifacts.
