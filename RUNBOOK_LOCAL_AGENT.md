# Local Agent Runbook — Polymarket Rust Copybot

Use this document as the authoritative local setup and launch procedure for branch `polymarket-copybot-final-20260716-done`.

The bot is experimental trading software. It does not guarantee profit and can lose money. Start in paper mode. Do not enable live mode merely because the program compiles.

## Ready-to-paste instruction for the local agent

```text
Work in the existing repository Chrisbryan17/ai-caller-bare on branch polymarket-copybot-final-20260716-done.

Goal: verify and run the Rust Polymarket copybot locally in paper mode with a $95 modeled bankroll. Do not place a real order. Do not request, print, log, commit, or transmit any private key, seed phrase, API secret, or credential.

1. Inspect README.md, RUNBOOK_LOCAL_AGENT.md, Cargo.toml, .env.example, and .gitignore.
2. Confirm Git and rustup are installed. Install and select Rust 1.88.0 with rustfmt and clippy when missing.
3. Check out the exact branch and ensure the working tree is clean before making changes.
4. Run the complete verification matrix exactly as written in this runbook. Stop and report the first failure with the full command and relevant output. Do not weaken, skip, or suppress tests or lints.
5. Build the default release binary only.
6. Create the local state directory. Run the reduced public paper smoke command. Confirm it exits successfully and creates the registry and JSONL journal.
7. Run the long-lived paper command. Confirm the process starts, public discovery runs, wallets are primed without copying historical fills, and state persists across one controlled Ctrl+C restart.
8. Report: current commit SHA, every verification command and result, smoke-created files, active/quarantined wallet summary from the registry, and the exact command used for the long-lived paper process.
9. Do not enable --mode live, --auto-live, the live-trading feature, or POLYMARKET_LIVE_ACK unless Christopher separately directs you to perform the live procedure.
10. Do not alter the fixed risk limits: bankroll 95, max risk fraction 0.05, daily capital-at-risk 9.50, one open market, and maximum slippage 0.01.
```

## 1. Clone or update the repository

### Windows PowerShell

```powershell
git clone https://github.com/Chrisbryan17/ai-caller-bare.git
Set-Location ai-caller-bare
git fetch --all --prune
git checkout polymarket-copybot-final-20260716-done
git pull --ff-only origin polymarket-copybot-final-20260716-done
git status --short
```

When the repository already exists, start at `Set-Location`, then fetch, check out, pull, and inspect status.

### Linux, WSL, or macOS

```bash
git clone https://github.com/Chrisbryan17/ai-caller-bare.git
cd ai-caller-bare
git fetch --all --prune
git checkout polymarket-copybot-final-20260716-done
git pull --ff-only origin polymarket-copybot-final-20260716-done
git status --short
```

Do not discard uncommitted user work. Stop and report it before switching branches or resetting files.

## 2. Pin the required Rust toolchain

```powershell
rustup toolchain install 1.88.0 --component rustfmt,clippy
rustup override set 1.88.0
rustc --version
cargo --version
```

Expected Rust compiler family: `rustc 1.88.x`.

## 3. Run the complete verification matrix

Run every command. A warning promoted by `-D warnings` is a failure.

```powershell
cargo fmt --all -- --check
cargo test --all-targets
cargo test --all-targets --all-features
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
cargo build --release
cargo build --release --all-features
```

The all-features commands compile the separately gated live connector but do not place an order. No credentials are needed for tests or builds.

## 4. Run a reduced public paper smoke test

### Windows PowerShell

```powershell
New-Item -ItemType Directory -Force state | Out-Null
.\target\release\polymarket-copybot.exe `
  --mode paper `
  --once `
  --bankroll 95 `
  --max-risk-fraction 0.05 `
  --max-daily-capital-at-risk 9.50 `
  --leaderboard-limit 1 `
  --replay-limit 100 `
  --discovery-concurrency 1 `
  --registry state\smoke-registry.json `
  --journal state\smoke.jsonl
```

### Linux, WSL, or macOS

```bash
mkdir -p state
./target/release/polymarket-copybot \
  --mode paper \
  --once \
  --bankroll 95 \
  --max-risk-fraction 0.05 \
  --max-daily-capital-at-risk 9.50 \
  --leaderboard-limit 1 \
  --replay-limit 100 \
  --discovery-concurrency 1 \
  --registry state/smoke-registry.json \
  --journal state/smoke.jsonl
```

Confirm these exist after the smoke run:

- `state/smoke-registry.json`
- `state/smoke-registry.runtime.json`
- `state/smoke.jsonl`

The smoke run may reject every candidate. That is valid. It must fail closed rather than inventing a qualified wallet.

## 5. Start the long-lived paper process

### Windows PowerShell

```powershell
$env:RUST_LOG = "info"
.\target\release\polymarket-copybot.exe `
  --mode paper `
  --bankroll 95 `
  --max-risk-fraction 0.05 `
  --max-daily-capital-at-risk 9.50 `
  --max-slippage 0.01 `
  --paper-slippage 0.01 `
  --poll-ms 250 `
  --discovery-ms 900000 `
  --leaderboard-limit 50 `
  --replay-limit 1000 `
  --discovery-concurrency 2 `
  --registry state\wallet-registry.json `
  --journal state\copybot.jsonl
```

### Linux, WSL, or macOS

```bash
RUST_LOG=info ./target/release/polymarket-copybot \
  --mode paper \
  --bankroll 95 \
  --max-risk-fraction 0.05 \
  --max-daily-capital-at-risk 9.50 \
  --max-slippage 0.01 \
  --paper-slippage 0.01 \
  --poll-ms 250 \
  --discovery-ms 900000 \
  --leaderboard-limit 50 \
  --replay-limit 1000 \
  --discovery-concurrency 2 \
  --registry state/wallet-registry.json \
  --journal state/copybot.jsonl
```

Expected behavior:

- DAY, WEEK, and MONTH Crypto leaderboards are discovered outside the hot polling loop.
- At least two leaderboard periods must succeed before candidate state can change.
- Historical trades are replayed deterministically with fees and adverse slippage.
- A newly accepted wallet enters `quarantined` status and remains paper-only.
- Its first successful live snapshot only primes its dedupe state; historical fills are not copied.
- At most four wallets are polled at 250 ms, preserving public API rate-limit headroom.
- At most three qualified, non-duplicative wallets can enter the active set.
- Same-market conflicts are skipped and agreement never multiplies position size.
- State and pending paper positions survive a controlled restart.

Press `Ctrl+C` once. Restart with the identical command and confirm the same registry is restored and every watcher primes before emitting new signals.

## 6. Inspect evidence

PowerShell examples:

```powershell
Get-Content state\copybot.jsonl -Tail 50
Get-Content state\wallet-registry.json
Get-Content state\wallet-registry.runtime.json
```

Useful journal record kinds include:

- `discovery_cycle_completed`
- `candidate_evaluated`
- `wallet_lifecycle_changed`
- `active_set_changed`
- `signal`
- `fill`
- `paper_resolved`
- `rejection`

Do not edit the persisted JSON while the process is running.

## 7. Live procedure — only after separate explicit authorization

The current live connector supports a direct EOA signer only. It does not configure a Polymarket proxy wallet, Gnosis Safe, or Poly1271 signer. The EOA must be funded and have the required token allowances. Use a dedicated limited-funds wallet, not a primary savings wallet.

Never paste the private key into chat, a ticket, source code, a command-line argument, the journal, or Git history. Do not store it in the repository `.env` file.

Build the live-enabled release:

```powershell
cargo build --release --all-features
```

Set secrets only in the local process environment. In PowerShell, read the key without echoing it:

```powershell
$secureKey = Read-Host "Dedicated EOA private key" -AsSecureString
$keyPtr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secureKey)
try {
  $env:POLYMARKET_PRIVATE_KEY = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($keyPtr)
} finally {
  [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($keyPtr)
}
$env:POLYMARKET_LIVE_ACK = "I_UNDERSTAND_REAL_MONEY"
$env:RUST_LOG = "info"
```

Then run:

```powershell
.\target\release\polymarket-copybot.exe `
  --mode live `
  --auto-live `
  --bankroll 95 `
  --max-risk-fraction 0.05 `
  --max-daily-capital-at-risk 9.50 `
  --registry state\wallet-registry.json `
  --journal state\copybot.jsonl
```

`--mode live --auto-live` does **not** immediately authorize an order. The route remains paper-only until all of these pass:

- the wallet passed historical replay;
- at least five forward paper positions resolved over at least 60 minutes;
- forward paper net P&L is positive;
- drawdown, consecutive-loss, and processing-health gates pass;
- the wallet is in the qualified active set;
- the global forward-paper launch gate passes;
- the latest authenticated no-order preflight confirms trading access, sufficient balance, positive allowance, an authenticated signer, and no open orders;
- the live acknowledgement is exact;
- the live feature was compiled.

A failed preflight must remain failed closed. Do not bypass geoblocking, closed-only status, allowance checks, or any safety gate.

Clear the key from the process environment after stopping:

```powershell
Remove-Item Env:POLYMARKET_PRIVATE_KEY -ErrorAction SilentlyContinue
Remove-Item Env:POLYMARKET_LIVE_ACK -ErrorAction SilentlyContinue
```

## 8. Local agent completion report

The agent should return:

1. Current branch and commit SHA.
2. Rust and Cargo versions.
3. A pass/fail line for every verification command.
4. Paper smoke exit status and created file paths.
5. Registry summary: discovered, rejected, quarantined, paper-qualified, and active wallets.
6. Confirmation that the controlled restart did not replay historical signals.
7. The exact long-lived paper command now running.
8. Any blocker, including network/API errors, geoblocking, missing allowances, or unsupported wallet type.

It must not include any private key, seed phrase, API credential, authentication header, or full secret-bearing environment dump.
