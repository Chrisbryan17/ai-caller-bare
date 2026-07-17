use std::collections::HashMap;

use polymarket_copybot::{MarketResolution, Outcome, ReplayEvaluator, StrategyFamily, Trade};
use rust_decimal_macros::dec;

fn trade(wallet: &str, index: i64, outcome: &str, price: rust_decimal::Decimal) -> Trade {
    let start = 1_800_000_000 + index * 300;
    Trade {
        proxy_wallet: wallet.into(),
        side: "BUY".into(),
        asset: format!("asset-{index}-{outcome}"),
        condition_id: format!("c{index}"),
        size: dec!(100),
        price,
        timestamp: start + 120,
        title: "BTC".into(),
        slug: format!("btc-updown-5m-{start}"),
        outcome: outcome.into(),
        transaction_hash: format!("tx-{index}-{outcome}"),
    }
}

fn resolutions(count: i64, winner: Outcome) -> HashMap<String, MarketResolution> {
    (0..count)
        .map(|index| {
            (
                format!("c{index}"),
                MarketResolution {
                    condition_id: format!("c{index}"),
                    closed: true,
                    winner: Some(winner),
                },
            )
        })
        .collect()
}

#[test]
fn replay_qualifies_profitable_recent_first_large_buy_wallet() {
    let wallet = "0x1111111111111111111111111111111111111111";
    let trades: Vec<_> = (0..6).map(|i| trade(wallet, i, "Up", dec!(0.30))).collect();
    let now = 1_800_000_000 + 6 * 300 + 60;
    let result = ReplayEvaluator::default()
        .evaluate(wallet, &trades, &resolutions(6, Outcome::Up), now)
        .unwrap();
    assert!(result.eligible);
    assert_eq!(result.family, StrategyFamily::FirstLargeBuy);
    assert_eq!(result.metrics.resolved_signals, 6);
    assert!(result.metrics.net_pnl_one_cent > dec!(0));
    assert_eq!(result.metrics.recent_six_hours, 6);
}

#[test]
fn unresolved_markets_do_not_count_as_replay_wins() {
    let wallet = "0x1111111111111111111111111111111111111111";
    let trades: Vec<_> = (0..6).map(|i| trade(wallet, i, "Up", dec!(0.30))).collect();
    let result = ReplayEvaluator::default()
        .evaluate(wallet, &trades, &HashMap::new(), 1_800_002_000)
        .unwrap();
    assert!(!result.eligible);
    assert_eq!(result.metrics.resolved_signals, 0);
}

#[test]
fn pervasive_two_sided_hedging_is_rejected() {
    let wallet = "0x1111111111111111111111111111111111111111";
    let mut trades = Vec::new();
    for index in 0..6 {
        trades.push(trade(wallet, index, "Up", dec!(0.30)));
        trades.push(trade(wallet, index, "Down", dec!(0.30)));
    }
    let result = ReplayEvaluator::default()
        .evaluate(wallet, &trades, &resolutions(6, Outcome::Up), 1_800_002_000)
        .unwrap();
    assert!(!result.eligible);
    assert!(
        result
            .rejection_reasons
            .iter()
            .any(|r| r == "two_sided_hedging")
    );
}
