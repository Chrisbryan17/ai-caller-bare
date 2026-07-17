use polymarket_copybot::{CopybotError, validate_live_ack};

#[test]
fn paper_mode_never_needs_live_acknowledgement() {
    validate_live_ack(false, None).unwrap();
}

#[test]
fn live_mode_rejects_every_inexact_acknowledgement() {
    assert!(matches!(
        validate_live_ack(true, None),
        Err(CopybotError::LiveTradingNotAcknowledged)
    ));
    assert!(matches!(
        validate_live_ack(true, Some("yes")),
        Err(CopybotError::LiveTradingNotAcknowledged)
    ));
}

#[test]
fn exact_acknowledgement_still_requires_compile_time_live_feature() {
    let result = validate_live_ack(true, Some("I_UNDERSTAND_REAL_MONEY"));
    #[cfg(feature = "live-trading")]
    assert!(result.is_ok());
    #[cfg(not(feature = "live-trading"))]
    assert!(matches!(result, Err(CopybotError::LiveTradingNotCompiled)));
}
