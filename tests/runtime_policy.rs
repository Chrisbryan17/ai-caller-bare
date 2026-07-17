use polymarket_copybot::{
    DEFAULT_CLOB_API_BASE, ExecutionRoute, LaunchMode, WalletLifecycle, execution_route,
};

#[cfg(feature = "live-trading")]
use polymarket_copybot::live::PRODUCTION_CLOB_HOST;

#[test]
fn unqualified_wallets_are_always_shadow_paper_only() {
    for launch in [
        LaunchMode::Paper,
        LaunchMode::ArmedLive,
        LaunchMode::Live,
        LaunchMode::Halted,
    ] {
        assert_eq!(
            execution_route(WalletLifecycle::Quarantined, true, launch, true),
            ExecutionRoute::ShadowPaper
        );
    }
}

#[test]
fn live_route_requires_active_qualified_wallet_launch_gate_and_auto_live() {
    assert_eq!(
        execution_route(
            WalletLifecycle::ActiveLive,
            true,
            LaunchMode::ArmedLive,
            true
        ),
        ExecutionRoute::Live
    );
    assert_eq!(
        execution_route(
            WalletLifecycle::ActiveLive,
            true,
            LaunchMode::ArmedLive,
            false
        ),
        ExecutionRoute::ActivePaper
    );
    assert_eq!(
        execution_route(
            WalletLifecycle::PaperQualified,
            false,
            LaunchMode::ArmedLive,
            true
        ),
        ExecutionRoute::ShadowPaper
    );
}

#[test]
fn halted_mode_never_routes_to_live() {
    assert_eq!(
        execution_route(WalletLifecycle::ActiveLive, true, LaunchMode::Halted, true),
        ExecutionRoute::Ignore
    );
}

#[test]
fn production_clob_default_uses_current_host() {
    assert_eq!(DEFAULT_CLOB_API_BASE, "https://clob.polymarket.com");
}

#[cfg(feature = "live-trading")]
#[test]
fn live_sdk_connector_uses_the_same_canonical_production_host() {
    assert_eq!(PRODUCTION_CLOB_HOST, DEFAULT_CLOB_API_BASE);
}
