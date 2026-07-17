use std::collections::HashMap;

use polymarket_copybot::{
    CandidateEvaluation, ExecutionFill, MarketResolution, Outcome, ReplayMetrics, RotationRuntime,
    StrategyFamily, SuspensionReason, WalletLifecycle,
};
use rust_decimal_macros::dec;
use tempfile::tempdir;

fn evaluation(wallet: &str) -> CandidateEvaluation {
    CandidateEvaluation {
        wallet: wallet.into(),
        family: StrategyFamily::FirstLargeBuy,
        estimated_win_probability: dec!(0.65),
        score: dec!(100),
        eligible: true,
        rejection_reasons: vec![],
        metrics: ReplayMetrics::default(),
    }
}

fn live_fill(condition: &str) -> ExecutionFill {
    ExecutionFill {
        condition_id: condition.into(),
        asset_id: format!("asset-{condition}"),
        outcome: Outcome::Up,
        shares: dec!(10),
        fill_price: dec!(0.40),
        fee: dec!(0.02),
        total_cost: dec!(4.02),
        external_id: Some(format!("order-{condition}")),
        paper: false,
    }
}

fn down_resolution(condition: &str) -> HashMap<String, MarketResolution> {
    HashMap::from([(
        condition.into(),
        MarketResolution {
            condition_id: condition.into(),
            closed: true,
            winner: Some(Outcome::Down),
        },
    )])
}

#[test]
fn two_resolved_live_losses_suspend_wallet_remove_authority_and_survive_restart() {
    let dir = tempdir().unwrap();
    let registry_path = dir.path().join("registry.json");
    let wallet = "0x5555555555555555555555555555555555555555";
    let mut runtime = RotationRuntime::open(&registry_path, dec!(95), 1_000).unwrap();
    runtime
        .apply_evaluations(vec![evaluation(wallet)], 1_000)
        .unwrap();
    runtime
        .registry_mut()
        .force_paper_qualified(wallet)
        .unwrap();
    runtime
        .registry_mut()
        .apply_active_wallets(vec![wallet.into()], true);
    runtime.registry().save_atomic().unwrap();

    runtime
        .track_live_fill(wallet, 2_000, live_fill("live-loss-1"))
        .unwrap();
    let first = runtime
        .resolve_positions(&down_resolution("live-loss-1"), 2_100)
        .unwrap();
    assert_eq!(first.len(), 1);
    assert!(first[0].live);
    assert_eq!(
        runtime.registry().record(wallet).unwrap().lifecycle,
        WalletLifecycle::ActiveLive
    );

    runtime
        .track_live_fill(wallet, 3_000, live_fill("live-loss-2"))
        .unwrap();
    let second = runtime
        .resolve_positions(&down_resolution("live-loss-2"), 3_100)
        .unwrap();
    assert_eq!(second.len(), 1);
    assert!(second[0].live);

    let record = runtime.registry().record(wallet).unwrap();
    assert_eq!(record.live_resolved, 2);
    assert_eq!(record.live_losses, 2);
    assert_eq!(record.lifecycle, WalletLifecycle::Suspended);
    assert_eq!(
        record.suspension_reason,
        Some(SuspensionReason::ConsecutiveLosses)
    );
    assert!(runtime.registry().state().active_wallets.is_empty());

    drop(runtime);
    let restored = RotationRuntime::open(&registry_path, dec!(95), 4_000).unwrap();
    let restored_record = restored.registry().record(wallet).unwrap();
    assert_eq!(restored_record.live_resolved, 2);
    assert_eq!(restored_record.lifecycle, WalletLifecycle::Suspended);
    assert!(restored.registry().state().active_wallets.is_empty());
}

#[test]
fn one_live_loss_above_wallet_drawdown_limit_suspends_immediately() {
    let dir = tempdir().unwrap();
    let registry_path = dir.path().join("registry.json");
    let wallet = "0x6666666666666666666666666666666666666666";
    let mut runtime = RotationRuntime::open(&registry_path, dec!(95), 1_000).unwrap();
    runtime
        .apply_evaluations(vec![evaluation(wallet)], 1_000)
        .unwrap();
    runtime
        .registry_mut()
        .force_paper_qualified(wallet)
        .unwrap();
    runtime
        .registry_mut()
        .apply_active_wallets(vec![wallet.into()], true);

    let mut fill = live_fill("oversized-loss");
    fill.total_cost = dec!(4.76);
    runtime.track_live_fill(wallet, 2_000, fill).unwrap();
    runtime
        .resolve_positions(&down_resolution("oversized-loss"), 2_100)
        .unwrap();

    let record = runtime.registry().record(wallet).unwrap();
    assert_eq!(record.lifecycle, WalletLifecycle::Suspended);
    assert_eq!(record.suspension_reason, Some(SuspensionReason::Drawdown));
}
