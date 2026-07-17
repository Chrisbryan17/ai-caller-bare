use polymarket_copybot::{
    PositionSizer, SizingConfig, crypto_taker_fee,
};
use rust_decimal_macros::dec;

#[test]
fn transaction_fee_is_rounded_to_five_decimal_places() {
    assert_eq!(crypto_taker_fee(dec!(100), dec!(0.50)).unwrap(), dec!(1.75));
    assert_eq!(crypto_taker_fee(dec!(10), dec!(0.41)).unwrap(), dec!(0.16933));
    assert_eq!(crypto_taker_fee(dec!(0.0001), dec!(0.01)).unwrap(), dec!(0));
}

#[test]
fn rounded_fee_never_pushes_sizing_over_the_hard_cap() {
    let bankroll = dec!(125);
    let cap = dec!(0.05);
    let decision = PositionSizer::new(SizingConfig {
        bankroll,
        estimated_win_probability: dec!(0.80),
        kelly_multiplier: dec!(0.25),
        max_bankroll_fraction: cap,
        minimum_shares: dec!(1),
    })
    .unwrap()
    .size(dec!(0.33))
    .unwrap();

    assert_eq!(decision.total_fee.scale(), 5);
    assert!(decision.total_cost <= bankroll * cap);
}
