use polymarket_copybot::{PositionSizer, SizingConfig, crypto_taker_fee_per_share};
use proptest::prelude::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

proptest! {
    #[test]
    fn fee_is_symmetric_around_half(millis in 1i64..1000) {
        let p = Decimal::new(millis, 3);
        let left = crypto_taker_fee_per_share(p).unwrap();
        let right = crypto_taker_fee_per_share(Decimal::ONE - p).unwrap();
        prop_assert_eq!(left, right);
        prop_assert!(left >= Decimal::ZERO);
        prop_assert!(left <= dec!(0.0175));
    }

    #[test]
    fn sizing_never_spends_over_hard_cap(cents in 5i64..61, bankroll_dollars in 20i64..501) {
        let price = Decimal::new(cents, 2);
        let bankroll = Decimal::new(bankroll_dollars, 0);
        let cap = dec!(0.05);
        let decision = PositionSizer::new(SizingConfig {
            bankroll,
            estimated_win_probability: dec!(0.80),
            kelly_multiplier: dec!(0.25),
            max_bankroll_fraction: cap,
            minimum_shares: dec!(1),
        }).unwrap().size(price).unwrap();
        prop_assert!(decision.total_cost <= bankroll * cap);
        prop_assert!(decision.shares.fract().is_zero());
        prop_assert!(decision.shares >= Decimal::ONE);
    }
}
