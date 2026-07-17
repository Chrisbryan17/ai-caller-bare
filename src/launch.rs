use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

use crate::WalletRegistry;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMode {
    Paper,
    ArmedLive,
    Live,
    Halted,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreflightFacts {
    pub geoblocked: bool,
    pub closed_only: bool,
    pub balance: Decimal,
    pub has_positive_allowance: bool,
    pub signer_authenticated: bool,
    pub open_orders: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreflightStatus {
    pub geoblock_ok: bool,
    pub balance_ok: bool,
    pub allowance_ok: bool,
    pub signer_ok: bool,
    pub open_orders_ok: bool,
    pub failure_reason: Option<String>,
}

impl PreflightStatus {
    #[must_use]
    pub fn all_passed() -> Self {
        Self {
            geoblock_ok: true,
            balance_ok: true,
            allowance_ok: true,
            signer_ok: true,
            open_orders_ok: true,
            failure_reason: None,
        }
    }

    #[must_use]
    pub fn failed(reason: impl Into<String>) -> Self {
        Self {
            geoblock_ok: false,
            balance_ok: false,
            allowance_ok: false,
            signer_ok: false,
            open_orders_ok: false,
            failure_reason: Some(reason.into()),
        }
    }

    #[must_use]
    pub fn from_facts(facts: &PreflightFacts, required_balance: Decimal) -> Self {
        let geoblock_ok = !facts.geoblocked && !facts.closed_only;
        let balance_ok = required_balance > Decimal::ZERO && facts.balance >= required_balance;
        let allowance_ok = facts.has_positive_allowance;
        let signer_ok = facts.signer_authenticated;
        let open_orders_ok = facts.open_orders == 0;
        let failure_reason = if facts.geoblocked {
            Some("geoblocked".into())
        } else if facts.closed_only {
            Some("account_closed_only".into())
        } else if !balance_ok {
            Some("insufficient_balance".into())
        } else if !allowance_ok {
            Some("missing_allowance".into())
        } else if !signer_ok {
            Some("signer_not_authenticated".into())
        } else if !open_orders_ok {
            Some("open_orders_present".into())
        } else {
            None
        };
        Self {
            geoblock_ok,
            balance_ok,
            allowance_ok,
            signer_ok,
            open_orders_ok,
            failure_reason,
        }
    }

    #[must_use]
    pub fn passed(&self) -> bool {
        self.geoblock_ok
            && self.balance_ok
            && self.allowance_ok
            && self.signer_ok
            && self.open_orders_ok
            && self.failure_reason.is_none()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LaunchGateSnapshot {
    pub elapsed_seconds: i64,
    pub resolved_positions: usize,
    pub net_pnl: Decimal,
    pub max_drawdown: Decimal,
    pub consecutive_losses: usize,
    pub processing_health: Decimal,
    pub qualified_wallets: usize,
    pub preflight_passed: bool,
    pub eligible: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LaunchController {
    started_epoch: i64,
    bankroll: Decimal,
    resolved_positions: usize,
    net_pnl: Decimal,
    peak_pnl: Decimal,
    max_drawdown: Decimal,
    consecutive_losses: usize,
    processing_errors: usize,
}

impl LaunchController {
    #[must_use]
    pub fn new(started_epoch: i64, bankroll: Decimal) -> Self {
        Self {
            started_epoch,
            bankroll,
            resolved_positions: 0,
            net_pnl: Decimal::ZERO,
            peak_pnl: Decimal::ZERO,
            max_drawdown: Decimal::ZERO,
            consecutive_losses: 0,
            processing_errors: 0,
        }
    }

    pub fn record_resolved(&mut self, pnl: Decimal, won: bool) {
        self.resolved_positions += 1;
        self.net_pnl += pnl;
        self.peak_pnl = self.peak_pnl.max(self.net_pnl);
        self.max_drawdown = self.max_drawdown.max(self.peak_pnl - self.net_pnl);
        if won {
            self.consecutive_losses = 0;
        } else {
            self.consecutive_losses += 1;
        }
    }

    pub fn record_processing_error(&mut self) {
        self.processing_errors = self.processing_errors.saturating_add(1);
    }

    #[must_use]
    pub fn snapshot(
        &self,
        now: i64,
        registry: &WalletRegistry,
        active_wallets: &[String],
        preflight: &PreflightStatus,
    ) -> LaunchGateSnapshot {
        let total = self.resolved_positions + self.processing_errors;
        let health = if total == 0 {
            Decimal::ZERO
        } else {
            Decimal::from(u64::try_from(self.resolved_positions).unwrap_or(u64::MAX))
                / Decimal::from(u64::try_from(total).unwrap_or(u64::MAX))
        };
        let qualified_wallets = active_wallets
            .iter()
            .filter(|wallet| {
                registry
                    .record(wallet)
                    .is_some_and(|record| record.is_paper_qualified())
            })
            .count();
        let elapsed_seconds = now - self.started_epoch;
        let eligible = elapsed_seconds >= 3_600
            && self.resolved_positions >= 5
            && self.net_pnl > Decimal::ZERO
            && self.max_drawdown <= self.bankroll * dec!(0.05)
            && self.consecutive_losses <= 3
            && health >= dec!(0.80)
            && qualified_wallets == active_wallets.len()
            && qualified_wallets > 0
            && preflight.passed();
        LaunchGateSnapshot {
            elapsed_seconds,
            resolved_positions: self.resolved_positions,
            net_pnl: self.net_pnl,
            max_drawdown: self.max_drawdown,
            consecutive_losses: self.consecutive_losses,
            processing_health: health,
            qualified_wallets,
            preflight_passed: preflight.passed(),
            eligible,
        }
    }

    #[must_use]
    pub fn evaluate(
        &self,
        now: i64,
        registry: &WalletRegistry,
        active_wallets: &[String],
        preflight: &PreflightStatus,
        auto_live: bool,
    ) -> LaunchMode {
        if auto_live
            && self
                .snapshot(now, registry, active_wallets, preflight)
                .eligible
        {
            LaunchMode::ArmedLive
        } else {
            LaunchMode::Paper
        }
    }
}
