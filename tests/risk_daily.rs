use polymarket_copybot::{
    CandidateSignal, Outcome, RiskArbiter, RiskConfig, RiskReject, utc_day_index,
};
use rust_decimal_macros::dec;

fn candidate(condition: &str) -> CandidateSignal {
    CandidateSignal {
        wallet: "0xwallet".into(),
        condition_id: condition.into(),
        asset_id: "asset".into(),
        outcome: Outcome::Up,
        source_price: dec!(0.40),
        source_timestamp: 1_000,
        market_end_epoch: 2_000,
        slug: "btc-updown-5m-1784241900".into(),
        title: "BTC".into(),
        strategy: "test".into(),
        estimated_win_probability: dec!(0.60),
    }
}

#[test]
fn utc_day_index_changes_only_at_midnight_boundary() {
    assert_eq!(utc_day_index(0), 0);
    assert_eq!(utc_day_index(86_399), 0);
    assert_eq!(utc_day_index(86_400), 1);
    assert_eq!(utc_day_index(172_799), 1);
}

#[test]
fn daily_risk_limit_blocks_then_explicit_reset_reopens_budget() {
    let mut risk = RiskArbiter::new(RiskConfig {
        minimum_lead_seconds: 90,
        max_open_markets: 1,
        max_daily_capital_at_risk: dec!(12),
    })
    .unwrap();

    risk.reserve(&candidate("c1"), 1_000).unwrap();
    risk.record_capital_at_risk("c1", dec!(7)).unwrap();
    risk.release("c1");

    risk.reserve(&candidate("c2"), 1_000).unwrap();
    assert_eq!(
        risk.record_capital_at_risk("c2", dec!(6)).unwrap_err(),
        RiskReject::DailyRiskLimit
    );

    risk.reset_daily_risk();
    risk.reserve(&candidate("c3"), 1_000).unwrap();
    risk.record_capital_at_risk("c3", dec!(6)).unwrap();
}
