# Polymarket Rust Copybot Design

## Objective

Build a low-latency Rust service that watches selected public Polymarket wallets, translates their transactions into fixed-risk small-account signals, and runs in paper mode by default. Live execution is compiled and activated only through explicit opt-in safeguards.

## Scope

The first production candidate supports short-duration `btc`, `eth`, `sol`, and `xrp` Up/Down markets with `5m` or `15m` slugs. It monitors:

- `0x208326efd5d051c59631ed626848b150b8d8259c` using the first BUY with at least $25 notional and at least 90 seconds remaining.
- `0x45230b4fb12569efcc908b4d22c3cee4a19429e2` using the same rule.
- `0xb89d0b6e96e790afa900b53476b8f267a94d1d4f` using cumulative BUY flow of at least $100, at least 80% directional dominance, a source price from 40c through 55c, and at least 90 seconds remaining.

The rules are historical candidates, not guarantees. The executable must emit this warning at startup.

## Architecture

1. **Poller**: concurrent wallet requests using one persistent HTTP client. Default cadence is 250 ms per wallet-equivalent cycle, globally bounded and backed off on failure.
2. **Normalizer**: parses public trade JSON, validates the target wallet, supports only BUY transactions and accepted crypto slugs, derives market end time, and creates a stable deduplication key.
3. **Per-wallet watcher**: independently primes historical state, deduplicates fills, and prevents a failed first poll from turning history into a false new signal after recovery.
4. **Strategy engine**: maintains per-wallet/per-market state and emits a candidate at most once per rule and market.
5. **Consensus coordinator**: collapses agreeing wallets into one position and skips same-cycle conflicts.
6. **Risk arbiter**: rejects stale signals, duplicate markets, conflicting outcomes, excessive daily capital at risk, and concurrent exposure.
7. **Sizer**: computes Crypto taker fee using `shares * 0.07 * p * (1-p)`, applies quarter-Kelly sizing with a configurable probability estimate, and caps capital at 5% per trade by default.
8. **Executor interface**: paper executor records deterministic simulated fills. Live executor is behind the `live-trading` Cargo feature and uses Polymarket's official `polymarket_client_sdk_v2` against the V2 CLOB.
9. **Journal**: append-only JSON Lines output written after accepted signals, fills, and rejections; secrets are never represented in journal record types.

## Live safety gates

Live order submission requires all of the following:

- binary compiled with `--features live-trading`;
- `--mode live` supplied;
- `POLYMARKET_LIVE_ACK=I_UNDERSTAND_REAL_MONEY` exactly;
- `POLYMARKET_PRIVATE_KEY` present in the process environment;
- no more than one open market at a time by default;
- source-price deterioration no greater than 1 cent by default and never more than 2 cents;
- an FOK marketable limit BUY with a hard maximum price; no unbounded order.

The private key must never be printed, persisted, passed through CLI arguments, or included in panic output.

## Failure handling

- Network failures trigger bounded exponential polling backoff up to five seconds.
- Invalid JSON, unsupported markets, missing fields, and invalid transactions are rejected without creating an order.
- Journal failure terminates the process rather than trading without an audit trail.
- Duplicate transactions are ignored in a bounded per-wallet cache.
- Any same-cycle conflicting outcome in the same market causes a skip.
- The first successful snapshot for every wallet is priming-only.

## Verification

Testing uses TDD and covers:

- slug parsing and end-time derivation;
- fee and quarter-Kelly sizing boundaries;
- each strategy's exact trigger and non-trigger cases;
- transaction deduplication and recovery-safe priming;
- same-market conflict handling and priority;
- daily-risk and one-position circuit breakers;
- paper execution accounting;
- append-only journal serialization and secret-field absence;
- Data API query contracts and HTTP failure propagation;
- property tests for fee and risk invariants;
- compile checks with and without `live-trading`;
- Clippy with warnings denied;
- Rustfmt and release builds;
- public paper-mode connectivity smoke test.

No test sends a real order. The live executor compile test authenticates nothing and performs no network action.
