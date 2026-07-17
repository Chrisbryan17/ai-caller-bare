use polymarket_copybot::{
    CandidateSignal, CopybotError, Outcome, PostedBuySummary, validated_live_buy_fill,
    validated_live_buy_fill_for_request,
};
use rust_decimal_macros::dec;

fn signal() -> CandidateSignal {
    CandidateSignal {
        wallet: "0xwallet".into(),
        condition_id: "condition".into(),
        asset_id: "123".into(),
        outcome: Outcome::Up,
        source_price: dec!(0.40),
        source_timestamp: 1,
        market_end_epoch: 200,
        slug: "btc-updown-5m-1784241900".into(),
        title: "BTC".into(),
        strategy: "test".into(),
        estimated_win_probability: dec!(0.60),
    }
}

#[test]
fn successful_limit_buy_uses_server_making_and_taking_amounts() {
    let fill = validated_live_buy_fill(
        signal(),
        dec!(0.41),
        PostedBuySummary {
            success: true,
            error_msg: None,
            making_amount: dec!(4.10),
            taking_amount: dec!(10),
            order_id: "order-1".into(),
        },
    )
    .unwrap();

    assert_eq!(fill.shares, dec!(10));
    assert_eq!(fill.fill_price, dec!(0.41));
    assert_eq!(fill.fee, dec!(0.16933));
    assert_eq!(fill.total_cost, dec!(4.26933));
    assert_eq!(fill.external_id.as_deref(), Some("order-1"));
    assert!(!fill.paper);
}

#[test]
fn live_response_requires_the_exact_requested_fok_share_count() {
    for taking_amount in [dec!(9), dec!(11)] {
        let result = validated_live_buy_fill_for_request(
            signal(),
            dec!(10),
            dec!(0.41),
            PostedBuySummary {
                success: true,
                error_msg: None,
                making_amount: taking_amount * dec!(0.41),
                taking_amount,
                order_id: "wrong-size".into(),
            },
        );
        assert!(matches!(result, Err(CopybotError::LiveExecution(_))));
    }
}

#[test]
fn live_response_rejects_false_success_error_text_and_zero_fill() {
    for summary in [
        PostedBuySummary {
            success: false,
            error_msg: None,
            making_amount: dec!(4.10),
            taking_amount: dec!(10),
            order_id: "a".into(),
        },
        PostedBuySummary {
            success: true,
            error_msg: Some("insufficient balance".into()),
            making_amount: dec!(4.10),
            taking_amount: dec!(10),
            order_id: "b".into(),
        },
        PostedBuySummary {
            success: true,
            error_msg: None,
            making_amount: dec!(0),
            taking_amount: dec!(0),
            order_id: "c".into(),
        },
    ] {
        assert!(matches!(
            validated_live_buy_fill(signal(), dec!(0.41), summary),
            Err(CopybotError::LiveExecution(_))
        ));
    }
}

#[test]
fn live_response_rejects_fill_price_outside_binary_range_or_above_cap() {
    let invalid_binary = PostedBuySummary {
        success: true,
        error_msg: None,
        making_amount: dec!(12),
        taking_amount: dec!(10),
        order_id: "bad-price".into(),
    };
    assert!(matches!(
        validated_live_buy_fill(signal(), dec!(0.99), invalid_binary),
        Err(CopybotError::InvalidPrice(_))
    ));

    let above_cap = PostedBuySummary {
        success: true,
        error_msg: None,
        making_amount: dec!(4.2),
        taking_amount: dec!(10),
        order_id: "above-cap".into(),
    };
    assert!(matches!(
        validated_live_buy_fill(signal(), dec!(0.41), above_cap),
        Err(CopybotError::LiveExecution(_))
    ));
}
