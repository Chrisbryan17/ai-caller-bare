use std::{path::Path, sync::Arc};

use rust_decimal::Decimal;
use serde::Serialize;
use thiserror::Error;
use tokio::{
    fs::{File, OpenOptions},
    io::AsyncWriteExt,
    sync::Mutex,
};

use crate::{CandidateSignal, ExecutionFill, StrategyFamily, WalletLifecycle};

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("journal I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("journal serialization failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JournalRecord {
    Signal {
        observed_epoch: i64,
        signal: CandidateSignal,
    },
    Fill {
        observed_epoch: i64,
        fill: ExecutionFill,
    },
    Rejection {
        observed_epoch: i64,
        condition_id: String,
        reason: String,
    },
    DiscoveryCycleCompleted {
        observed_epoch: i64,
        period_successes: usize,
        period_failures: usize,
        candidate_wallets: usize,
        evaluations: usize,
        candidate_failures: usize,
        failed_closed: bool,
        active_set_generation: u64,
    },
    CandidateEvaluated {
        observed_epoch: i64,
        wallet: String,
        family: StrategyFamily,
        eligible: bool,
        score: Decimal,
        resolved_signals: usize,
        net_pnl_one_cent: Decimal,
        rejection_reasons: Vec<String>,
        active_set_generation: u64,
    },
    WalletLifecycleChanged {
        observed_epoch: i64,
        wallet: String,
        from: WalletLifecycle,
        to: WalletLifecycle,
        reason: String,
        active_set_generation: u64,
    },
    ActiveSetChanged {
        observed_epoch: i64,
        previous_wallets: Vec<String>,
        wallets: Vec<String>,
        reason: String,
        active_set_generation: u64,
    },
    PaperResolved {
        observed_epoch: i64,
        wallet: String,
        condition_id: String,
        won: bool,
        pnl: Decimal,
        counts_global: bool,
        active_set_generation: u64,
    },
    LiveResolved {
        observed_epoch: i64,
        wallet: String,
        condition_id: String,
        won: bool,
        pnl: Decimal,
        active_set_generation: u64,
    },
}

#[derive(Clone, Debug)]
pub struct JsonlJournal {
    file: Arc<Mutex<File>>,
}

impl JsonlJournal {
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, JournalError> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
        })
    }

    pub async fn append(&self, record: &JournalRecord) -> Result<(), JournalError> {
        let mut encoded = serde_json::to_vec(record)?;
        encoded.push(b'\n');
        let mut file = self.file.lock().await;
        file.write_all(&encoded).await?;
        file.flush().await?;
        Ok(())
    }
}
