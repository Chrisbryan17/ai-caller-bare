use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

use crate::{CandidateEvaluation, CopybotError, Result, StrategyFamily};

const REGISTRY_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WalletLifecycle {
    Discovered,
    Rejected,
    Quarantined,
    PaperQualified,
    ActivePaper,
    ActiveLive,
    Suspended,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SuspensionReason {
    ConsecutiveLosses,
    Drawdown,
    StaleSignals,
    Inactivity,
    ReplayFailure,
    StrategyDrift,
    Manual,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PaperOutcome {
    pub condition_id: String,
    pub resolved_epoch: i64,
    pub pnl: Decimal,
    pub won: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LiveOutcome {
    pub condition_id: String,
    pub resolved_epoch: i64,
    pub pnl: Decimal,
    pub won: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WalletRecord {
    pub wallet: String,
    pub lifecycle: WalletLifecycle,
    pub family: Option<StrategyFamily>,
    pub estimated_win_probability: Decimal,
    pub evaluation: Option<CandidateEvaluation>,
    pub quarantine_started_epoch: Option<i64>,
    pub prime_epoch: Option<i64>,
    pub retry_after_epoch: Option<i64>,
    pub paper_resolved: usize,
    pub paper_wins: usize,
    pub paper_losses: usize,
    pub paper_net_pnl: Decimal,
    pub paper_peak_pnl: Decimal,
    pub paper_max_drawdown: Decimal,
    pub paper_consecutive_losses: usize,
    #[serde(default)]
    pub paper_max_loss_streak: usize,
    pub paper_processing_errors: usize,
    pub paper_outcomes: Vec<PaperOutcome>,
    #[serde(default)]
    pub live_resolved: usize,
    #[serde(default)]
    pub live_wins: usize,
    #[serde(default)]
    pub live_losses: usize,
    #[serde(default)]
    pub live_net_pnl: Decimal,
    #[serde(default)]
    pub live_peak_pnl: Decimal,
    #[serde(default)]
    pub live_max_drawdown: Decimal,
    #[serde(default)]
    pub live_consecutive_losses: usize,
    #[serde(default)]
    pub live_outcomes: Vec<LiveOutcome>,
    pub suspension_reason: Option<SuspensionReason>,
}

impl WalletRecord {
    fn new(wallet: String) -> Self {
        Self {
            wallet,
            lifecycle: WalletLifecycle::Discovered,
            family: None,
            estimated_win_probability: dec!(0.50),
            evaluation: None,
            quarantine_started_epoch: None,
            prime_epoch: None,
            retry_after_epoch: None,
            paper_resolved: 0,
            paper_wins: 0,
            paper_losses: 0,
            paper_net_pnl: Decimal::ZERO,
            paper_peak_pnl: Decimal::ZERO,
            paper_max_drawdown: Decimal::ZERO,
            paper_consecutive_losses: 0,
            paper_max_loss_streak: 0,
            paper_processing_errors: 0,
            paper_outcomes: Vec::new(),
            live_resolved: 0,
            live_wins: 0,
            live_losses: 0,
            live_net_pnl: Decimal::ZERO,
            live_peak_pnl: Decimal::ZERO,
            live_max_drawdown: Decimal::ZERO,
            live_consecutive_losses: 0,
            live_outcomes: Vec::new(),
            suspension_reason: None,
        }
    }

    #[must_use]
    pub fn score(&self) -> Decimal {
        self.evaluation
            .as_ref()
            .map_or(Decimal::MIN, |evaluation| evaluation.score)
    }

    #[must_use]
    pub fn is_paper_qualified(&self) -> bool {
        matches!(
            self.lifecycle,
            WalletLifecycle::PaperQualified
                | WalletLifecycle::ActivePaper
                | WalletLifecycle::ActiveLive
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RegistryState {
    pub schema_version: u32,
    pub bankroll: Decimal,
    pub max_risk_fraction: Decimal,
    pub daily_capital_at_risk_limit: Decimal,
    pub active_set_generation: u64,
    pub last_successful_discovery_epoch: Option<i64>,
    pub active_wallets: Vec<String>,
    pub records: BTreeMap<String, WalletRecord>,
}

#[derive(Clone, Debug)]
pub struct WalletRegistry {
    path: PathBuf,
    state: RegistryState,
}

impl WalletRegistry {
    pub fn load_or_new(path: impl AsRef<Path>, bankroll: Decimal) -> Result<Self> {
        if bankroll <= Decimal::ZERO {
            return Err(CopybotError::InvalidConfiguration(
                "registry bankroll must be positive".into(),
            ));
        }
        let path = path.as_ref().to_path_buf();
        let state = if path.exists() {
            let bytes = fs::read(&path)?;
            let state: RegistryState = serde_json::from_slice(&bytes)?;
            validate_state(&state)?;
            state
        } else {
            RegistryState {
                schema_version: REGISTRY_SCHEMA_VERSION,
                bankroll,
                max_risk_fraction: dec!(0.05),
                daily_capital_at_risk_limit: bankroll * dec!(0.10),
                active_set_generation: 0,
                last_successful_discovery_epoch: None,
                active_wallets: Vec::new(),
                records: BTreeMap::new(),
            }
        };
        Ok(Self { path, state })
    }

    #[must_use]
    pub fn state(&self) -> &RegistryState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut RegistryState {
        &mut self.state
    }

    #[must_use]
    pub fn record(&self, wallet: &str) -> Option<&WalletRecord> {
        self.state.records.get(&wallet.to_ascii_lowercase())
    }

    pub fn record_mut(&mut self, wallet: &str) -> Option<&mut WalletRecord> {
        self.state.records.get_mut(&wallet.to_ascii_lowercase())
    }

    pub fn upsert_evaluation(&mut self, evaluation: CandidateEvaluation, now: i64) {
        let wallet = evaluation.wallet.to_ascii_lowercase();
        let mut remove_active = false;
        {
            let record = self
                .state
                .records
                .entry(wallet.clone())
                .or_insert_with(|| WalletRecord::new(wallet.clone()));
            record.family = Some(evaluation.family);
            record.estimated_win_probability = evaluation.estimated_win_probability;
            record.evaluation = Some(evaluation.clone());
            if evaluation.eligible {
                if matches!(
                    record.lifecycle,
                    WalletLifecycle::Discovered
                        | WalletLifecycle::Rejected
                        | WalletLifecycle::Suspended
                ) {
                    reset_paper(record);
                    record.lifecycle = WalletLifecycle::Quarantined;
                    record.quarantine_started_epoch = Some(now);
                    record.retry_after_epoch = None;
                    record.suspension_reason = None;
                }
            } else if record.is_paper_qualified() {
                record.lifecycle = WalletLifecycle::Suspended;
                record.suspension_reason = Some(SuspensionReason::ReplayFailure);
                record.retry_after_epoch = Some(now + 86_400);
                remove_active = true;
            } else {
                record.lifecycle = WalletLifecycle::Rejected;
                record.retry_after_epoch = Some(now + 86_400);
            }
        }
        if remove_active {
            let before = self.state.active_wallets.len();
            self.state
                .active_wallets
                .retain(|active| !active.eq_ignore_ascii_case(&wallet));
            if self.state.active_wallets.len() != before {
                self.state.active_set_generation += 1;
            }
        }
    }

    pub fn mark_primed(&mut self, wallet: &str, epoch: i64) -> Result<()> {
        let record = self.record_mut(wallet).ok_or_else(|| {
            CopybotError::InvalidConfiguration("cannot prime unknown wallet".into())
        })?;
        record.prime_epoch = Some(epoch);
        Ok(())
    }

    pub fn record_processing_error(&mut self, wallet: &str) -> Result<()> {
        let record = self.record_mut(wallet).ok_or_else(|| {
            CopybotError::InvalidConfiguration("unknown wallet processing error".into())
        })?;
        record.paper_processing_errors = record.paper_processing_errors.saturating_add(1);
        Ok(())
    }

    pub fn record_paper_outcome(&mut self, wallet: &str, outcome: PaperOutcome) -> Result<()> {
        let record = self.record_mut(wallet).ok_or_else(|| {
            CopybotError::InvalidConfiguration("paper outcome wallet is unknown".into())
        })?;
        if record
            .paper_outcomes
            .iter()
            .any(|existing| existing.condition_id == outcome.condition_id)
        {
            return Ok(());
        }
        record.paper_resolved += 1;
        record.paper_net_pnl += outcome.pnl;
        record.paper_peak_pnl = record.paper_peak_pnl.max(record.paper_net_pnl);
        record.paper_max_drawdown = record
            .paper_max_drawdown
            .max(record.paper_peak_pnl - record.paper_net_pnl);
        if outcome.won {
            record.paper_wins += 1;
            record.paper_consecutive_losses = 0;
        } else {
            record.paper_losses += 1;
            record.paper_consecutive_losses += 1;
            record.paper_max_loss_streak = record
                .paper_max_loss_streak
                .max(record.paper_consecutive_losses);
        }
        record.paper_outcomes.push(outcome);
        Ok(())
    }

    pub fn record_live_outcome(&mut self, wallet: &str, outcome: LiveOutcome) -> Result<bool> {
        let bankroll = self.state.bankroll;
        let normalized = wallet.to_ascii_lowercase();
        let suspension = {
            let record = self.record_mut(&normalized).ok_or_else(|| {
                CopybotError::InvalidConfiguration("live outcome wallet is unknown".into())
            })?;
            if record
                .live_outcomes
                .iter()
                .any(|existing| existing.condition_id == outcome.condition_id)
            {
                return Ok(false);
            }
            record.live_resolved += 1;
            record.live_net_pnl += outcome.pnl;
            record.live_peak_pnl = record.live_peak_pnl.max(record.live_net_pnl);
            record.live_max_drawdown = record
                .live_max_drawdown
                .max(record.live_peak_pnl - record.live_net_pnl);
            if outcome.won {
                record.live_wins += 1;
                record.live_consecutive_losses = 0;
            } else {
                record.live_losses += 1;
                record.live_consecutive_losses += 1;
            }
            record.live_outcomes.push(outcome);
            let reason = if record.live_consecutive_losses >= 2 {
                Some(SuspensionReason::ConsecutiveLosses)
            } else if record.live_max_drawdown > bankroll * dec!(0.05) {
                Some(SuspensionReason::Drawdown)
            } else {
                None
            };
            if let Some(reason) = reason.clone() {
                record.lifecycle = WalletLifecycle::Suspended;
                record.suspension_reason = Some(reason);
            }
            reason
        };
        if suspension.is_some() {
            let before = self.state.active_wallets.len();
            self.state
                .active_wallets
                .retain(|active| !active.eq_ignore_ascii_case(&normalized));
            if self.state.active_wallets.len() != before {
                self.state.active_set_generation += 1;
            }
        }
        Ok(suspension.is_some())
    }

    pub fn refresh_qualification(&mut self, wallet: &str, now: i64) -> Result<bool> {
        let bankroll = self.state.bankroll;
        let record = self.record_mut(wallet).ok_or_else(|| {
            CopybotError::InvalidConfiguration("qualification wallet is unknown".into())
        })?;
        if !matches!(record.lifecycle, WalletLifecycle::Quarantined) {
            return Ok(record.is_paper_qualified());
        }
        let elapsed = now - record.quarantine_started_epoch.unwrap_or(now);
        let total_processing = record.paper_resolved + record.paper_processing_errors;
        let health = if total_processing == 0 {
            Decimal::ZERO
        } else {
            Decimal::from(u64::try_from(record.paper_resolved).unwrap_or(u64::MAX))
                / Decimal::from(u64::try_from(total_processing).unwrap_or(u64::MAX))
        };
        let qualified = elapsed >= 3_600
            && record.paper_resolved >= 5
            && record.paper_net_pnl > Decimal::ZERO
            && record.paper_max_drawdown <= bankroll * dec!(0.05)
            && record.paper_max_loss_streak <= 3
            && health >= dec!(0.80);
        if qualified {
            record.lifecycle = WalletLifecycle::PaperQualified;
        }
        Ok(qualified)
    }

    pub fn force_paper_qualified(&mut self, wallet: &str) -> Result<()> {
        let record = self.record_mut(wallet).ok_or_else(|| {
            CopybotError::InvalidConfiguration("cannot qualify unknown wallet".into())
        })?;
        record.lifecycle = WalletLifecycle::PaperQualified;
        Ok(())
    }

    pub fn suspend(&mut self, wallet: &str, reason: SuspensionReason) -> Result<()> {
        let record = self.record_mut(wallet).ok_or_else(|| {
            CopybotError::InvalidConfiguration("cannot suspend unknown wallet".into())
        })?;
        record.lifecycle = WalletLifecycle::Suspended;
        record.suspension_reason = Some(reason);
        Ok(())
    }

    #[must_use]
    pub fn qualified_records(&self) -> Vec<&WalletRecord> {
        self.state
            .records
            .values()
            .filter(|record| record.is_paper_qualified())
            .collect()
    }

    pub fn apply_active_wallets(&mut self, wallets: Vec<String>, live: bool) {
        let normalized: Vec<String> = wallets
            .into_iter()
            .map(|wallet| wallet.to_ascii_lowercase())
            .collect();
        for record in self.state.records.values_mut() {
            if normalized.contains(&record.wallet) && record.is_paper_qualified() {
                record.lifecycle = if live {
                    WalletLifecycle::ActiveLive
                } else {
                    WalletLifecycle::ActivePaper
                };
            } else if matches!(
                record.lifecycle,
                WalletLifecycle::ActiveLive | WalletLifecycle::ActivePaper
            ) {
                record.lifecycle = WalletLifecycle::PaperQualified;
            }
        }
        if self.state.active_wallets != normalized {
            self.state.active_set_generation += 1;
            self.state.active_wallets = normalized;
        }
    }

    pub fn save_atomic(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = self.path.with_extension("tmp");
        let encoded = serde_json::to_vec_pretty(&self.state)?;
        let mut file = File::create(&temporary)?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        fs::rename(temporary, &self.path)?;
        Ok(())
    }
}

fn validate_state(state: &RegistryState) -> Result<()> {
    if state.schema_version != REGISTRY_SCHEMA_VERSION {
        return Err(CopybotError::InvalidConfiguration(format!(
            "unsupported registry schema {}",
            state.schema_version
        )));
    }
    if state.bankroll <= Decimal::ZERO
        || state.max_risk_fraction != dec!(0.05)
        || state.daily_capital_at_risk_limit <= Decimal::ZERO
    {
        return Err(CopybotError::InvalidConfiguration(
            "registry risk metadata is invalid".into(),
        ));
    }
    Ok(())
}

fn reset_paper(record: &mut WalletRecord) {
    record.prime_epoch = None;
    record.paper_resolved = 0;
    record.paper_wins = 0;
    record.paper_losses = 0;
    record.paper_net_pnl = Decimal::ZERO;
    record.paper_peak_pnl = Decimal::ZERO;
    record.paper_max_drawdown = Decimal::ZERO;
    record.paper_consecutive_losses = 0;
    record.paper_max_loss_streak = 0;
    record.paper_processing_errors = 0;
    record.paper_outcomes.clear();
}
