use std::collections::{HashMap, HashSet, VecDeque};

use rust_decimal::Decimal;
use thiserror::Error;

use crate::{CandidateSignal, CopybotError, Outcome, Result};

const SECONDS_PER_DAY: i64 = 86_400;

#[must_use]
pub fn utc_day_index(epoch_seconds: i64) -> i64 {
    epoch_seconds.div_euclid(SECONDS_PER_DAY)
}

#[derive(Clone, Debug)]
pub struct BoundedDedupe {
    capacity: usize,
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl BoundedDedupe {
    pub fn new(capacity: usize) -> Result<Self> {
        if capacity == 0 {
            return Err(CopybotError::InvalidConfiguration(
                "dedupe capacity must be positive".into(),
            ));
        }
        Ok(Self {
            capacity,
            order: VecDeque::with_capacity(capacity),
            set: HashSet::with_capacity(capacity),
        })
    }

    pub fn insert(&mut self, key: String) -> bool {
        if self.set.contains(&key) {
            return false;
        }
        if self.order.len() == self.capacity
            && let Some(old) = self.order.pop_front()
        {
            self.set.remove(&old);
        }
        self.set.insert(key.clone());
        self.order.push_back(key);
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiskConfig {
    pub minimum_lead_seconds: i64,
    pub max_open_markets: usize,
    pub max_daily_capital_at_risk: Decimal,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RiskReject {
    #[error("signal is stale")]
    Stale,
    #[error("market already reserved")]
    DuplicateMarket,
    #[error("wallet signals conflict on outcome")]
    ConflictingOutcome,
    #[error("maximum open markets reached")]
    ExposureLimit,
    #[error("daily capital-at-risk limit reached")]
    DailyRiskLimit,
}

#[derive(Clone, Debug)]
pub struct RiskArbiter {
    config: RiskConfig,
    open: HashSet<String>,
    outcomes: HashMap<String, Outcome>,
    daily: Decimal,
}

impl RiskArbiter {
    pub fn new(config: RiskConfig) -> Result<Self> {
        if config.minimum_lead_seconds < 0
            || config.max_open_markets == 0
            || config.max_daily_capital_at_risk <= Decimal::ZERO
        {
            return Err(CopybotError::InvalidConfiguration(
                "invalid risk configuration".into(),
            ));
        }
        Ok(Self {
            config,
            open: HashSet::new(),
            outcomes: HashMap::new(),
            daily: Decimal::ZERO,
        })
    }

    pub fn reserve(
        &mut self,
        signal: &CandidateSignal,
        now: i64,
    ) -> std::result::Result<(), RiskReject> {
        if signal.market_end_epoch - now < self.config.minimum_lead_seconds {
            return Err(RiskReject::Stale);
        }
        if let Some(old) = self.outcomes.get(&signal.condition_id) {
            return Err(if *old == signal.outcome {
                RiskReject::DuplicateMarket
            } else {
                RiskReject::ConflictingOutcome
            });
        }
        if self.open.len() >= self.config.max_open_markets {
            return Err(RiskReject::ExposureLimit);
        }
        if self.daily >= self.config.max_daily_capital_at_risk {
            return Err(RiskReject::DailyRiskLimit);
        }
        self.outcomes
            .insert(signal.condition_id.clone(), signal.outcome);
        self.open.insert(signal.condition_id.clone());
        Ok(())
    }

    pub fn record_capital_at_risk(
        &mut self,
        condition: &str,
        amount: Decimal,
    ) -> std::result::Result<(), RiskReject> {
        if !self.open.contains(condition) {
            return Err(RiskReject::DuplicateMarket);
        }
        if self.daily + amount > self.config.max_daily_capital_at_risk {
            self.release(condition);
            return Err(RiskReject::DailyRiskLimit);
        }
        self.daily += amount;
        Ok(())
    }

    /// Releases active exposure while retaining the selected direction until the market finishes.
    pub fn release(&mut self, condition: &str) {
        self.open.remove(condition);
    }

    /// Removes all state for a finished market so the long-running ledger stays bounded.
    pub fn forget_market(&mut self, condition: &str) {
        self.open.remove(condition);
        self.outcomes.remove(condition);
    }

    pub fn reset_daily_risk(&mut self) {
        self.daily = Decimal::ZERO;
    }
}
