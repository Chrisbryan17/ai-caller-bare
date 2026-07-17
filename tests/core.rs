#![allow(clippy::too_many_arguments)]

use polymarket_copybot::{
    BoundedDedupe, CandidateSignal, ExecutionRequest, Executor, Outcome, PaperExecutor,
    PositionSizer, RiskArbiter, RiskConfig, RiskReject, SizingConfig, StrategyConfig,
    StrategyEngine, Trade, crypto_taker_fee_per_share, parse_market_window, trade_key,
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

fn trade(
    wallet: &str,
    condition: &str,
    asset: &str,
    outcome: &str,
    size: Decimal,
    price: Decimal,
    timestamp: i64,
    slug: &str,
    tx: &str,
) -> Trade {
    Trade {
        proxy_wallet: wallet.to_owned(),
        side: "BUY".to_owned(),
        asset: asset.to_owned(),
        condition_id: condition.to_owned(),
        size,
        price,
        timestamp,
        title: slug.to_owned(),
        slug: slug.to_owned(),
        outcome: outcome.to_owned(),
        transaction_hash: tx.to_owned(),
    }
}

#[test]
fn parses_only_supported_short_duration_crypto_slugs() {
    let market = parse_market_window("eth-updown-15m-1784241900").unwrap();
    assert_eq!(market.symbol, "eth");
    assert_eq!(market.duration_seconds, 900);
    assert_eq!(market.start_epoch, 1_784_241_900);
    assert_eq!(market.end_epoch, 1_784_242_800);

    assert!(parse_market_window("doge-updown-5m-1784241900").is_err());
    assert!(parse_market_window("btc-updown-1h-1784241900").is_err());
}

#[test]
fn stable_trade_key_changes_when_fill_identity_changes() {
    let a = trade(
        "0xabc",
        "0xcondition",
        "123",
        "Up",
        dec!(100),
        dec!(0.31),
        1_784_241_950,
        "btc-updown-5m-1784241900",
        "0xtx",
    );
    let mut b = a.clone();
    assert_eq!(trade_key(&a), trade_key(&b));
    b.price = dec!(0.32);
    assert_ne!(trade_key(&a), trade_key(&b));
}

#[test]
fn crypto_fee_uses_exact_decimal_formula() {
    assert_eq!(crypto_taker_fee_per_share(dec!(0.5)).unwrap(), dec!(0.0175));
    assert_eq!(crypto_taker_fee_per_share(dec!(0)).unwrap(), Decimal::ZERO);
    assert!(crypto_taker_fee_per_share(dec!(1.01)).is_err());
}

#[test]
fn sizing_uses_quarter_kelly_but_never_exceeds_bankroll_cap() {
    let sizer = PositionSizer::new(SizingConfig {
        bankroll: dec!(125),
        estimated_win_probability: dec!(0.60),
        kelly_multiplier: dec!(0.25),
        max_bankroll_fraction: dec!(0.05),
        minimum_shares: dec!(5),
    })
    .unwrap();

    let decision = sizer.size(dec!(0.31)).unwrap();
    assert_eq!(decision.fraction, dec!(0.05));
    assert!(decision.total_cost <= dec!(6.25));
    assert!(decision.shares >= dec!(5));
    assert_eq!(decision.shares.scale(), 0);
}

#[test]
fn first_large_buy_strategy_emits_once_with_enough_lead() {
    let wallet = "0x208326efd5d051c59631ed626848b150b8d8259c";
    let mut engine = StrategyEngine::new(StrategyConfig::FirstLargeBuy {
        wallet: wallet.to_owned(),
        minimum_notional: dec!(25),
        minimum_lead_seconds: 90,
        estimated_win_probability: dec!(0.52),
    });

    let small = trade(
        wallet,
        "c1",
        "asset-up",
        "Up",
        dec!(50),
        dec!(0.20),
        1_784_241_950,
        "btc-updown-5m-1784241900",
        "t1",
    );
    assert!(engine.ingest(&small).unwrap().is_none());

    let large = trade(
        wallet,
        "c1",
        "asset-up",
        "Up",
        dec!(100),
        dec!(0.30),
        1_784_241_970,
        "btc-updown-5m-1784241900",
        "t2",
    );
    let signal = engine.ingest(&large).unwrap().unwrap();
    assert_eq!(signal.outcome, Outcome::Up);
    assert_eq!(signal.asset_id, "asset-up");
    assert_eq!(signal.source_price, dec!(0.30));
    assert_eq!(signal.market_end_epoch, 1_784_242_200);

    let later = trade(
        wallet,
        "c1",
        "asset-up",
        "Up",
        dec!(200),
        dec!(0.35),
        1_784_241_980,
        "btc-updown-5m-1784241900",
        "t3",
    );
    assert!(engine.ingest(&later).unwrap().is_none());
}

#[test]
fn confirmed_flow_strategy_uses_directional_cumulative_notional() {
    let wallet = "0xb89d0b6e96e790afa900b53476b8f267a94d1d4f";
    let mut engine = StrategyEngine::new(StrategyConfig::ConfirmedFlow {
        wallet: wallet.to_owned(),
        minimum_cumulative_notional: dec!(100),
        minimum_directional_share: dec!(0.80),
        minimum_price: dec!(0.40),
        maximum_price: dec!(0.55),
        minimum_lead_seconds: 90,
        estimated_win_probability: dec!(0.65),
    });

    let first = trade(
        wallet,
        "c2",
        "asset-up",
        "Up",
        dec!(120),
        dec!(0.50),
        1_784_241_920,
        "eth-updown-5m-1784241900",
        "a",
    );
    assert!(engine.ingest(&first).unwrap().is_none());

    let second = trade(
        wallet,
        "c2",
        "asset-up",
        "Up",
        dec!(100),
        dec!(0.50),
        1_784_241_930,
        "eth-updown-5m-1784241900",
        "b",
    );
    let signal = engine.ingest(&second).unwrap().unwrap();
    assert_eq!(signal.outcome, Outcome::Up);
    assert_eq!(signal.source_price, dec!(0.50));
}

#[test]
fn bounded_dedupe_evicts_oldest_entry() {
    let mut dedupe = BoundedDedupe::new(2).unwrap();
    assert!(dedupe.insert("a".to_owned()));
    assert!(!dedupe.insert("a".to_owned()));
    assert!(dedupe.insert("b".to_owned()));
    assert!(dedupe.insert("c".to_owned()));
    assert!(dedupe.insert("a".to_owned()));
}

fn candidate(condition: &str, outcome: Outcome, end: i64) -> CandidateSignal {
    CandidateSignal {
        wallet: "0xwallet".to_owned(),
        condition_id: condition.to_owned(),
        asset_id: "123".to_owned(),
        outcome,
        source_price: dec!(0.40),
        source_timestamp: end - 120,
        market_end_epoch: end,
        slug: "btc-updown-5m-1784241900".to_owned(),
        title: "BTC Up or Down".to_owned(),
        strategy: "test".to_owned(),
        estimated_win_probability: dec!(0.60),
    }
}

#[test]
fn risk_arbiter_rejects_stale_duplicate_and_conflicting_markets() {
    let mut risk = RiskArbiter::new(RiskConfig {
        minimum_lead_seconds: 90,
        max_open_markets: 1,
        max_daily_capital_at_risk: dec!(12),
    })
    .unwrap();

    let now = 2_000;
    let stale = candidate("stale", Outcome::Up, now + 89);
    assert_eq!(risk.reserve(&stale, now).unwrap_err(), RiskReject::Stale);

    let accepted = candidate("same", Outcome::Up, now + 120);
    risk.reserve(&accepted, now).unwrap();
    assert_eq!(
        risk.reserve(&accepted, now).unwrap_err(),
        RiskReject::DuplicateMarket
    );

    risk.release("same");
    let up = candidate("conflict", Outcome::Up, now + 120);
    risk.reserve(&up, now).unwrap();
    risk.release("conflict");
    let down = candidate("conflict", Outcome::Down, now + 120);
    assert_eq!(
        risk.reserve(&down, now).unwrap_err(),
        RiskReject::ConflictingOutcome
    );
}

#[tokio::test]
async fn paper_executor_applies_hard_price_cap_and_exact_fee() {
    let executor = PaperExecutor::new(dec!(0.01)).unwrap();
    let signal = candidate("paper", Outcome::Up, 3_000);
    let request = ExecutionRequest {
        signal,
        shares: dec!(10),
        maximum_price: dec!(0.41),
    };
    let fill = executor.execute(request).await.unwrap();
    assert_eq!(fill.fill_price, dec!(0.41));
    assert_eq!(fill.shares, dec!(10));
    assert_eq!(fill.fee, dec!(0.16933));
    assert_eq!(fill.total_cost, dec!(4.26933));
}
