use polymarket_copybot::{CandidateSignal, Outcome, select_signal};
use rust_decimal_macros::dec;

fn signal(wallet: &str, condition: &str, outcome: Outcome, timestamp: i64) -> CandidateSignal {
    CandidateSignal {
        wallet: wallet.into(),
        condition_id: condition.into(),
        asset_id: "asset".into(),
        outcome,
        source_price: dec!(0.40),
        source_timestamp: timestamp,
        market_end_epoch: timestamp + 120,
        slug: "btc-updown-5m-1784241900".into(),
        title: "BTC".into(),
        strategy: "test".into(),
        estimated_win_probability: dec!(0.60),
    }
}

#[test]
fn same_cycle_wallet_conflict_skips_market() {
    let signals = vec![
        signal("0x208326efd5d051c59631ed626848b150b8d8259c", "c", Outcome::Up, 10),
        signal("0x45230b4fb12569efcc908b4d22c3cee4a19429e2", "c", Outcome::Down, 10),
    ];
    assert!(select_signal(signals).is_none());
}

#[test]
fn agreement_is_one_position_and_primary_wins_tie() {
    let primary = "0x208326efd5d051c59631ed626848b150b8d8259c";
    let signals = vec![
        signal("0x45230b4fb12569efcc908b4d22c3cee4a19429e2", "c", Outcome::Up, 10),
        signal(primary, "c", Outcome::Up, 10),
    ];
    let selected = select_signal(signals).unwrap();
    assert_eq!(selected.wallet, primary);
}

#[test]
fn earliest_market_is_selected_before_later_market() {
    let selected = select_signal(vec![
        signal("later", "c2", Outcome::Up, 20),
        signal("earlier", "c1", Outcome::Up, 10),
    ]).unwrap();
    assert_eq!(selected.condition_id, "c1");
}
