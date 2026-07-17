use std::{
    collections::{HashMap, HashSet},
    env,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use futures::future::join_all;
use polymarket_copybot::{
    BoundedDedupe, CandidateSignal, CopybotError, DataApiClient, ExecutionRequest, Executor,
    PaperExecutor, PositionSizer, RiskArbiter, RiskConfig, SizingConfig, StrategyConfig,
    StrategyEngine, trade_key, validate_live_ack,
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

const PRIMARY: &str = "0x208326efd5d051c59631ed626848b150b8d8259c";
const SECONDARY: &str = "0x45230b4fb12569efcc908b4d22c3cee4a19429e2";
const SPECIALIST: &str = "0xb89d0b6e96e790afa900b53476b8f267a94d1d4f";

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
    let mut engines = strategy_engines();
    let wallets: Vec<String> = engines.keys().cloned().collect();
    let mut dedupe = BoundedDedupe::new(20_000)?;
    let mut risk = RiskArbiter::new(RiskConfig {
        minimum_lead_seconds: 90,
        max_open_markets: 1,
        max_daily_capital_at_risk: args.max_daily_capital_at_risk,
    })?;
    let executor = executor(&args).await?;
    let mut primed = false;
    let mut open: Option<(String, i64)> = None;

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
        for (wallet, result) in wallets.iter().zip(results) {
            match result {
                Ok(trades) => {
                    let engine = engines.get_mut(wallet).expect("wallet has engine");
                    for trade in trades {
                        if !dedupe.insert(trade_key(&trade)) {
                            continue;
                        }
                        match engine.ingest(&trade) {
                            Ok(Some(signal)) if primed => candidates.push(signal),
                            Ok(_) | Err(CopybotError::InvalidSlug(_)) => {}
                            Err(error) => warn!(wallet, %error, "trade rejected"),
                        }
                    }
                }
                Err(error) => error!(wallet, %error, "wallet poll failed"),
            }
        }
        if !primed {
            primed = true;
            info!("baseline primed; old fills will never be copied");
        } else if let Some(signal) = select_signal(candidates) {
            match risk.reserve(&signal, now) {
                Ok(()) => {
                    let maximum_price =
                        (signal.source_price + args.max_slippage).min(dec!(0.99));
                    let sizing = PositionSizer::new(SizingConfig {
                        bankroll: args.bankroll,
                        estimated_win_probability: signal.estimated_win_probability,
                        kelly_multiplier: dec!(0.25),
                        max_bankroll_fraction: args.max_risk_fraction,
                        minimum_shares: dec!(5),
                    })?
                    .size(maximum_price);
                    match sizing {
                        Ok(size) => {
                            if let Err(reason) = risk.record_capital_at_risk(
                                &signal.condition_id,
                                size.total_cost,
                            ) {
                                warn!(?reason, "daily risk gate rejected signal");
                            } else {
                                let end = signal.market_end_epoch;
                                let condition = signal.condition_id.clone();
                                let request = ExecutionRequest {
                                    signal,
                                    shares: size.shares,
                                    maximum_price,
                                };
                                match executor.execute(request).await {
                                    Ok(fill) => {
                                        info!(
                                            paper = fill.paper,
                                            condition = %fill.condition_id,
                                            outcome = %fill.outcome,
                                            shares = %fill.shares,
                                            price = %fill.fill_price,
                                            total_cost = %fill.total_cost,
                                            "order filled"
                                        );
                                        open = Some((fill.condition_id, end));
                                    }
                                    Err(error) => {
                                        risk.release(&condition);
                                        error!(%error, "execution failed");
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            risk.release(&signal.condition_id);
                            warn!(%error, "sizing rejected signal");
                        }
                    }
                }
                Err(reason) => info!(?reason, "risk gate skipped signal"),
            }
        }

        if args.once {
            break;
        }
        tokio::time::sleep(Duration::from_millis(args.poll_ms)).await;
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

fn strategy_engines() -> HashMap<String, StrategyEngine> {
    [
        StrategyConfig::FirstLargeBuy {
            wallet: PRIMARY.into(),
            minimum_notional: dec!(25),
            minimum_lead_seconds: 90,
            estimated_win_probability: dec!(0.515),
        },
        StrategyConfig::FirstLargeBuy {
            wallet: SECONDARY.into(),
            minimum_notional: dec!(25),
            minimum_lead_seconds: 90,
            estimated_win_probability: dec!(0.50),
        },
        StrategyConfig::ConfirmedFlow {
            wallet: SPECIALIST.into(),
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
        (
            config.wallet().to_owned(),
            StrategyEngine::new(config),
        )
    })
    .collect()
}

fn select_signal(signals: Vec<CandidateSignal>) -> Option<CandidateSignal> {
    let mut groups: HashMap<String, Vec<CandidateSignal>> = HashMap::new();
    for signal in signals {
        groups
            .entry(signal.condition_id.clone())
            .or_default()
            .push(signal);
    }
    let mut groups: Vec<_> = groups.into_values().collect();
    groups.sort_by_key(|rows| {
        rows.iter()
            .map(|signal| signal.source_timestamp)
            .min()
            .unwrap_or(i64::MAX)
    });
    for mut rows in groups {
        let outcomes: HashSet<_> = rows.iter().map(|signal| signal.outcome).collect();
        if outcomes.len() != 1 {
            warn!(condition = %rows[0].condition_id, "wallet conflict; market skipped");
            continue;
        }
        rows.sort_by_key(|signal| wallet_priority(&signal.wallet));
        return rows.into_iter().next();
    }
    None
}

fn wallet_priority(wallet: &str) -> u8 {
    if wallet.eq_ignore_ascii_case(PRIMARY) {
        0
    } else if wallet.eq_ignore_ascii_case(SECONDARY) {
        1
    } else {
        2
    }
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
