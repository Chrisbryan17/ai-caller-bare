# Hybrid Active-Wallet Rotation Design

Date: 2026-07-17
Status: Approved design
Repository: `Chrisbryan17/ai-caller-bare`
Branch: `polymarket-copybot-final-20260716-done`

## 1. Objective

Replace the fixed three-wallet watchlist with a restart-safe, auditable rotation system that continuously discovers active Crypto wallets, evaluates them using the bot's existing fixed strategy families, quarantines new candidates in paper mode, and permits live use only after each wallet has independently produced sufficient forward paper evidence.

The design must increase signal availability without weakening the existing trade-quality thresholds or allowing a newly discovered leaderboard wallet to receive real-money authority immediately.

## 2. Operating model

The system uses the approved hybrid model:

- Discovery and candidate scoring are automatic.
- Newly discovered wallets are paper-only.
- A wallet becomes live-eligible only after completing its forward paper quarantine.
- Once the bot itself is live, rotation among already qualified wallets is automatic.
- Unqualified wallets remain paper-only even while qualified wallets may trade live.
- A wallet's promotion never increases the global position-risk limit.
- Agreement among wallets never multiplies exposure.

## 3. Non-goals

This change will not:

- lower the existing `$25` first-large-buy threshold;
- lower the existing `$100` confirmed-flow threshold;
- invent a new per-wallet strategy after inspecting that wallet's results;
- use leaderboard P&L alone as evidence of copyability;
- allow more than one open market position;
- bypass geoblocking, authentication, signature, allowance, balance, or platform restrictions;
- increase risk above the configured current tier;
- automatically graduate the account from 5% to 25% risk;
- submit live orders from tests, CI, discovery jobs, or quarantine evaluation.

## 4. Discovery cadence and source set

A `DiscoveryCoordinator` runs every 15 minutes, with jitter of up to 30 seconds to avoid synchronized bursts.

For each cycle it requests the top 50 Crypto leaderboard entries for:

- day;
- week;
- month.

The resulting addresses are normalized to lowercase and deduplicated. Existing active, quarantined, suspended, and recently rejected wallets are preserved in the candidate state store so that discovery remains restart-safe.

A failed leaderboard request does not mutate the current watchlist. Partial leaderboard results may be scored only when at least two of the three periods succeeded; otherwise the cycle fails closed.

## 5. Candidate eligibility filters

A candidate must pass all hard filters before replay scoring.

### 5.1 Market-domain filter

Only public trades whose slugs match the currently supported short-duration Crypto markets are admissible:

- BTC, ETH, SOL, or XRP;
- 5-minute or 15-minute Up/Down markets.

Sports, elections, macro, entertainment, and unrelated prediction markets are ignored.

### 5.2 Recency filter

A candidate must have at least one strategy-qualifying trade during the last six hours.

A trade counts as strategy-qualifying only when it satisfies one of the fixed strategy families defined below. Mere wallet activity is insufficient.

### 5.3 Activity-density filter

The candidate must have at least five chronological strategy signals within the replay lookback window and at least two within the last 24 hours.

The replay lookback window is seven completed days plus the current partial day, bounded by the amount of data available from the public API.

### 5.4 Copyability filter

The candidate is rejected when any of the following applies:

- the observed source price is at or above `0.95` for more than 80% of signals;
- modeled profitability becomes non-positive after the configured taker fee and one-cent adverse slippage;
- fewer than 70% of qualifying signals remain executable after the configured minimum lead time;
- the candidate's median qualifying notional is below the strategy threshold;
- the wallet's activity is dominated by offsetting both sides of the same condition in a manner that makes directional copying materially different from the wallet's own exposure.

### 5.5 Correlation filter

The selected live-eligible set must not contain multiple wallets whose qualifying signals are effectively duplicates.

Two wallets are considered strongly correlated when, over the common replay window:

- at least 70% of their signals occur in the same condition;
- at least 80% of those shared-condition signals select the same outcome; and
- their median signal timestamps differ by no more than five seconds.

Strongly correlated wallets may both remain in quarantine for observation, but only the higher-ranked wallet may occupy an active live slot. Agreement is treated as confirmation of one signal, not as additional exposure.

## 6. Fixed strategy families

Discovery may assign a candidate only to one of these predeclared strategy families.

### 6.1 First large buy

A signal is emitted for the first qualifying BUY in a condition when:

- trade notional is at least `$25`;
- at least 90 seconds remain before market end;
- the market is supported;
- no earlier BUY for that wallet and condition has already emitted a signal.

Replay assumes execution at source price plus one cent, capped below `1.00`, plus the documented Crypto taker fee model.

### 6.2 Confirmed directional flow

A signal is emitted when cumulative BUY flow in one outcome reaches at least `$100`, that outcome represents at least 80% of cumulative BUY notional in the condition, at least 90 seconds remain, and the source price lies within the configured specialist price band.

The initial specialist price band remains `0.40` through `0.55`. Changes to that band require a separate reviewed design because it was selected from historical data and is vulnerable to overfitting.

### 6.3 Strategy assignment

For each candidate, both fixed families are replayed when technically applicable. The candidate receives the family with the better conservative score only when that family has at least five signals. The system must not tune thresholds independently for each wallet.

## 7. Conservative replay score

Each candidate receives a deterministic score derived only from chronological replay results.

The score includes:

- fee-and-slippage-adjusted net return on turnover;
- lower-confidence win-rate estimate rather than raw win rate;
- signal count;
- qualifying signals during the last 24 hours;
- maximum consecutive losses;
- maximum replay drawdown;
- median source price;
- median lead time;
- percentage of signals that remain positive at two cents adverse slippage;
- correlation penalty;
- inactivity decay.

A candidate is ineligible when its conservative expected value is non-positive at one-cent slippage. Positive performance at two-cent slippage improves ranking but is not a hard requirement.

The exact score weights must be constants committed in source control and covered by tests. They may not be changed dynamically in production.

## 8. Wallet lifecycle

Every wallet has exactly one lifecycle state.

### 8.1 Discovered

The wallet appeared in the candidate pool but has not completed replay evaluation.

### 8.2 Rejected

The wallet failed a hard filter or replay threshold. The rejection record stores machine-readable reasons and a retry-after timestamp.

Default retry periods:

- inactivity: 60 minutes;
- insufficient sample: 6 hours;
- non-positive replay edge: 24 hours;
- unsupported activity mix: 24 hours;
- strong correlation: re-evaluate whenever the active set changes.

### 8.3 Quarantined

The wallet passed historical replay and is monitored in forward paper mode. Historical trades used for scoring are never counted toward quarantine promotion.

A newly quarantined wallet is primed from its first successful snapshot. Only post-prime transactions may create forward paper signals.

### 8.4 Paper-qualified

A wallet becomes paper-qualified only after all of the following are true:

- at least five resolved forward paper positions;
- at least 60 minutes elapsed since quarantine began;
- positive net paper P&L after modeled fees and configured slippage;
- maximum wallet-specific paper drawdown no greater than 5% of the current bankroll;
- no more than three consecutive paper losses;
- at least 80% of signals were processed without stale-data, malformed-data, or execution-model errors;
- no unresolved accounting discrepancies.

Passing these gates grants eligibility; it does not force immediate selection.

### 8.5 Active paper

Before the global bot is live, the highest-ranked qualified wallets occupy the active paper slots. Up to three wallets may be active, subject to correlation limits.

### 8.6 Active live

Once the global launch controller has independently passed its live gate, up to three paper-qualified wallets may occupy live slots. All global risk limits still apply:

- starting bankroll: `$95` unless overridden by verified live balance reconciliation;
- current maximum position-risk fraction: `0.05`;
- current maximum modeled position cost: `$4.75` at a `$95` bankroll;
- current daily capital-at-risk limit: `$9.50`;
- one open market position at a time;
- one-cent maximum deterioration;
- FOK marketable limit BUY only.

### 8.7 Suspended

A wallet is suspended from new live signals when any wallet-specific or global demotion condition is met.

Wallet-specific suspension triggers include:

- two consecutive resolved live losses;
- wallet-specific live drawdown greater than 5% of the reconciled bankroll;
- three signals rejected for stale data within one hour;
- seven days without a qualifying signal;
- no longer passing the current replay screen;
- detected strategy drift, such as a material shift into unsupported markets or pervasive two-sided hedging.

A suspended wallet returns to quarantine. It cannot resume live use until it requalifies with new forward paper evidence.

## 9. Active-set selection and rotation

The `ActiveSetManager` maintains at most three active wallets.

Rotation may occur only when:

- no market position is open;
- no order submission is in flight;
- the new candidate is fully primed;
- the state snapshot has been persisted successfully;
- the selected set satisfies the correlation constraint.

Rotation policy:

1. Preserve qualified incumbents unless they are inactive, suspended, materially outscored, or newly correlated with a better candidate.
2. Require a challenger to exceed the incumbent's conservative score by at least 10% before replacing a healthy incumbent.
3. Limit ordinary replacements to one wallet per discovery cycle.
4. Permit immediate removal of a wallet that becomes unsafe or ineligible.
5. Never remove the last usable wallet merely because discovery failed.

When multiple selected wallets produce the same condition and outcome during one polling cycle, the system emits one consolidated signal using the highest-priority qualified wallet's metadata. When selected wallets conflict on outcome, the condition is skipped.

## 10. Runtime architecture

### 10.1 `LeaderboardClient`

Fetches day, week, and month Crypto leaderboard rows. It owns HTTP request construction, timeout policy, status validation, and response decoding.

### 10.2 `CandidatePool`

Normalizes addresses, merges leaderboard evidence, tracks retry-after times, and exposes candidates due for evaluation.

### 10.3 `ReplayEvaluator`

Fetches historical public trades, filters supported markets, runs both fixed strategy families chronologically, computes conservative metrics, and returns a deterministic `CandidateEvaluation`.

### 10.4 `CorrelationAnalyzer`

Compares signal traces and produces pairwise correlation decisions with supporting metrics.

### 10.5 `WalletRegistry`

Persists wallet lifecycle state, assigned strategy, evaluation evidence, quarantine counters, paper/live performance, suspension reasons, and timestamps.

### 10.6 `DynamicWatcherSet`

Creates, primes, adds, and removes `WalletWatcher` instances. Watcher replacement is atomic from the main loop's perspective.

### 10.7 `ActiveSetManager`

Selects the best uncorrelated active set and enforces the rotation rules.

### 10.8 `DiscoveryCoordinator`

Schedules discovery, bounds concurrent API calls, applies backoff, and submits completed evaluations to the registry and active-set manager.

### 10.9 `LaunchController` integration

The global launch controller remains authoritative. Wallet qualification cannot bypass the one-hour global paper gate, resolved-position minimum, profitability requirement, preflight checks, or explicit armed-live policy.

## 11. Concurrency model

Discovery must not block the low-latency trade polling loop.

- Discovery runs in a separate Tokio task.
- It publishes immutable proposed-registry updates through a bounded channel.
- The main loop applies updates only at safe rotation points.
- At most four candidate replay fetches may run concurrently.
- Live wallet polling retains its existing cadence and independent backoff.
- Slow or failed candidate evaluation cannot delay signal handling for the current active set.

## 12. Persistence and restart behavior

State is persisted as an atomic JSON document or equivalent transactional store outside the hot path.

The persisted state includes:

- schema version;
- active, quarantined, rejected, qualified, and suspended wallets;
- strategy assignment and immutable threshold version;
- replay metrics and source time window;
- quarantine start and prime timestamp;
- resolved paper and live outcomes;
- score history;
- correlation evidence;
- retry-after times;
- last successful discovery time;
- current active-set generation.

Writes use a temporary file followed by atomic rename. A failed write leaves the previous valid state intact.

After restart:

- current active wallets are re-primed before signal generation;
- no trade that predates the restart prime may trigger;
- quarantine elapsed time may be restored, but only previously persisted resolved paper positions count;
- an uncertain or corrupt state file causes paper-only fail-closed operation with the static fallback watchlist disabled until state is repaired or rebuilt.

## 13. Journaling and observability

The journal gains structured records for:

- discovery-cycle start and completion;
- leaderboard source success or failure;
- candidate discovered;
- hard-filter rejection;
- replay evaluation metrics;
- quarantine started;
- wallet primed;
- wallet paper-qualified;
- active-set proposal;
- active-set applied;
- rotation deferred because a position is open;
- wallet suspended or demoted;
- correlation conflict;
- fallback preservation after discovery failure.

Every record includes an epoch timestamp and active-set generation. Secrets, credentials, private keys, and recovery material must never be serialized.

Required health metrics include:

- age of last successful discovery;
- number of active, quarantined, qualified, and suspended wallets;
- candidate evaluation latency;
- current wallet signal age;
- API error rate by endpoint;
- active-set generation;
- paper and live P&L by wallet;
- stale-signal and conflict counts.

## 14. Failure handling

The system fails closed.

- Leaderboard failure: retain the existing active set.
- Candidate trade-history failure: leave candidate state unchanged and retry with bounded exponential backoff.
- Malformed candidate data: reject that evaluation only.
- Persistence failure: do not apply the proposed rotation.
- Correlation-analysis failure: preserve incumbents and do not promote challengers.
- No eligible candidates: continue paper monitoring of quarantined wallets and retain safe qualified incumbents.
- All active wallets suspended: remain operational in observation mode but emit no live orders.
- Clock anomaly: reject discovery results and remain on the prior active set.

## 15. Security boundaries

Discovery and paper quarantine require only public APIs.

The discovery subsystem must never receive:

- wallet private keys;
- API secrets;
- passphrases;
- seed phrases;
- signing handles.

Only the existing live executor may access signing material, and only after the global launch controller authorizes a live request.

## 16. Testing strategy

Implementation follows strict red-green-refactor TDD.

### 16.1 Unit tests

Required unit coverage includes:

- leaderboard deduplication across day, week, and month;
- partial-source failure behavior;
- recency and activity-density filters;
- supported-market filtering;
- deterministic replay for both fixed strategies;
- fee and one-cent/two-cent slippage calculations;
- conservative scoring and stable tie-breaking;
- correlation classification boundaries;
- lifecycle transitions;
- quarantine counters excluding historical replay trades;
- promotion and suspension gates;
- 10% challenger replacement hysteresis;
- one-replacement-per-cycle rule;
- conflict skip and agreement consolidation;
- restart priming;
- atomic state-file recovery;
- fail-closed persistence errors;
- current `$95` risk limits remaining unchanged during rotation.

### 16.2 Property tests

Property tests verify:

- candidate ordering is deterministic for any input order;
- adding an inferior candidate cannot displace a superior healthy incumbent;
- no active set exceeds three wallets;
- no active set contains a prohibited correlated pair;
- agreement never increases requested capital;
- historical trades never become forward quarantine results;
- rotations never occur while a position or submission is open.

### 16.3 Integration tests

HTTP integration tests use a local mock server for leaderboard and trade-history endpoints. Scenarios include pagination-sized responses, timeouts, non-200 statuses, malformed rows, duplicate addresses, inactive leaders, correlated leaders, and recovery after transient failure.

A restart integration test persists a populated registry, restarts the engine, primes all restored watchers, and proves that no pre-restart transaction emits a signal.

### 16.4 End-to-end paper test

A deterministic recorded-data test must demonstrate:

1. static incumbents become inactive;
2. discovery finds active candidates;
3. historical replay qualifies a candidate for quarantine only;
4. forward paper signals accumulate after priming;
5. the candidate becomes paper-qualified after the required resolved evidence;
6. rotation occurs only with no open position;
7. the new wallet produces one consolidated paper signal;
8. no live executor method is called.

### 16.5 Live-safety tests

Tests must prove that:

- discovery cannot instantiate or call a live executor;
- an unqualified wallet cannot generate a live execution request;
- global paper/live gating remains authoritative;
- 5% risk remains the maximum starting tier;
- changing the active set does not reset daily capital-at-risk accounting;
- CI contains no signing material and sends no real order.

## 17. Verification gate

Before deployment, the implementation must pass:

```bash
cargo fmt --all -- --check
cargo test --all-targets
cargo test --all-targets --all-features
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
cargo build --release --all-features
```

It must also complete:

- a public-API discovery smoke cycle;
- a public-API paper polling smoke cycle;
- a restart/priming smoke cycle;
- a secret-pattern scan;
- an assertion that no live order endpoint was called.

## 18. Deployment sequence

1. Implement and verify discovery in observation-only mode.
2. Run discovery on the VPS while preserving the current fixed watchlist.
3. Compare proposed candidates and replay evidence against offline calculations.
4. Enable dynamic paper quarantine, still with no live rotation.
5. Accumulate and inspect forward paper evidence.
6. Enable automatic paper active-set rotation.
7. Only after global live gating and wallet qualification are both verified may qualified-wallet live rotation be armed.
8. Begin at the existing 5% risk tier. Risk graduation remains a separate evidence-based process.

## 19. Acceptance criteria

The feature is accepted only when:

- inactive static wallets no longer prevent the system from discovering paper signals;
- discovery failures cannot remove a healthy active set;
- no historical trade triggers after discovery, promotion, rotation, or restart;
- every newly discovered wallet is paper-only until independently qualified;
- live rotation uses only qualified wallets;
- correlation cannot multiply exposure;
- existing global risk and launch gates remain intact;
- all lifecycle decisions are persisted and journaled with reasons;
- the full verification matrix passes with zero live orders transmitted.
