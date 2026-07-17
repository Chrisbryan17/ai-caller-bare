use std::collections::HashMap;

use polymarket_copybot::{
    CandidateEvaluation, ExecutionFill, MAX_WATCHED_WALLETS, MarketResolution, Outcome,
    ReplayMetrics, RotationContext, RotationRuntime, StrategyFamily, Trade, WalletLifecycle,
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
        metrics: ReplayMetrics {
            resolved_signals: 10,
            wins: 7,
            losses: 3,
            recent_twenty_four_hours: 5,
            recent_six_hours: 2,
            net_pnl_one_cent: dec!(2),
            net_pnl_two_cent: dec!(1),
            turnover_one_cent: dec!(5),
            roi_one_cent: dec!(0.4),
            max_loss_streak: 2,
            max_drawdown: dec!(1),
            median_price: dec!(0.30),
            median_lead_seconds: 120,
            two_cent_profitable_fraction: dec!(0.70),
            latest_signal_epoch: 10_000,
            signal_trace: vec![],
        },
    }
}

fn trade(condition: &str, tx: &str) -> Trade {
    Trade {
        proxy_wallet: "0x1111111111111111111111111111111111111111".into(),
        side: "BUY".into(),
        asset: "asset".into(),
        condition_id: condition.into(),
        size: dec!(100),
        price: dec!(0.30),
        timestamp: 1_800_000_120,
        title: "BTC".into(),
        slug: "btc-updown-5m-1800000000".into(),
        outcome: "Up".into(),
        transaction_hash: tx.into(),
    }
}

#[test]
fn discovered_wallet_is_quarantined_and_first_snapshot_only_primes() {
    let dir = tempdir().unwrap();
    let mut runtime =
        RotationRuntime::open(dir.path().join("registry.json"), dec!(95), 1_000).unwrap();
    runtime
        .apply_evaluations(
            vec![evaluation("0x1111111111111111111111111111111111111111")],
            1_000,
        )
        .unwrap();
    assert_eq!(
        runtime
            .registry()
            .record("0x1111111111111111111111111111111111111111")
            .unwrap()
            .lifecycle,
        WalletLifecycle::Quarantined
    );
    assert!(
        runtime
            .process_snapshot(
                "0x1111111111111111111111111111111111111111",
                vec![trade("old", "a")],
                1_001
            )
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        runtime
            .process_snapshot(
                "0x1111111111111111111111111111111111111111",
                vec![trade("new", "b")],
                1_002
            )
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn qualified_rotation_is_deferred_while_position_is_open() {
    let dir = tempdir().unwrap();
    let mut runtime =
        RotationRuntime::open(dir.path().join("registry.json"), dec!(95), 1_000).unwrap();
    runtime
        .apply_evaluations(
            vec![evaluation("0x1111111111111111111111111111111111111111")],
            1_000,
        )
        .unwrap();
    runtime
        .registry_mut()
        .force_paper_qualified("0x1111111111111111111111111111111111111111")
        .unwrap();
    let proposal = runtime
        .rotate(
            RotationContext {
                position_open: true,
                submission_in_flight: false,
                state_persisted: true,
            },
            false,
        )
        .unwrap();
    assert!(proposal.deferred);
    assert!(runtime.registry().state().active_wallets.is_empty());
}

#[test]
fn resolved_shadow_position_updates_wallet_and_global_paper_ledgers() {
    let dir = tempdir().unwrap();
    let wallet = "0x1111111111111111111111111111111111111111";
    let mut runtime =
        RotationRuntime::open(dir.path().join("registry.json"), dec!(95), 1_000).unwrap();
    runtime
        .apply_evaluations(vec![evaluation(wallet)], 1_000)
        .unwrap();
    runtime
        .track_paper_fill(
            wallet,
            1_300,
            true,
            ExecutionFill {
                condition_id: "c1".into(),
                asset_id: "asset".into(),
                outcome: Outcome::Up,
                shares: dec!(10),
                fill_price: dec!(0.31),
                fee: dec!(0.15),
                total_cost: dec!(3.25),
                external_id: None,
                paper: true,
            },
        )
        .unwrap();
    let resolutions = HashMap::from([(
        "c1".into(),
        MarketResolution {
            condition_id: "c1".into(),
            closed: true,
            winner: Some(Outcome::Up),
        },
    )]);
    let records = runtime.resolve_paper(&resolutions, 1_400).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(runtime.registry().record(wallet).unwrap().paper_resolved, 1);
    assert_eq!(
        runtime
            .launch_controller()
            .snapshot(
                5_000,
                runtime.registry(),
                &[wallet.into()],
                &polymarket_copybot::PreflightStatus::all_passed()
            )
            .resolved_positions,
        1
    );
}

#[test]
fn watcher_pool_is_capped_to_preserve_the_public_trade_rate_budget() {
    let dir = tempdir().unwrap();
    let mut runtime =
        RotationRuntime::open(dir.path().join("registry.json"), dec!(95), 1_000).unwrap();
    let evaluations = (0..6)
        .map(|index| evaluation(&format!("wallet-{index}")))
        .collect();
    runtime.apply_evaluations(evaluations, 1_000).unwrap();
    assert_eq!(MAX_WATCHED_WALLETS, 4);
    assert_eq!(runtime.watched_wallets().len(), MAX_WATCHED_WALLETS);
}
