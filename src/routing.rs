use serde::{Deserialize, Serialize};

use crate::{LaunchMode, WalletLifecycle};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionRoute {
    Ignore,
    ShadowPaper,
    ActivePaper,
    Live,
}

#[must_use]
pub fn execution_route(
    lifecycle: WalletLifecycle,
    is_active: bool,
    launch_mode: LaunchMode,
    auto_live: bool,
) -> ExecutionRoute {
    if matches!(lifecycle, WalletLifecycle::Quarantined) {
        return ExecutionRoute::ShadowPaper;
    }
    if matches!(launch_mode, LaunchMode::Halted) {
        return ExecutionRoute::Ignore;
    }
    match lifecycle {
        WalletLifecycle::Quarantined => ExecutionRoute::ShadowPaper,
        WalletLifecycle::PaperQualified if !is_active => ExecutionRoute::ShadowPaper,
        WalletLifecycle::ActivePaper | WalletLifecycle::ActiveLive if is_active => {
            if auto_live
                && matches!(launch_mode, LaunchMode::ArmedLive | LaunchMode::Live)
                && matches!(lifecycle, WalletLifecycle::ActiveLive)
            {
                ExecutionRoute::Live
            } else {
                ExecutionRoute::ActivePaper
            }
        }
        WalletLifecycle::PaperQualified => ExecutionRoute::ShadowPaper,
        WalletLifecycle::Discovered
        | WalletLifecycle::Rejected
        | WalletLifecycle::Suspended
        | WalletLifecycle::ActivePaper
        | WalletLifecycle::ActiveLive => ExecutionRoute::Ignore,
    }
}
