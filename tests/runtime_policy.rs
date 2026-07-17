use polymarket_copybot::{ExecutionRoute, LaunchMode, WalletLifecycle, execution_route};

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
