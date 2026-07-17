use polymarket_copybot::{StrategyConfig, StrategyEngine, Trade, WalletWatcher};
use rust_decimal_macros::dec;

fn trade(condition: &str, timestamp: i64, tx: &str) -> Trade {
    Trade {
        proxy_wallet: "0xwallet".into(),
        side: "BUY".into(),
        asset: "asset".into(),
        condition_id: condition.into(),
        size: dec!(100),
        price: dec!(0.30),
        timestamp,
        title: "BTC".into(),
        slug: "btc-updown-5m-1784241900".into(),
        outcome: "Up".into(),
        transaction_hash: tx.into(),
    }
}

#[test]
fn first_successful_snapshot_primes_without_copying_history() {
    let engine = StrategyEngine::new(StrategyConfig::FirstLargeBuy {
        wallet: "0xwallet".into(),
        minimum_notional: dec!(25),
        minimum_lead_seconds: 90,
        estimated_win_probability: dec!(0.60),
    });
    let mut watcher = WalletWatcher::new(engine, 100).unwrap();
    assert!(!watcher.is_primed());
    assert!(
        watcher
            .process_snapshot(vec![trade("old", 1_784_241_950, "a")])
            .unwrap()
            .is_empty()
    );
    assert!(watcher.is_primed());
    assert!(
        watcher
            .process_snapshot(vec![trade("old", 1_784_241_950, "a")])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn new_market_after_priming_can_emit_exactly_once() {
    let engine = StrategyEngine::new(StrategyConfig::FirstLargeBuy {
        wallet: "0xwallet".into(),
        minimum_notional: dec!(25),
        minimum_lead_seconds: 90,
        estimated_win_probability: dec!(0.60),
    });
    let mut watcher = WalletWatcher::new(engine, 100).unwrap();
    watcher
        .process_snapshot(vec![trade("old", 1_784_241_950, "a")])
        .unwrap();
    let signals = watcher
        .process_snapshot(vec![
            trade("old", 1_784_241_950, "a"),
            trade("new", 1_784_241_960, "b"),
        ])
        .unwrap();
    assert_eq!(signals.len(), 1);
    assert_eq!(signals[0].condition_id, "new");
    assert!(
        watcher
            .process_snapshot(vec![trade("new", 1_784_241_960, "b")])
            .unwrap()
            .is_empty()
    );
}
