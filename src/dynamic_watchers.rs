use std::collections::{HashMap, HashSet};

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

use crate::{
    CandidateSignal, CopybotError, Result, StrategyConfig, StrategyEngine, StrategyFamily, Trade,
    WalletWatcher,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WatcherSpec {
    pub wallet: String,
    pub family: StrategyFamily,
    pub estimated_win_probability: Decimal,
}

#[derive(Clone, Debug)]
struct Entry {
    spec: WatcherSpec,
    watcher: WalletWatcher,
}

#[derive(Clone, Debug)]
pub struct DynamicWatcherSet {
    entries: HashMap<String, Entry>,
    dedupe_capacity: usize,
}

impl DynamicWatcherSet {
    pub fn new(dedupe_capacity: usize) -> Result<Self> {
        if dedupe_capacity == 0 {
            return Err(CopybotError::InvalidConfiguration(
                "dynamic watcher dedupe capacity must be positive".into(),
            ));
        }
        Ok(Self {
            entries: HashMap::new(),
            dedupe_capacity,
        })
    }

    pub fn synchronize(&mut self, specs: &[WatcherSpec]) -> Result<()> {
        let wanted: HashSet<String> = specs
            .iter()
            .map(|spec| spec.wallet.to_ascii_lowercase())
            .collect();
        self.entries.retain(|wallet, _| wanted.contains(wallet));
        for requested in specs {
            let mut spec = requested.clone();
            spec.wallet.make_ascii_lowercase();
            let replace = self
                .entries
                .get(&spec.wallet)
                .is_none_or(|entry| entry.spec != spec);
            if replace {
                let watcher = WalletWatcher::new(
                    StrategyEngine::new(strategy_config(&spec)),
                    self.dedupe_capacity,
                )?;
                self.entries
                    .insert(spec.wallet.clone(), Entry { spec, watcher });
            }
        }
        Ok(())
    }

    pub fn process_snapshot(
        &mut self,
        wallet: &str,
        trades: Vec<Trade>,
    ) -> Result<Vec<CandidateSignal>> {
        let entry = self
            .entries
            .get_mut(&wallet.to_ascii_lowercase())
            .ok_or_else(|| {
                CopybotError::InvalidConfiguration("wallet is not in dynamic watcher set".into())
            })?;
        entry.watcher.process_snapshot(trades)
    }

    #[must_use]
    pub fn is_primed(&self, wallet: &str) -> bool {
        self.entries
            .get(&wallet.to_ascii_lowercase())
            .is_some_and(|entry| entry.watcher.is_primed())
    }

    #[must_use]
    pub fn wallets(&self) -> Vec<String> {
        let mut wallets: Vec<String> = self.entries.keys().cloned().collect();
        wallets.sort();
        wallets
    }
}

fn strategy_config(spec: &WatcherSpec) -> StrategyConfig {
    match spec.family {
        StrategyFamily::FirstLargeBuy => StrategyConfig::FirstLargeBuy {
            wallet: spec.wallet.clone(),
            minimum_notional: dec!(25),
            minimum_lead_seconds: 90,
            estimated_win_probability: spec.estimated_win_probability,
        },
        StrategyFamily::ConfirmedFlow => StrategyConfig::ConfirmedFlow {
            wallet: spec.wallet.clone(),
            minimum_cumulative_notional: dec!(100),
            minimum_directional_share: dec!(0.80),
            minimum_price: dec!(0.40),
            maximum_price: dec!(0.55),
            minimum_lead_seconds: 90,
            estimated_win_probability: spec.estimated_win_probability,
        },
    }
}
