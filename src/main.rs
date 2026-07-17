mod app;
mod app_support;

use std::path::PathBuf;

use anyhow::Result;
use app::{AppConfig, RunMode};
use clap::{Parser, ValueEnum};
use polymarket_copybot::{DEFAULT_CLOB_API_BASE, validate_live_ack};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tracing_subscriber::EnvFilter;

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
enum Mode {
    Paper,
    Live,
}

impl From<Mode> for RunMode {
    fn from(value: Mode) -> Self {
        match value {
            Mode::Paper => Self::Paper,
            Mode::Live => Self::Live,
        }
    }
}

#[derive(Debug, Parser)]
#[command(version, about = "Paper-first Polymarket wallet discovery and copybot")]
struct Args {
    #[arg(long, value_enum, default_value = "paper")]
    mode: Mode,
    #[arg(long, default_value_t = false)]
    auto_live: bool,
    #[arg(long, default_value = "95")]
    bankroll: Decimal,
    #[arg(long, default_value = "0.05")]
    max_risk_fraction: Decimal,
    #[arg(long, default_value = "9.50")]
    max_daily_capital_at_risk: Decimal,
    #[arg(long, default_value = "0.01")]
    max_slippage: Decimal,
    #[arg(long, default_value = "0.01")]
    paper_slippage: Decimal,
    #[arg(long, default_value_t = 250)]
    poll_ms: u64,
    #[arg(long, default_value_t = 100)]
    fetch_limit: usize,
    #[arg(long, default_value_t = 1000)]
    replay_limit: usize,
    #[arg(long, default_value_t = 900_000)]
    discovery_ms: u64,
    #[arg(long, default_value_t = 50)]
    leaderboard_limit: usize,
    #[arg(long, default_value_t = 2)]
    discovery_concurrency: usize,
    #[arg(long, default_value_t = 60_000)]
    preflight_ms: u64,
    #[arg(long, default_value = "https://data-api.polymarket.com")]
    data_api_base: String,
    #[arg(long, default_value = "https://gamma-api.polymarket.com")]
    gamma_api_base: String,
    #[arg(long, default_value = DEFAULT_CLOB_API_BASE)]
    clob_api_base: String,
    #[arg(long, default_value = "state/wallet-registry.json")]
    registry: PathBuf,
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
    validate_live_ack(
        args.mode == Mode::Live && args.auto_live,
        args.live_ack.as_deref(),
    )?;
    app::run(AppConfig {
        mode: args.mode.into(),
        auto_live: args.auto_live,
        bankroll: args.bankroll,
        max_risk_fraction: args.max_risk_fraction,
        max_daily_capital_at_risk: args.max_daily_capital_at_risk,
        max_slippage: args.max_slippage,
        paper_slippage: args.paper_slippage,
        poll_ms: args.poll_ms,
        fetch_limit: args.fetch_limit,
        replay_limit: args.replay_limit,
        discovery_ms: args.discovery_ms,
        leaderboard_limit: args.leaderboard_limit,
        discovery_concurrency: args.discovery_concurrency,
        preflight_ms: args.preflight_ms,
        data_api_base: args.data_api_base,
        gamma_api_base: args.gamma_api_base,
        clob_api_base: args.clob_api_base,
        registry: args.registry,
        journal: args.journal,
        once: args.once,
    })
    .await
}

fn validate(args: &Args) -> Result<()> {
    if args.poll_ms < 250 {
        anyhow::bail!("poll-ms must be at least 250 for the four-wallet rate budget");
    }
    if args.fetch_limit == 0 || args.fetch_limit > 1000 {
        anyhow::bail!("fetch-limit must be 1..=1000");
    }
    if args.replay_limit == 0 || args.replay_limit > 1000 {
        anyhow::bail!("replay-limit must be 1..=1000");
    }
    if args.discovery_ms < 60_000 {
        anyhow::bail!("discovery-ms must be at least 60000");
    }
    if args.preflight_ms < 30_000 {
        anyhow::bail!("preflight-ms must be at least 30000");
    }
    if args.leaderboard_limit == 0 || args.leaderboard_limit > 50 {
        anyhow::bail!("leaderboard-limit must be 1..=50");
    }
    if args.discovery_concurrency == 0 || args.discovery_concurrency > 4 {
        anyhow::bail!("discovery-concurrency must be 1..=4");
    }
    if args.bankroll <= Decimal::ZERO {
        anyhow::bail!("bankroll must be positive");
    }
    if args.max_risk_fraction <= Decimal::ZERO || args.max_risk_fraction > dec!(0.05) {
        anyhow::bail!("max-risk-fraction must be in (0,0.05]");
    }
    if args.max_daily_capital_at_risk <= Decimal::ZERO
        || args.max_daily_capital_at_risk > args.bankroll * dec!(0.10)
    {
        anyhow::bail!(
            "max-daily-capital-at-risk must be positive and no more than 10% of bankroll"
        );
    }
    if args.max_slippage < Decimal::ZERO || args.max_slippage > dec!(0.02) {
        anyhow::bail!("max-slippage must be 0..=0.02");
    }
    if args.paper_slippage < Decimal::ZERO || args.paper_slippage > dec!(0.25) {
        anyhow::bail!("paper-slippage must be 0..=0.25");
    }
    if args.data_api_base.trim().is_empty()
        || args.gamma_api_base.trim().is_empty()
        || args.clob_api_base.trim().is_empty()
    {
        anyhow::bail!("API base URLs cannot be empty");
    }
    match (args.mode, args.auto_live) {
        (Mode::Paper, true) => anyhow::bail!("--auto-live requires --mode live"),
        (Mode::Live, false) => anyhow::bail!("--mode live requires --auto-live"),
        _ => {}
    }
    Ok(())
}
