# Polymarket Rust Copybot

A paper-first, low-latency Rust service that watches selected public Polymarket wallets and translates their short-duration Crypto trades into small-account signals.

**This is experimental trading software, not a profit guarantee. It can lose money. Paper mode is the default. No test in this repository sends a real order.**

## What it watches

| Wallet | Rule |
|---|---|
| `0x208326efd5d051c59631ed626848b150b8d8259c` | First BUY worth at least $25 with at least 90 seconds remaining |
| `0x45230b4fb12569efcc908b4d22c3cee4a19429e2` | First BUY worth at least $25 with at least 90 seconds remaining |
| `0xb89d0b6e96e790afa900b53476b8f267a94d1d4f` | Cumulative BUY flow at least $100, dominant side at least 80%, source price 40–55c, at least 90 seconds remaining |

Supported market slugs are only BTC, ETH, SOL, and XRP `5m` or `15m` Up/Down markets.

## Safety model

- Paper mode by default.
- Historical fills from the first successful snapshot of **each wallet** are used only to prime state and are never copied.
- Exact-decimal fee and sizing arithmetic; no binary floating point in the money path.
- Quarter-Kelly sizing capped at 5% of the configured bankroll by default.
- One open market at a time.
- Daily capital-at-risk circuit breaker.
- Same-cycle conflicting wallet outcomes are skipped.
- Maximum source-price deterioration defaults to 1 cent and cannot be configured above 2 cents.
- Live mode requires both a Cargo feature and the exact acknowledgement string.
- Private keys are environment-only and are never written to the journal.

## Requirements

- Rust 1.88 or newer.
- A stable internet connection.
- For live EOA trading: a separately funded wallet with the required Polymarket token allowances. Never paste its private key into chat, source code, a command-line argument, or a log file.

## Run safely in paper mode

```bash
cargo run --release -- \
  --mode paper \
  --bankroll 125 \
  --max-risk-fraction 0.05 \
  --max-daily-capital-at-risk 12 \
  --poll-ms 250
```

On Windows PowerShell:

```powershell
cargo run --release -- --mode paper --bankroll 125 --max-risk-fraction 0.05 --max-daily-capital-at-risk 12 --poll-ms 250
```

The first successful request for each wallet only primes its history. Accepted signals, rejections, and paper fills are appended to `copybot.jsonl`.

A connectivity smoke test that makes one public polling cycle and then exits:

```bash
cargo run --release -- --mode paper --once
```

## Live mode

Live support is excluded from the default binary. Build it explicitly:

```bash
cargo build --release --features live-trading
```

Set secrets in the process environment, not in shell history where possible:

```bash
export POLYMARKET_PRIVATE_KEY='...'
export POLYMARKET_LIVE_ACK='I_UNDERSTAND_REAL_MONEY'
./target/release/polymarket-copybot \
  --mode live \
  --bankroll 125 \
  --max-risk-fraction 0.05 \
  --max-daily-capital-at-risk 12
```

The live executor uses Polymarket's official V2 Rust SDK, the V2 CLOB host, an FOK marketable limit BUY, and an explicit maximum price. It does not bypass geoblocking or platform restrictions.

## Emergency stop

Press `Ctrl+C` or terminate the process. The bot submits only FOK orders, so an unfilled order is cancelled rather than resting. Inspect `copybot.jsonl` and the Polymarket account before restarting.

## Verification commands

```bash
cargo fmt --all -- --check
cargo test --all-targets
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
cargo build --release --all-features
```

The CI matrix also runs a public paper-mode smoke test. It never supplies a private key or live acknowledgement.

## Important limitations

- Public wallet trades may appear after the whale's actual execution. Rust cannot remove upstream indexing latency.
- A wallet's reported trade price is not a guarantee that the same liquidity remains available.
- Historical wallet performance can decay or reverse.
- The configured win probabilities are research estimates, not facts.
- This version tracks capital at risk conservatively but does not yet reconcile resolved P&L into a dynamically changing bankroll.
- Run paper mode long enough to measure real detection delay, missed fills, and live slippage before considering live execution.
