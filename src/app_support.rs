use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use polymarket_copybot::{
    CandidateEvaluation, CandidateSignal, DiscoveryCycleResult, ExecutionRequest, Executor,
    JournalRecord, JsonlJournal, MarketResolution, PositionSizer, PreflightStatus, RiskArbiter,
    RotationContext, RotationRuntime, SizingConfig, WalletLifecycle, conservative_cent_price,
    crypto_taker_fee,
};
use rust_decimal_macros::dec;
use tracing::{error, info, warn};

use crate::app::AppConfig;

pub(crate) async fn apply_discovery_result(
    now: i64,
    result: DiscoveryCycleResult,
    runtime: &mut RotationRuntime,
    journal: &JsonlJournal,
    position_open: bool,
    live_active_set: bool,
) -> Result<()> {
    let generation = runtime.registry().state().active_set_generation;
    journal
        .append(&JournalRecord::DiscoveryCycleCompleted {
            observed_epoch: now,
            period_successes: result.period_successes,
            period_failures: result.period_failures,
            candidate_wallets: result.candidate_wallets,
            evaluations: result.evaluations.len(),
            candidate_failures: result.candidate_failures.len(),
            failed_closed: result.failed_closed,
            active_set_generation: generation,
        })
        .await?;
    for evaluation in &result.evaluations {
        journal_candidate(now, evaluation, generation, journal).await?;
    }
    if result.failed_closed {
        warn!(
            period_successes = result.period_successes,
            "discovery failed closed; registry and active set unchanged"
        );
        return Ok(());
    }

    let before = lifecycle_snapshot(runtime);
    runtime.apply_evaluations(result.evaluations, now)?;
    journal_lifecycle_changes(now, &before, runtime, "historical_replay_update", journal).await?;
    rotate_and_journal(now, runtime, position_open, live_active_set, journal).await?;
    Ok(())
}

pub(crate) async fn apply_paper_resolutions(
    now: i64,
    resolutions: HashMap<String, MarketResolution>,
    runtime: &mut RotationRuntime,
    journal: &JsonlJournal,
    position_open: bool,
    live_active_set: bool,
) -> Result<()> {
    let before = lifecycle_snapshot(runtime);
    let records = runtime.resolve_paper(&resolutions, now)?;
    for record in records {
        journal
            .append(&JournalRecord::PaperResolved {
                observed_epoch: now,
                wallet: record.wallet,
                condition_id: record.condition_id,
                won: record.won,
                pnl: record.pnl,
                counts_global: record.counts_global,
                active_set_generation: runtime.registry().state().active_set_generation,
            })
            .await?;
    }
    journal_lifecycle_changes(now, &before, runtime, "forward_paper_resolution", journal).await?;
    rotate_and_journal(now, runtime, position_open, live_active_set, journal).await?;
    Ok(())
}

pub(crate) async fn process_shadow_signal<E: Executor + ?Sized>(
    config: &AppConfig,
    now: i64,
    signal: CandidateSignal,
    executor: &E,
    journal: &JsonlJournal,
    runtime: &mut RotationRuntime,
) -> Result<()> {
    let request = match execution_request(config, &signal) {
        Ok(request) => request,
        Err(error) => {
            journal_rejection(now, signal.condition_id, error.to_string(), journal).await?;
            return Ok(());
        }
    };
    journal
        .append(&JournalRecord::Signal {
            observed_epoch: now,
            signal: signal.clone(),
        })
        .await?;
    let end = signal.market_end_epoch;
    let wallet = signal.wallet.clone();
    match executor.execute(request).await {
        Ok(fill) => {
            journal
                .append(&JournalRecord::Fill {
                    observed_epoch: epoch(),
                    fill: fill.clone(),
                })
                .await?;
            runtime.track_paper_fill(&wallet, end, false, fill)?;
        }
        Err(error) => {
            journal_rejection(now, signal.condition_id, error.to_string(), journal).await?;
        }
    }
    Ok(())
}

pub(crate) async fn process_active_signal<E: Executor + ?Sized>(
    config: &AppConfig,
    now: i64,
    signal: CandidateSignal,
    risk: &mut RiskArbiter,
    executor: &E,
    journal: &JsonlJournal,
    runtime: &mut RotationRuntime,
    open: &mut Option<(String, i64)>,
) -> Result<()> {
    if let Err(reason) = risk.reserve(&signal, now) {
        journal_rejection(now, signal.condition_id, reason.to_string(), journal).await?;
        return Ok(());
    }
    journal
        .append(&JournalRecord::Signal {
            observed_epoch: now,
            signal: signal.clone(),
        })
        .await?;
    let request = match execution_request(config, &signal) {
        Ok(request) => request,
        Err(error) => {
            risk.release(&signal.condition_id);
            journal_rejection(now, signal.condition_id, error.to_string(), journal).await?;
            return Ok(());
        }
    };
    let expected_cost = request.shares * request.maximum_price
        + crypto_taker_fee(request.shares, request.maximum_price)?;
    if let Err(reason) = risk.record_capital_at_risk(&signal.condition_id, expected_cost) {
        journal_rejection(now, signal.condition_id, reason.to_string(), journal).await?;
        return Ok(());
    }

    let end = signal.market_end_epoch;
    let wallet = signal.wallet.clone();
    let condition = signal.condition_id.clone();
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
                "active order filled"
            );
            if fill.paper {
                runtime.track_paper_fill(&wallet, end, true, fill.clone())?;
            }
            *open = Some((fill.condition_id, end));
        }
        Err(error) => {
            risk.release(&condition);
            journal_rejection(now, condition, error.to_string(), journal).await?;
            error!(%error, "active execution failed");
        }
    }
    Ok(())
}

pub(crate) fn retry_after_map(runtime: &RotationRuntime) -> HashMap<String, i64> {
    runtime
        .registry()
        .state()
        .records
        .iter()
        .filter_map(|(wallet, record)| {
            record
                .retry_after_epoch
                .map(|until| (wallet.clone(), until))
        })
        .collect()
}

pub(crate) fn log_preflight(preflight: &PreflightStatus) {
    if preflight.passed() {
        info!("authenticated live preflight passed without placing an order");
    } else {
        warn!(
            reason = preflight.failure_reason.as_deref().unwrap_or("unknown"),
            "authenticated live preflight failed closed"
        );
    }
}

pub(crate) fn millis_to_seconds(milliseconds: u64) -> i64 {
    i64::try_from(milliseconds.div_ceil(1_000)).unwrap_or(i64::MAX)
}

pub(crate) fn epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

async fn rotate_and_journal(
    now: i64,
    runtime: &mut RotationRuntime,
    position_open: bool,
    live_active_set: bool,
    journal: &JsonlJournal,
) -> Result<()> {
    let previous = runtime.registry().state().active_wallets.clone();
    let proposal = runtime.rotate(
        RotationContext {
            position_open,
            submission_in_flight: false,
            state_persisted: true,
        },
        live_active_set,
    )?;
    if proposal.changed {
        journal
            .append(&JournalRecord::ActiveSetChanged {
                observed_epoch: now,
                previous_wallets: previous,
                wallets: proposal.wallets,
                reason: proposal.reason,
                active_set_generation: runtime.registry().state().active_set_generation,
            })
            .await?;
    }
    Ok(())
}

fn execution_request(config: &AppConfig, signal: &CandidateSignal) -> Result<ExecutionRequest> {
    let raw_maximum_price = (signal.source_price + config.max_slippage).min(dec!(0.99));
    let maximum_price = conservative_cent_price(raw_maximum_price)?;
    let size = PositionSizer::new(SizingConfig {
        bankroll: config.bankroll,
        estimated_win_probability: signal.estimated_win_probability,
        kelly_multiplier: dec!(0.25),
        max_bankroll_fraction: config.max_risk_fraction,
        minimum_shares: dec!(5),
    })?
    .size(maximum_price)?;
    Ok(ExecutionRequest {
        signal: signal.clone(),
        shares: size.shares,
        maximum_price,
    })
}

async fn journal_candidate(
    now: i64,
    evaluation: &CandidateEvaluation,
    generation: u64,
    journal: &JsonlJournal,
) -> Result<()> {
    journal
        .append(&JournalRecord::CandidateEvaluated {
            observed_epoch: now,
            wallet: evaluation.wallet.clone(),
            family: evaluation.family,
            eligible: evaluation.eligible,
            score: evaluation.score,
            resolved_signals: evaluation.metrics.resolved_signals,
            net_pnl_one_cent: evaluation.metrics.net_pnl_one_cent,
            rejection_reasons: evaluation.rejection_reasons.clone(),
            active_set_generation: generation,
        })
        .await?;
    Ok(())
}

async fn journal_lifecycle_changes(
    now: i64,
    before: &HashMap<String, WalletLifecycle>,
    runtime: &RotationRuntime,
    reason: &str,
    journal: &JsonlJournal,
) -> Result<()> {
    let generation = runtime.registry().state().active_set_generation;
    for (wallet, record) in &runtime.registry().state().records {
        let from = before
            .get(wallet)
            .copied()
            .unwrap_or(WalletLifecycle::Discovered);
        if from != record.lifecycle {
            journal
                .append(&JournalRecord::WalletLifecycleChanged {
                    observed_epoch: now,
                    wallet: wallet.clone(),
                    from,
                    to: record.lifecycle,
                    reason: reason.into(),
                    active_set_generation: generation,
                })
                .await?;
        }
    }
    Ok(())
}

fn lifecycle_snapshot(runtime: &RotationRuntime) -> HashMap<String, WalletLifecycle> {
    runtime
        .registry()
        .state()
        .records
        .iter()
        .map(|(wallet, record)| (wallet.clone(), record.lifecycle))
        .collect()
}

async fn journal_rejection(
    now: i64,
    condition_id: String,
    reason: String,
    journal: &JsonlJournal,
) -> Result<()> {
    journal
        .append(&JournalRecord::Rejection {
            observed_epoch: now,
            condition_id,
            reason,
        })
        .await?;
    Ok(())
}
