use polymarket_copybot::conservative_cent_price;
use rust_decimal_macros::dec;

#[test]
fn price_cap_is_floored_to_a_valid_cent_without_exceeding_raw_cap() {
    assert_eq!(conservative_cent_price(dec!(0.196984)).unwrap(), dec!(0.19));
    assert_eq!(conservative_cent_price(dec!(0.21)).unwrap(), dec!(0.21));
    assert_eq!(conservative_cent_price(dec!(0.014)).unwrap(), dec!(0.01));
}

#[test]
fn price_cap_rejects_values_outside_tradeable_binary_range() {
    assert!(conservative_cent_price(dec!(0)).is_err());
    assert!(conservative_cent_price(dec!(1)).is_err());
    assert!(conservative_cent_price(dec!(-0.01)).is_err());
}
