use polymarket_copybot::{
    CandidateEvaluation, CandidateSignal, ExecutionFill, Outcome, ReplayMetrics, RiskArbiter,
    RiskConfig, RiskReject, RotationRuntime, StrategyFamily,
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

fn fill(condition: &str, paper: bool) -> ExecutionFill {
    ExecutionFill {
        condition_id: condition.into(),
        asset_id: format!("asset-{condition}"),
        outcome: Outcome::Up,
        shares: dec!(10),
        fill_price: dec!(0.40),
        fee: dec!(0.02),
        total_cost: dec!(4.02),
        external_id: (!paper).then(|| format!("order-{condition}")),
        paper,
    }
}

fn candidate(condition: &str, end: i64) -> CandidateSignal {
    CandidateSignal {
        wallet: "0x7777777777777777777777777777777777777777".into(),
        condition_id: condition.into(),
        asset_id: format!("asset-{condition}"),
        outcome: Outcome::Up,
        source_price: dec!(0.40),
        source_timestamp: 1_500,
        market_end_epoch: end,
        slug: "btc-updown-5m-1784241900".into(),
        title: "BTC".into(),
        strategy: "restart-test".into(),
        estimated_win_probability: dec!(0.65),
    }
}

#[test]
fn restart_restores_the_single_active_position_into_the_risk_arbiter() {
    let dir = tempdir().unwrap();
    let registry_path = dir.path().join("registry.json");
    let wallet = "0x7777777777777777777777777777777777777777";
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
    runtime
        .track_live_fill(wallet, 2_000, fill("still-open", false))
        .unwrap();
    drop(runtime);

    let runtime = RotationRuntime::open(&registry_path, dec!(95), 1_500).unwrap();
    let restored = runtime.active_position(1_500).unwrap().unwrap();
    assert_eq!(restored.condition_id, "still-open");
    assert_eq!(restored.outcome, Outcome::Up);

    let mut risk = RiskArbiter::new_with_daily(
        RiskConfig {
            minimum_lead_seconds: 90,
            max_open_markets: 1,
            max_daily_capital_at_risk: dec!(9.50),
        },
        dec!(0),
    )
    .unwrap();
    risk.restore_open(&restored.condition_id, restored.outcome)
        .unwrap();
    assert_eq!(
        risk.reserve(&candidate("second-market", 2_500), 1_500),
        Err(RiskReject::ExposureLimit)
    );
}

#[test]
fn pending_live_submission_survives_ambiguous_crash_until_market_end() {
    let dir = tempdir().unwrap();
    let registry_path = dir.path().join("registry.json");
    let wallet = "0x7777777777777777777777777777777777777777";
    let mut runtime = RotationRuntime::open(&registry_path, dec!(95), 1_000).unwrap();
    runtime
        .reserve_live_submission(wallet, "ambiguous", Outcome::Up, 1_500, 2_000)
        .unwrap();
    drop(runtime);

    let mut restored = RotationRuntime::open(&registry_path, dec!(95), 1_600).unwrap();
    let active = restored.active_position(1_600).unwrap().unwrap();
    assert_eq!(active.condition_id, "ambiguous");
    assert_eq!(active.outcome, Outcome::Up);

    assert!(
        restored
            .clear_expired_live_submission(1_999)
            .unwrap()
            .is_none()
    );
    assert!(restored.active_position(1_999).unwrap().is_some());
    let cleared = restored
        .clear_expired_live_submission(2_000)
        .unwrap()
        .unwrap();
    assert_eq!(cleared.condition_id, "ambiguous");
    assert!(restored.active_position(2_000).unwrap().is_none());
}

#[test]
fn restart_allows_multiple_expired_unresolved_paper_positions() {
    let dir = tempdir().unwrap();
    let registry_path = dir.path().join("registry.json");
    let wallet = "0x7777777777777777777777777777777777777777";
    let mut runtime = RotationRuntime::open(&registry_path, dec!(95), 1_000).unwrap();
    runtime
        .track_paper_fill(wallet, 1_100, true, fill("expired-one", true))
        .unwrap();
    runtime
        .track_paper_fill(wallet, 1_200, true, fill("expired-two", true))
        .unwrap();
    drop(runtime);

    let restored = RotationRuntime::open(&registry_path, dec!(95), 1_300).unwrap();
    assert!(restored.active_position(1_300).unwrap().is_none());
    assert_eq!(restored.pending_positions().len(), 2);
    assert_eq!(
        restored.due_condition_ids(1_300),
        vec!["expired-one".to_owned(), "expired-two".to_owned()]
    );
}

#[test]
fn restart_preserves_live_daily_risk_and_resets_only_on_a_new_utc_day() {
    let dir = tempdir().unwrap();
    let registry_path = dir.path().join("registry.json");
    let mut runtime = RotationRuntime::open(&registry_path, dec!(95), 1_000).unwrap();
    runtime
        .persist_live_daily_capital_at_risk(20_000, dec!(9.50))
        .unwrap();
    drop(runtime);

    let mut restored = RotationRuntime::open(&registry_path, dec!(95), 1_500).unwrap();
    let amount = restored.live_daily_capital_at_risk(20_000).unwrap();
    assert_eq!(amount, dec!(9.50));

    let mut risk = RiskArbiter::new_with_daily(
        RiskConfig {
            minimum_lead_seconds: 90,
            max_open_markets: 1,
            max_daily_capital_at_risk: dec!(9.50),
        },
        amount,
    )
    .unwrap();
    assert_eq!(
        risk.reserve(&candidate("blocked", 2_500), 1_500),
        Err(RiskReject::DailyRiskLimit)
    );

    assert_eq!(
        restored.live_daily_capital_at_risk(20_001).unwrap(),
        dec!(0)
    );
}
