use polymarket_copybot::{LaunchController, LaunchMode, PreflightStatus, WalletRegistry};
use rust_decimal_macros::dec;
use tempfile::tempdir;

#[test]
fn global_launch_gate_requires_time_resolutions_profit_health_and_wallet_qualification() {
    let dir = tempdir().unwrap();
    let registry = WalletRegistry::load_or_new(dir.path().join("r.json"), dec!(95)).unwrap();
    let mut controller = LaunchController::new(1_000, dec!(95));
    controller.record_resolved(dec!(0.50), true);
    controller.record_resolved(dec!(0.50), true);
    controller.record_resolved(dec!(0.50), true);
    controller.record_resolved(dec!(0.50), true);
    controller.record_resolved(dec!(0.50), true);
    controller.record_processing_error();
    let preflight = PreflightStatus::all_passed();

    assert_eq!(
        controller.evaluate(4_599, &registry, &[], &preflight, true),
        LaunchMode::Paper
    );
    assert_eq!(
        controller.evaluate(4_600, &registry, &[], &preflight, true),
        LaunchMode::Paper
    );
}

#[test]
fn failed_preflight_never_arms_live() {
    let dir = tempdir().unwrap();
    let registry = WalletRegistry::load_or_new(dir.path().join("r.json"), dec!(95)).unwrap();
    let controller = LaunchController::new(1_000, dec!(95));
    assert_eq!(
        controller.evaluate(
            10_000,
            &registry,
            &[],
            &PreflightStatus::failed("geoblocked"),
            true
        ),
        LaunchMode::Paper
    );
}
