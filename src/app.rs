use std::{collections::HashMap, path::PathBuf, time::Duration};

#[cfg(feature = "live-trading")]
use std::sync::Arc;

use anyhow::Result;
use futures::future::join_all;
use polymarket_copybot::{
    DataApiClient, DepthAwarePaperExecutor, DiscoveryApiClient, DiscoveryConfig,
    DiscoveryCoordinator, DiscoveryCycleResult, ExecutionRoute, JsonlJournal, MarketResolution,
    PRIMARY_WALLET, PreflightStatus, RiskArbiter, RiskConfig, RotationRuntime, SECONDARY_WALLET,
    SPECIALIST_WALLET, WalletLifecycle, execution_route, select_signal_with_priority,
    utc_day_index,
};
use rust_decimal::Decimal;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

#[cfg(feature = "live-trading")]
use polymarket_copybot::live::{LiveTradingExecutor, connect_eoa_at};

#[cfg(feature = "live-trading")]
use crate::app_support::log_preflight;

use crate::app_support::{
    ActiveSignalContext, apply_discovery_result, apply_paper_resolutions, epoch, millis_to_seconds,
    process_active_signal, process_shadow_signal, retry_after_map,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunMode {
    Paper,
    Live,
}

#[derive(Clone, Debug)]
pub(crate) struct AppConfig {
    pub mode: RunMode,
    pub auto_live: bool,
    pub bankroll: Decimal,
    pub max_risk_fraction: Decimal,
    pub max_daily_capital_at_risk: Decimal,
    pub max_slippage: Decimal,
    pub paper_slippage: Decimal,
    pub poll_ms: u64,
    pub fetch_limit: usize,
    pub replay_limit: usize,
    pub discovery_ms: u64,
    pub leaderboard_limit: usize,
    pub discovery_concurrency: usize,
    pub preflight_ms: u64,
    pub data_api_base: String,
    pub gamma_api_base: String,
    pub clob_api_base: String,
    pub registry: PathBuf,
    pub journal: PathBuf,
    pub once: bool,
}

pub(crate) async fn run(config: AppConfig) -> Result<()> {
    #[cfg(not(feature = "live-trading"))]
    let _ = config.preflight_ms;
    warn!(
        mode = ?config.mode,
        auto_live = config.auto_live,
        "No strategy guarantees profit; live trading can lose every dollar committed"
    );

    let data_api = DataApiClient::new(config.data_api_base.clone(), Duration::from_secs(4))?;
    let discovery_api = DiscoveryApiClient::new(
        config.data_api_base.clone(),
        config.gamma_api_base.clone(),
        Duration::from_secs(8),
    )?;
    let discovery = DiscoveryCoordinator::new(
        discovery_api.clone(),
        data_api.clone(),
        DiscoveryConfig {
            leaderboard_limit: config.leaderboard_limit,
            trade_limit: config.replay_limit,
            max_concurrency: config.discovery_concurrency,
        },
    )?
    .with_bootstrap_wallets([PRIMARY_WALLET, SECONDARY_WALLET, SPECIALIST_WALLET]);

    let started_epoch = epoch();
    let mut runtime = RotationRuntime::open(&config.registry, config.bankroll, started_epoch)?;
    let journal = JsonlJournal::open(&config.journal).await?;
    let paper_executor = DepthAwarePaperExecutor::new(
        config.clob_api_base.clone(),
        Duration::from_secs(4),
        config.paper_slippage,
    )?;

    #[cfg(feature = "live-trading")]
    let live_executor: Option<Arc<dyn LiveTradingExecutor>> = if config.mode == RunMode::Live {
        let private_key = std::env::var("POLYMARKET_PRIVATE_KEY").map_err(|_| {
            anyhow::anyhow!("POLYMARKET_PRIVATE_KEY is required locally; never paste it into chat")
        })?;
        Some(Arc::from(
            connect_eoa_at(&private_key, &config.clob_api_base).await?,
        ))
    } else {
        None
    };

    #[cfg(feature = "live-trading")]
    let maximum_position_cost = config.bankroll * config.max_risk_fraction;
    #[cfg(feature = "live-trading")]
    let mut preflight = if let Some(executor) = live_executor.as_deref() {
        let status = executor.preflight(maximum_position_cost).await;
        log_preflight(&status);
        status
    } else {
        PreflightStatus::failed("live_mode_disabled")
    };
    #[cfg(not(feature = "live-trading"))]
    let preflight = PreflightStatus::failed("live_trading_not_compiled");

    let (discovery_tx, mut discovery_rx) = mpsc::channel::<DiscoveryCycleResult>(1);
    let (resolution_tx, mut resolution_rx) =
        mpsc::channel::<std::result::Result<HashMap<String, MarketResolution>, String>>(1);
    let mut discovery_in_flight = false;
    let mut resolution_in_flight = false;

    #[cfg(feature = "live-trading")]
    let (preflight_tx, mut preflight_rx) = mpsc::channel::<PreflightStatus>(1);
    #[cfg(feature = "live-trading")]
    let mut preflight_in_flight = false;

    let mut risk_day = utc_day_index(started_epoch);
    let persisted_daily_risk = runtime.live_daily_capital_at_risk(risk_day)?;
    let mut risk = RiskArbiter::new_with_daily(
        RiskConfig {
            minimum_lead_seconds: 90,
            max_open_markets: 1,
            max_daily_capital_at_risk: config.max_daily_capital_at_risk,
        },
        persisted_daily_risk,
    )?;
    let mut open = if let Some(position) = runtime.active_position(started_epoch)? {
        risk.restore_open(&position.condition_id, position.outcome)?;
        info!(
            condition = %position.condition_id,
            end_epoch = position.market_end_epoch,
            "restored persisted open position into risk arbiter"
        );
        Some((position.condition_id, position.market_end_epoch))
    } else {
        None
    };
    let mut failure_streak = 0_u32;
    let mut next_discovery_epoch = 0_i64;
    #[cfg(feature = "live-trading")]
    let mut next_preflight_epoch = started_epoch + millis_to_seconds(config.preflight_ms);

    loop {
        let now = epoch();
        let current_day = utc_day_index(now);
        if current_day != risk_day {
            risk.reset_daily_risk();
            runtime.persist_live_daily_capital_at_risk(current_day, Decimal::ZERO)?;
            risk_day = current_day;
            info!(utc_day = current_day, "daily capital-at-risk budget reset");
        }
        if let Some((condition, end)) = &open
            && now >= *end
        {
            risk.forget_market(condition);
            open = None;
        }

        while let Ok(result) = discovery_rx.try_recv() {
            discovery_in_flight = false;
            apply_discovery_result(
                now,
                result,
                &mut runtime,
                &journal,
                open.is_some(),
                config.mode == RunMode::Live,
            )
            .await?;
        }
        while let Ok(result) = resolution_rx.try_recv() {
            resolution_in_flight = false;
            match result {
                Ok(resolutions) => {
                    apply_paper_resolutions(
                        now,
                        resolutions,
                        &mut runtime,
                        &journal,
                        open.is_some(),
                        config.mode == RunMode::Live,
                    )
                    .await?;
                }
                Err(error) => warn!(%error, "background paper resolution lookup failed"),
            }
        }

        #[cfg(feature = "live-trading")]
        while let Ok(status) = preflight_rx.try_recv() {
            preflight_in_flight = false;
            preflight = status;
            log_preflight(&preflight);
        }

        if !resolution_in_flight {
            let due = runtime.due_condition_ids(now);
            if !due.is_empty() {
                if config.once {
                    match discovery_api.fetch_resolutions(&due).await {
                        Ok(resolutions) => {
                            apply_paper_resolutions(
                                now,
                                resolutions,
                                &mut runtime,
                                &journal,
                                open.is_some(),
                                config.mode == RunMode::Live,
                            )
                            .await?;
                        }
                        Err(error) => warn!(%error, "paper resolution lookup failed"),
                    }
                } else {
                    let api = discovery_api.clone();
                    let tx = resolution_tx.clone();
                    tokio::spawn(async move {
                        let result = api
                            .fetch_resolutions(&due)
                            .await
                            .map_err(|error| error.to_string());
                        let _ = tx.send(result).await;
                    });
                    resolution_in_flight = true;
                }
            }
        }

        #[cfg(feature = "live-trading")]
        if config.mode == RunMode::Live
            && now >= next_preflight_epoch
            && !preflight_in_flight
            && let Some(executor) = live_executor.as_ref()
        {
            let executor = Arc::clone(executor);
            let tx = preflight_tx.clone();
            tokio::spawn(async move {
                let status = executor.preflight(maximum_position_cost).await;
                let _ = tx.send(status).await;
            });
            preflight_in_flight = true;
            next_preflight_epoch = now + millis_to_seconds(config.preflight_ms);
        }

        if now >= next_discovery_epoch && !discovery_in_flight {
            let skip_until = retry_after_map(&runtime);
            if config.once {
                let result = discovery.run_cycle(now, &skip_until).await;
                apply_discovery_result(
                    now,
                    result,
                    &mut runtime,
                    &journal,
                    open.is_some(),
                    config.mode == RunMode::Live,
                )
                .await?;
            } else {
                let coordinator = discovery.clone();
                let tx = discovery_tx.clone();
                tokio::spawn(async move {
                    let result = coordinator.run_cycle(now, &skip_until).await;
                    let _ = tx.send(result).await;
                });
                discovery_in_flight = true;
            }
            next_discovery_epoch = now + millis_to_seconds(config.discovery_ms);
        }

        let wallets = runtime.watched_wallets();
        let requests = wallets
            .iter()
            .map(|wallet| data_api.fetch_trades(wallet, config.fetch_limit));
        let results = join_all(requests).await;
        let mut candidates = Vec::new();
        let mut had_poll_failure = false;
        for (wallet, result) in wallets.iter().zip(results) {
            match result {
                Ok(trades) => {
                    let was_primed = runtime
                        .registry()
                        .record(wallet)
                        .and_then(|record| record.prime_epoch)
                        .is_some();
                    match runtime.process_snapshot(wallet, trades, now) {
                        Ok(signals) => {
                            if !was_primed
                                && runtime
                                    .registry()
                                    .record(wallet)
                                    .and_then(|record| record.prime_epoch)
                                    .is_some()
                            {
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

        let active_wallets = runtime.registry().state().active_wallets.clone();
        let launch_mode = runtime.launch_controller().evaluate(
            now,
            runtime.registry(),
            &active_wallets,
            &preflight,
            config.mode == RunMode::Live && config.auto_live,
        );
        let mut shadow = Vec::new();
        let mut active = Vec::new();
        for signal in candidates {
            let Some(record) = runtime.registry().record(&signal.wallet) else {
                continue;
            };
            let is_active = active_wallets
                .iter()
                .any(|wallet| wallet.eq_ignore_ascii_case(&signal.wallet));
            match execution_route(record.lifecycle, is_active, launch_mode, config.auto_live) {
                ExecutionRoute::ShadowPaper => shadow.push(signal),
                ExecutionRoute::ActivePaper | ExecutionRoute::Live => active.push(signal),
                ExecutionRoute::Ignore => {}
            }
        }

        for signal in shadow {
            process_shadow_signal(
                &config,
                now,
                signal,
                &paper_executor,
                &journal,
                &mut runtime,
            )
            .await?;
        }

        if let Some(signal) = select_signal_with_priority(active, &active_wallets) {
            let lifecycle = runtime
                .registry()
                .record(&signal.wallet)
                .map(|record| record.lifecycle)
                .unwrap_or(WalletLifecycle::Rejected);
            let route = execution_route(lifecycle, true, launch_mode, config.auto_live);
            match route {
                ExecutionRoute::ActivePaper => {
                    process_active_signal(
                        now,
                        signal,
                        &paper_executor,
                        ActiveSignalContext {
                            config: &config,
                            risk: &mut risk,
                            journal: &journal,
                            runtime: &mut runtime,
                            open: &mut open,
                            live_execution: false,
                        },
                    )
                    .await?;
                }
                ExecutionRoute::Live => {
                    #[cfg(feature = "live-trading")]
                    if let Some(executor) = live_executor.as_deref() {
                        process_active_signal(
                            now,
                            signal,
                            executor,
                            ActiveSignalContext {
                                config: &config,
                                risk: &mut risk,
                                journal: &journal,
                                runtime: &mut runtime,
                                open: &mut open,
                                live_execution: true,
                            },
                        )
                        .await?;
                    }
                    #[cfg(not(feature = "live-trading"))]
                    unreachable!("live acknowledgement gate rejects builds without live-trading");
                }
                ExecutionRoute::Ignore | ExecutionRoute::ShadowPaper => {}
            }
        }

        if config.once {
            break;
        }
        failure_streak = if had_poll_failure {
            failure_streak.saturating_add(1).min(5)
        } else {
            0
        };
        let multiplier = 1_u64 << failure_streak.min(4);
        let delay_ms = config.poll_ms.saturating_mul(multiplier).min(5_000);
        tokio::select! {
            () = tokio::time::sleep(Duration::from_millis(delay_ms)) => {}
            signal = tokio::signal::ctrl_c() => {
                signal?;
                info!("shutdown requested; persisted runtime state retained");
                break;
            }
        }
    }
    Ok(())
}
