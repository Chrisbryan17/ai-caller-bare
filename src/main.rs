use std::{
    collections::HashMap,
    env,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use futures::future::join_all;
use polymarket_copybot::{
    CandidateSignal, DataApiClient, ExecutionRequest, Executor, JournalRecord, JsonlJournal,
    PRIMARY_WALLET, PaperExecutor, PositionSizer, RiskArbiter, RiskConfig, SECONDARY_WALLET,
    SPECIALIST_WALLET, SizingConfig, StrategyConfig, StrategyEngine, WalletWatcher, select_signal,
    validate_live_ack,
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum Mode {
    Paper,
    Live,
}

#[derive(Debug, Parser)]
#[command(version, about = "Paper-first Polymarket wallet copybot")]
struct Args {
    #[arg(long, value_enum, default_value = "paper")]
    mode: Mode,
    #[arg(long, default_value = "125")]
    bankroll: Decimal,
    #[arg(long, default_value = "0.05")]
    max_risk_fraction: Decimal,
    #[arg(long, default_value = "12")]
    max_daily_capital_at_risk: Decimal,
    #[arg(long, default_value = "0.01")]
    max_slippage: Decimal,
    #[arg(long, default_value = "0.01")]
    paper_slippage: Decimal,
    #[arg(long, default_value_t = 250)]
    poll_ms: u64,
    #[arg(long, default_value_t = 100)]
    fetch_limit: usize,
    #[arg(long, default_value = "https://data-api.polymarket.com")]
    data_api_base: String,
    #[arg(long, default_value = "copybot.jsonl")]
    journal: PathBuf,
    #[arg(long, env = "POLYMARKET_LIVE_ACK")]
    live_ack: Option<String>,
    #[arg(long)]
    once: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();
    validate(&args)?;
    validate_live_ack(args.mode == Mode::Live, args.live_ack.as_deref())?;

    warn!(
        mode = ?args.mode,
        "No strategy guarantees profit; live mode can lose the full amount placed"
    );
    let api = DataApiClient::new(args.data_api_base.clone(), Duration::from_secs(3))?;
    let mut watchers = strategy_watchers()?;
    let wallets: Vec<String> = watchers.keys().cloned().collect();
    let journal = JsonlJournal::open(&args.journal).await?;
    let mut risk = RiskArbiter::new(RiskConfig {
        minimum_lead_seconds: 90,
        max_open_markets: 1,
        max_daily_capital_at_risk: args.max_daily_capital_at_risk,
    })?;
    let executor = executor(&args).await?;
    let mut open: Option<(String, i64)> = None;
    let mut failure_streak = 0_u32;

    loop {
        let now = epoch();
        if let Some((condition, end)) = &open {
            if now >= *end {
                risk.release(condition);
                open = None;
            }
        }

        let requests = wallets
            .iter()
            .map(|wallet| api.fetch_trades(wallet, args.fetch_limit));
        let results = join_all(requests).await;
        let mut candidates = Vec::new();
        let mut had_poll_failure = false;
        for (wallet, result) in wallets.iter().zip(results) {
            match result {
                Ok(trades) => {
                    let watcher = watchers.get_mut(wallet).expect("wallet has watcher");
                    let was_primed = watcher.is_primed();
                    match watcher.process_snapshot(trades) {
                        Ok(signals) => {
                            if !was_primed {
                                info!(wallet, "wallet baseline primed; historical fills ignored");
                            }
                            candidates.extend(signals);
                        }
                        Err(error) => {
                            had_poll_failure = true;
                            warn!(wallet, %error, "wallet snapshot rejected");
                        }
                    }
                }
                Err(error) => {
                    had_poll_failure = true;
                    error!(wallet, %error, "wallet poll failed");
                }
            }
        }

        if let Some(signal) = select_signal(candidates) {
            process_signal(
                &args,
                now,
                signal,
                &mut risk,
                executor.as_ref(),
                &journal,
                &mut open,
            )
            .await?;
        }

        if args.once {
            break;
        }
        failure_streak = if had_poll_failure {
            failure_streak.saturating_add(1).min(5)
        } else {
            0
        };
        let multiplier = 1_u64 << failure_streak.min(4);
        let delay_ms = args.poll_ms.saturating_mul(multiplier).min(5_000);
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
    Ok(())
}

async fn process_signal(
    args: &Args,
    now: i64,
    signal: CandidateSignal,
    risk: &mut RiskArbiter,
    executor: &dyn Executor,
    journal: &JsonlJournal,
    open: &mut Option<(String, i64)>,
) -> Result<()> {
    if let Err(reason) = risk.reserve(&signal, now) {
        journal
            .append(&JournalRecord::Rejection {
                observed_epoch: now,
                condition_id: signal.condition_id,
                reason: reason.to_string(),
            })
            .await?;
        return Ok(());
    }

    journal
        .append(&JournalRecord::Signal {
            observed_epoch: now,
            signal: signal.clone(),
        })
        .await?;
    let maximum_price = (signal.source_price + args.max_slippage).min(dec!(0.99));
    let sizing = PositionSizer::new(SizingConfig {
        bankroll: args.bankroll,
        estimated_win_probability: signal.estimated_win_probability,
        kelly_multiplier: dec!(0.25),
        max_bankroll_fraction: args.max_risk_fraction,
        minimum_shares: dec!(5),
    })?
    .size(maximum_price);
    let size = match sizing {
        Ok(size) => size,
        Err(error) => {
            risk.release(&signal.condition_id);
            journal
                .append(&JournalRecord::Rejection {
                    observed_epoch: now,
                    condition_id: signal.condition_id,
                    reason: error.to_string(),
                })
                .await?;
            return Ok(());
        }
    };
    if let Err(reason) = risk.record_capital_at_risk(&signal.condition_id, size.total_cost) {
        journal
            .append(&JournalRecord::Rejection {
                observed_epoch: now,
                condition_id: signal.condition_id,
                reason: reason.to_string(),
            })
            .await?;
        return Ok(());
    }

    let end = signal.market_end_epoch;
    let condition = signal.condition_id.clone();
    let request = ExecutionRequest {
        signal,
        shares: size.shares,
        maximum_price,
    };
    match executor.execute(request).await {
        Ok(fill) => {
            journal
                .append(&JournalRecord::Fill {
                    observed_epoch: epoch(),
                    fill: fill.clone(),
                })
                .await?;
            info!(
                paper = fill.paper,
                condition = %fill.condition_id,
                outcome = %fill.outcome,
                shares = %fill.shares,
                price = %fill.fill_price,
                total_cost = %fill.total_cost,
                "order filled"
            );
            *open = Some((fill.condition_id, end));
        }
        Err(error) => {
            risk.release(&condition);
            journal
                .append(&JournalRecord::Rejection {
                    observed_epoch: epoch(),
                    condition_id: condition,
                    reason: error.to_string(),
                })
                .await?;
            error!(%error, "execution failed");
        }
    }
    Ok(())
}

fn validate(args: &Args) -> Result<()> {
    if args.poll_ms < 150 {
        anyhow::bail!("poll-ms must be at least 150 for three wallets");
    }
    if args.fetch_limit == 0 || args.fetch_limit > 1000 {
        anyhow::bail!("fetch-limit must be 1..=1000");
    }
    if args.max_slippage < Decimal::ZERO || args.max_slippage > dec!(0.02) {
        anyhow::bail!("max-slippage must be 0..=0.02");
    }
    Ok(())
}

fn strategy_watchers() -> Result<HashMap<String, WalletWatcher>> {
    [
        StrategyConfig::FirstLargeBuy {
            wallet: PRIMARY_WALLET.into(),
            minimum_notional: dec!(25),
            minimum_lead_seconds: 90,
            estimated_win_probability: dec!(0.515),
        },
        StrategyConfig::FirstLargeBuy {
            wallet: SECONDARY_WALLET.into(),
            minimum_notional: dec!(25),
            minimum_lead_seconds: 90,
            estimated_win_probability: dec!(0.50),
        },
        StrategyConfig::ConfirmedFlow {
            wallet: SPECIALIST_WALLET.into(),
            minimum_cumulative_notional: dec!(100),
            minimum_directional_share: dec!(0.80),
            minimum_price: dec!(0.40),
            maximum_price: dec!(0.55),
            minimum_lead_seconds: 90,
            estimated_win_probability: dec!(0.65),
        },
    ]
    .into_iter()
    .map(|config| {
        let wallet = config.wallet().to_owned();
        Ok((
            wallet,
            WalletWatcher::new(StrategyEngine::new(config), 10_000)?,
        ))
    })
    .collect()
}

async fn executor(args: &Args) -> Result<Box<dyn Executor>> {
    match args.mode {
        Mode::Paper => Ok(Box::new(PaperExecutor::new(args.paper_slippage)?)),
        Mode::Live => {
            #[cfg(feature = "live-trading")]
            {
                let private_key = env::var("POLYMARKET_PRIVATE_KEY").context(
                    "POLYMARKET_PRIVATE_KEY is required; never paste it into chat",
                )?;
                Ok(Box::new(
                    polymarket_copybot::live::connect_eoa(&private_key).await?,
                ))
            }
            #[cfg(not(feature = "live-trading"))]
            unreachable!("live acknowledgement gate rejects builds without live-trading")
        }
    }
}

fn epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
