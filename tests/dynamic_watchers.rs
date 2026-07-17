use polymarket_copybot::{DynamicWatcherSet, StrategyFamily, Trade, WatcherSpec};
use rust_decimal_macros::dec;

fn trade(condition: &str, tx: &str) -> Trade {
    Trade {
        proxy_wallet: "0xwallet".into(),
        side: "BUY".into(),
        asset: "asset".into(),
        condition_id: condition.into(),
        size: dec!(100),
        price: dec!(0.30),
        timestamp: 1_784_241_950,
        title: "BTC".into(),
        slug: "btc-updown-5m-1784241900".into(),
        outcome: "Up".into(),
        transaction_hash: tx.into(),
    }
}

#[test]
fn newly_added_and_restarted_watchers_prime_before_emitting() {
    let spec = WatcherSpec {
        wallet: "0xwallet".into(),
        family: StrategyFamily::FirstLargeBuy,
        estimated_win_probability: dec!(0.65),
    };
    let mut set = DynamicWatcherSet::new(100).unwrap();
    set.synchronize(&[spec.clone()]).unwrap();
    assert!(
        set.process_snapshot("0xwallet", vec![trade("old", "a")])
            .unwrap()
            .is_empty()
    );
    assert!(set.is_primed("0xwallet"));
    let signals = set
        .process_snapshot("0xwallet", vec![trade("new", "b")])
        .unwrap();
    assert_eq!(signals.len(), 1);

    let mut restarted = DynamicWatcherSet::new(100).unwrap();
    restarted.synchronize(&[spec]).unwrap();
    assert!(
        restarted
            .process_snapshot("0xwallet", vec![trade("new", "b")])
            .unwrap()
            .is_empty()
    );
}
