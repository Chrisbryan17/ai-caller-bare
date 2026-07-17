use polymarket_copybot::{
    CandidateEvaluation, LaunchController, LaunchMode, PreflightStatus, ReplayMetrics,
    StrategyFamily, WalletRegistry,
};
use rust_decimal_macros::dec;
use tempfile::tempdir;

#[test]
fn global_launch_gate_requires_time_resolutions_profit_health_and_wallet_qualification() {
    let dir = tempdir().unwrap();
    let registry = WalletRegistry::load_or_new(dir.path().join("r.json"), dec!(95)).unwrap();
    let mut controller = LaunchController::new(1_000, dec!(95));
    controller.record_resolved(dec!(0.50), true);
    controller.record_resolved(dec!(0.50), true);
    controller.record_resolved(dec!(0.50), true);
    controller.record_resolved(dec!(0.50), true);
    controller.record_resolved(dec!(0.50), true);
    controller.record_processing_error();
    let preflight = PreflightStatus::all_passed();

    assert_eq!(
        controller.evaluate(4_599, &registry, &[], &preflight, true),
        LaunchMode::Paper
    );
    assert_eq!(
        controller.evaluate(4_600, &registry, &[], &preflight, true),
        LaunchMode::Paper
    );
}

#[test]
fn failed_preflight_never_arms_live() {
    let dir = tempdir().unwrap();
    let registry = WalletRegistry::load_or_new(dir.path().join("r.json"), dec!(95)).unwrap();
    let controller = LaunchController::new(1_000, dec!(95));
    assert_eq!(
        controller.evaluate(
            10_000,
            &registry,
            &[],
            &PreflightStatus::failed("geoblocked"),
            true
        ),
        LaunchMode::Paper
    );
}

#[test]
fn global_four_loss_streak_cannot_be_erased_by_later_wins() {
    let dir = tempdir().unwrap();
    let wallet = "0x4444444444444444444444444444444444444444";
    let mut registry = WalletRegistry::load_or_new(dir.path().join("r.json"), dec!(95)).unwrap();
    registry.upsert_evaluation(
        CandidateEvaluation {
            wallet: wallet.into(),
            family: StrategyFamily::FirstLargeBuy,
            estimated_win_probability: dec!(0.65),
            score: dec!(100),
            eligible: true,
            rejection_reasons: vec![],
            metrics: ReplayMetrics::default(),
        },
        1_000,
    );
    registry.force_paper_qualified(wallet).unwrap();
    registry.apply_active_wallets(vec![wallet.into()], false);

    let mut controller = LaunchController::new(1_000, dec!(95));
    for _ in 0..4 {
        controller.record_resolved(dec!(-0.10), false);
    }
    for _ in 0..9 {
        controller.record_resolved(dec!(0.20), true);
    }

    assert_eq!(
        controller.evaluate(
            4_600,
            &registry,
            &[wallet.into()],
            &PreflightStatus::all_passed(),
            true,
        ),
        LaunchMode::Paper
    );
}
