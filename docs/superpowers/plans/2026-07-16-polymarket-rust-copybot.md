# Polymarket Rust Copybot Implementation Plan

**Goal:** Build and rigorously test a low-latency, paper-first Rust copy-trading bot for selected Polymarket short-duration Crypto wallets.

**Architecture:** A Tokio binary polls public wallet trades, normalizes and deduplicates events per wallet, applies wallet-specific state machines, collapses same-market agreement into one candidate, passes candidates through a risk arbiter and quarter-Kelly sizing engine, journals every decision, and dispatches to a paper executor or an explicitly gated live V2 CLOB executor.

**Tech stack:** Rust 1.88, Tokio, Reqwest, Serde, Clap, Rust Decimal, Tracing, Proptest, Wiremock, and optional `polymarket_client_sdk_v2` 0.6.

## Constraints

- Paper mode is the default and no test sends a real order.
- Supported slugs are only `btc|eth|sol|xrp-updown-(5m|15m)-<10-digit epoch>`.
- Default maximum capital per trade is 5% and default maximum slippage is 1 cent.
- Live mode requires a compile-time feature, explicit runtime mode, and exact acknowledgement environment variable.
- Secrets are environment-only and never logged or persisted.
- One open market at a time and a daily capital-at-risk circuit breaker are enabled by default.

## Tasks

1. Define exact-decimal trade, market, outcome, signal, error, and transaction-key primitives.
2. Implement and property-test Crypto fees and capped quarter-Kelly sizing.
3. Implement first-large-buy and confirmed-flow wallet state machines.
4. Implement bounded deduplication, recovery-safe per-wallet priming, and risk arbitration.
5. Implement same-cycle consensus, conflict rejection, and wallet priority.
6. Implement deterministic paper execution and append-only typed JSONL journal records.
7. Implement persistent public Data API polling, sorting, timeout handling, and adaptive backoff.
8. Implement CLI orchestration with paper defaults, one-position tracking, shutdown-safe FOK behavior, and startup warnings.
9. Compile-gate the official V2 live executor and require the exact runtime acknowledgement plus environment-only private key.
10. Run Rustfmt, default and all-feature tests, property tests, Clippy with warnings denied, release builds, and a public paper-mode smoke test.
11. Record final hashes and exact verification evidence in `VERIFICATION.md` and package a source archive.
