# Polymarket Copybot — $95 Paper-to-Live Handoff

## Mission

Continue the existing Rust copybot from this repository and branch. Add a rigorously tested launch controller that:

1. runs the complete verification matrix before runtime;
2. runs in paper mode for at least one hour;
3. evaluates explicit operational and strategy gates using resolved paper trades;
4. switches to live mode automatically only when every gate passes and the operator has deliberately armed live trading;
5. starts at a maximum 5% bankroll allocation per position;
6. graduates risk only through evidence-based stages, never directly from 5% to 25% after one hour.

No real order may be submitted during development, CI, tests, or the initial paper hour.

## Repository

- Repository: `Chrisbryan17/ai-caller-bare`
- Branch: `polymarket-copybot-final-20260716-done`
- Pull request: `#2 — Build rigorously tested Rust Polymarket copybot`
- Current verified commit before this handoff: `ef8f1145714d6f8f6f295b8bd0318506fbd6c64a`
- Current CI run: `29549717214`
- Current CI conclusion: success

The branch is isolated and is not merged into `main`.

## Current account parameters

- Starting Polymarket bankroll: **$95.00**
- Stage-1 position allocation cap: **5% = $4.75**
- Initial daily capital-at-risk cap: **10% = $9.50**
- Maximum concurrent positions: **1**
- Maximum source-price deterioration: **$0.01**
- Hard absolute slippage configuration ceiling: **$0.02**
- Minimum remaining market time: **90 seconds**
- Order type: **FOK marketable limit BUY** with a hard maximum price

Do not hard-code a fixed share count. Use the existing exact-decimal position sizer against the live executable maximum price and fee model.

## Existing strategy set

The current branch watches:

1. Primary wallet: `0x208326efd5d051c59631ed626848b150b8d8259c`
   - First BUY with at least $25 notional
   - At least 90 seconds remaining

2. Secondary wallet: `0x45230b4fb12569efcc908b4d22c3cee4a19429e2`
   - First BUY with at least $25 notional
   - At least 90 seconds remaining

3. Specialist wallet: `0xb89d0b6e96e790afa900b53476b8f267a94d1d4f`
   - At least $100 cumulative BUY flow
   - Dominant outcome at least 80%
   - Source price between $0.40 and $0.55
   - At least 90 seconds remaining

Supported markets are BTC, ETH, SOL, and XRP 5-minute or 15-minute Up/Down markets.

## Credential warning

An API key alone is not sufficient to sign Polymarket orders.

The live process needs a compatible wallet signer/private key for the funded account architecture. The current live adapter is EOA-based. Never place any private key, API secret, passphrase, seed phrase, or session token in this repository, issue, pull request, chat, command-line argument, or journal.

Use environment variables or a production secret manager. At minimum, the current EOA adapter expects:

- `POLYMARKET_PRIVATE_KEY`
- `POLYMARKET_LIVE_ACK=I_UNDERSTAND_REAL_MONEY`

If the funded Polymarket account is not an EOA account, stop and implement the correct signature type and funder address before live testing. API credentials do not correct a signature-type mismatch.

## Required TDD sequence

Do not change production launch behavior first.

### RED

Add focused failing tests for each behavior below and run them to confirm failure for the intended reason:

1. `paper_gate_never_arms_before_3600_seconds`
2. `paper_gate_extends_when_minimum_resolved_sample_is_missing`
3. `paper_gate_rejects_negative_net_pnl_after_fees`
4. `paper_gate_rejects_excess_drawdown`
5. `paper_gate_rejects_poll_or_journal_failures`
6. `paper_gate_rejects_geoblocked_runtime`
7. `paper_gate_rejects_missing_live_acknowledgement`
8. `paper_gate_rejects_missing_or_invalid_signer`
9. `paper_gate_rejects_insufficient_balance_or_allowance`
10. `paper_gate_arms_live_only_when_every_condition_passes`
11. `live_stage_starts_at_five_percent_for_a_95_dollar_bankroll`
12. `risk_stage_never_jumps_directly_from_five_to_twenty_five_percent`
13. `risk_stage_demotes_after_drawdown_or_execution_degradation`
14. `restart_recovers_launch_stage_without_replaying_old_signals`
15. `tests_and_ci_can_never_construct_a_live_order_sender`

### GREEN

Implement only enough code to satisfy the tests. Prefer a pure launch-state module with deterministic inputs, for example:

- `LaunchStage::Verifying`
- `LaunchStage::Paper`
- `LaunchStage::LiveFivePercent`
- `LaunchStage::LiveTenPercent`
- `LaunchStage::LiveFifteenPercent`
- `LaunchStage::LiveTwentyPercent`
- `LaunchStage::LiveTwentyFivePercent`
- `LaunchStage::Halted`

Persist state atomically outside the append-only trade journal. A crash or restart must not reset the paper timer, risk budget, loss counters, or stage history incorrectly.

### REFACTOR

After all tests pass, remove duplication and rerun the entire verification matrix. Do not alter strategy thresholds during the launch-controller work.

## One-hour paper gate

The one-hour timer is a minimum, not sufficient proof by itself.

The bot may arm live mode only when all of the following are true:

- continuous paper runtime is at least 3,600 seconds;
- at least **5 qualifying paper positions have resolved**;
- resolved paper net P&L after modeled taker fees and configured slippage is positive;
- maximum paper drawdown is no greater than **5% of starting bankroll ($4.75)**;
- no unresolved journal write error occurred;
- no stale startup transaction was emitted as a signal;
- no duplicate market was executed;
- no conflicting wallet signal was executed;
- public Data API and CLOB health checks pass;
- geoblock endpoint reports `blocked: false` for the deployment IP;
- authenticated CLOB preflight succeeds without submitting an order;
- collateral balance and token allowance cover at least the next capped position;
- there are no unknown resting orders from a previous process;
- the exact live acknowledgement is present;
- the operator explicitly started the process with an auto-live flag.

If one hour elapses but fewer than five positions have resolved, remain in paper mode until the sample requirement is met. Do not manufacture trades to satisfy the gate.

## Paper resolution and P&L

The current branch records signals and paper fills but does not yet provide sufficient resolved-P&L accounting for this gate. Add a resolver using official public market data.

Requirements:

- resolve by condition ID/token outcome, not by title text;
- tolerate delayed market resolution;
- never count an unresolved market as a win or loss;
- include modeled fee precision and paper slippage;
- append immutable resolution records to the journal;
- make resolution processing idempotent;
- update paper bankroll and drawdown from resolved records only;
- test delayed, duplicate, malformed, and contradictory resolution responses.

## Preflight command gate

Before the one-hour paper session, run and require zero exit status from:

```bash
cargo fmt --all -- --check
cargo test --all-targets
cargo test --all-targets --all-features
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
cargo build --release --all-features
cargo run --release -- --mode paper --once --bankroll 95 --max-risk-fraction 0.05 --max-daily-capital-at-risk 9.50
```

Also verify that default-mode live startup fails without the exact acknowledgement.

## Proposed runtime command

After implementing and verifying the launch controller, expose an explicit command similar to:

```bash
cargo run --release --features live-trading -- \
  --mode staged \
  --bankroll 95 \
  --paper-min-seconds 3600 \
  --paper-min-resolved 5 \
  --max-risk-fraction 0.05 \
  --max-daily-capital-at-risk 9.50 \
  --max-slippage 0.01 \
  --poll-ms 250 \
  --auto-live-after-paper
```

The exact flag names may change, but the behavior and safety gates must remain testable and explicit.

## Risk graduation policy

Do not graduate based on elapsed time alone. Do not jump directly from 5% to 25%.

### Stage 1 — 5%

- Per-position cap: 5% of current reconciled bankroll
- Starting dollar cap at $95: $4.75
- Minimum evidence before promotion:
  - at least 50 resolved live positions;
  - at least 7 separate UTC trading days;
  - positive net P&L after actual fees;
  - maximum live drawdown no greater than 10%;
  - p95 observed price deterioration no greater than $0.01;
  - no safety, signing, reconciliation, or duplicate-execution incident.

### Stage 2 — 10%

Require another 50 resolved live positions and another 7 UTC days under the same quality constraints.

### Stage 3 — 15%

Require at least 150 total resolved live positions, at least 21 UTC days, positive rolling 50-trade expectancy, and no drawdown above 12%.

### Stage 4 — 20%

Require at least 200 total resolved live positions, at least 30 UTC days, positive rolling 100-trade expectancy, and no drawdown above 15%.

### Stage 5 — 25%

Twenty-five percent per binary position can lose $23.75 from the initial $95 balance on one losing trade. It must therefore require:

- at least 250 resolved live positions;
- at least 30 UTC days;
- positive net P&L after all actual fees and execution costs;
- positive rolling 100-trade expectancy;
- maximum drawdown no greater than 15%;
- zero unresolved accounting mismatch;
- an additional explicit acknowledgement, separate from the general live acknowledgement;
- automatic demotion to 5% after any two-loss sequence, 10% drawdown, or p95 deterioration above $0.01.

Do not automatically promote to 25% merely because prior stages elapsed. Promotion must be evidence-based and reversible.

## Live safeguards

- One open market at a time.
- At 5%, no single position may exceed $4.75 while the bankroll remains $95.
- Initial daily capital-at-risk cap: $9.50.
- Stop opening positions after two resolved live losses in the same UTC day.
- Halt on any unknown order status, signing mismatch, funder mismatch, balance mismatch, journal failure, contradictory resolution, or repeated API failure.
- Use FOK so an unfilled order does not rest.
- Validate `success`, status, matched amounts, and effective fill price.
- Reject a fill whose effective price exceeds the hard ceiling.
- Reconcile the authenticated order/trade feed before permitting another position.
- Never bypass geoblocking or platform restrictions.

## Current code locations

- Runtime: `src/main.rs`
- Strategy engine: `src/strategy.rs`
- Wallet snapshot coordination: `src/coordinator.rs`
- Risk controls: `src/risk.rs`
- Position sizing and fees: `src/sizing.rs`
- Paper and response validation: `src/execution.rs`
- Live V2 adapter: `src/live.rs`
- Public Data API client: `src/data_api.rs`
- Journal: `src/journal.rs`
- Core tests: `tests/core.rs`
- Live response tests: `tests/live_response.rs`
- Daily-risk tests: `tests/risk_daily.rs`
- CI: `.github/workflows/copybot-ci.yml`

## Definition of done

Do not claim completion until fresh evidence shows:

- every new launch-controller test passed;
- the test was observed failing before its implementation;
- all existing tests still pass in default and all-feature modes;
- strict Clippy passes in default and all-feature modes;
- release builds pass in default and all-feature modes;
- public one-cycle paper smoke test passes;
- a full one-hour paper run completes without fatal error;
- at least five paper positions resolve and the gate evaluation is reproducible from the journal;
- a deliberately failed gate remains in paper mode;
- a fully satisfied gate reaches an armed-live state in a no-order test harness;
- no CI/test code contains or receives real credentials;
- no real order was submitted as part of verification.

Only the operator may start the final real-money process with credentials present locally.
