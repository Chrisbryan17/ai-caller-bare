use crate::{
    BoundedDedupe, CandidateSignal, CopybotError, Result, StrategyEngine, Trade, trade_key,
};

/// Per-wallet snapshot processor. Priming and deduplication are deliberately isolated so a failed
/// first poll for one wallet can never cause its historical fills to be copied after recovery.
#[derive(Clone, Debug)]
pub struct WalletWatcher {
    engine: StrategyEngine,
    dedupe: BoundedDedupe,
    primed: bool,
}

impl WalletWatcher {
    pub fn new(engine: StrategyEngine, dedupe_capacity: usize) -> Result<Self> {
        Ok(Self {
            engine,
            dedupe: BoundedDedupe::new(dedupe_capacity)?,
            primed: false,
        })
    }

    #[must_use]
    pub fn is_primed(&self) -> bool {
        self.primed
    }

    pub fn process_snapshot(&mut self, trades: Vec<Trade>) -> Result<Vec<CandidateSignal>> {
        let was_primed = self.primed;
        let mut signals = Vec::new();
        for trade in trades {
            if !self.dedupe.insert(trade_key(&trade)) {
                continue;
            }
            match self.engine.ingest(&trade) {
                Ok(Some(signal)) if was_primed => signals.push(signal),
                Ok(_) | Err(CopybotError::InvalidSlug(_)) => {}
                Err(error) => return Err(error),
            }
        }
        self.primed = true;
        Ok(signals)
    }
}
