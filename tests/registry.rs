use polymarket_copybot::{
    CandidateEvaluation, PaperOutcome, ReplayMetrics, StrategyFamily, SuspensionReason,
    WalletLifecycle, WalletRegistry,
};
use rust_decimal_macros::dec;
use tempfile::tempdir;

fn evaluation(wallet: &str, score: rust_decimal::Decimal) -> CandidateEvaluation {
    CandidateEvaluation {
        wallet: wallet.into(),
        family: StrategyFamily::FirstLargeBuy,
        estimated_win_probability: dec!(0.65),
        score,
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

#[test]
fn forward_paper_evidence_promotes_wallet_and_survives_restart() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("registry.json");
    let wallet = "0x1111111111111111111111111111111111111111";
    let mut registry = WalletRegistry::load_or_new(&path, dec!(95)).unwrap();
    registry.upsert_evaluation(evaluation(wallet, dec!(100)), 1_000);
    assert_eq!(
        registry.record(wallet).unwrap().lifecycle,
        WalletLifecycle::Quarantined
    );

    for index in 0..5 {
        registry
            .record_paper_outcome(
                wallet,
                PaperOutcome {
                    condition_id: format!("c{index}"),
                    resolved_epoch: 4_700 + index,
                    pnl: dec!(0.50),
                    won: true,
                },
            )
            .unwrap();
    }
    registry.refresh_qualification(wallet, 4_700).unwrap();
    assert_eq!(
        registry.record(wallet).unwrap().lifecycle,
        WalletLifecycle::PaperQualified
    );
    registry.save_atomic().unwrap();

    let restored = WalletRegistry::load_or_new(&path, dec!(95)).unwrap();
    assert_eq!(restored.state().bankroll, dec!(95));
    assert_eq!(restored.state().max_risk_fraction, dec!(0.05));
    assert_eq!(restored.record(wallet).unwrap().paper_resolved, 5);
    assert_eq!(
        restored.record(wallet).unwrap().lifecycle,
        WalletLifecycle::PaperQualified
    );
}

#[test]
fn corrupt_registry_fails_closed() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("registry.json");
    std::fs::write(&path, "not-json").unwrap();
    assert!(WalletRegistry::load_or_new(&path, dec!(95)).is_err());
}

#[test]
fn replay_failure_suspends_previously_qualified_wallet_and_removes_active_authority() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("registry.json");
    let wallet = "0x2222222222222222222222222222222222222222";
    let mut registry = WalletRegistry::load_or_new(&path, dec!(95)).unwrap();
    registry.upsert_evaluation(evaluation(wallet, dec!(100)), 1_000);
    registry.force_paper_qualified(wallet).unwrap();
    registry.apply_active_wallets(vec![wallet.into()], true);

    let mut failed = evaluation(wallet, dec!(-10));
    failed.eligible = false;
    failed.rejection_reasons = vec!["non_positive_edge".into()];
    registry.upsert_evaluation(failed, 2_000);

    let record = registry.record(wallet).unwrap();
    assert_eq!(record.lifecycle, WalletLifecycle::Suspended);
    assert_eq!(
        record.suspension_reason,
        Some(SuspensionReason::ReplayFailure)
    );
    assert!(
        !registry
            .state()
            .active_wallets
            .contains(&wallet.to_string())
    );
}

#[test]
fn four_loss_streak_cannot_be_erased_by_later_wins() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("registry.json");
    let wallet = "0x3333333333333333333333333333333333333333";
    let mut registry = WalletRegistry::load_or_new(&path, dec!(95)).unwrap();
    registry.upsert_evaluation(evaluation(wallet, dec!(100)), 1_000);

    for index in 0..4 {
        registry
            .record_paper_outcome(
                wallet,
                PaperOutcome {
                    condition_id: format!("loss-{index}"),
                    resolved_epoch: 2_000 + index,
                    pnl: dec!(-0.50),
                    won: false,
                },
            )
            .unwrap();
    }
    for index in 0..5 {
        registry
            .record_paper_outcome(
                wallet,
                PaperOutcome {
                    condition_id: format!("win-{index}"),
                    resolved_epoch: 3_000 + index,
                    pnl: dec!(1.00),
                    won: true,
                },
            )
            .unwrap();
    }

    assert!(!registry.refresh_qualification(wallet, 4_700).unwrap());
    assert_eq!(
        registry.record(wallet).unwrap().lifecycle,
        WalletLifecycle::Quarantined
    );
}
