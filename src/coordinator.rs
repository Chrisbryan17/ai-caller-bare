use std::collections::{HashMap, HashSet};

use crate::CandidateSignal;

pub const PRIMARY_WALLET: &str = "0x208326efd5d051c59631ed626848b150b8d8259c";
pub const SECONDARY_WALLET: &str = "0x45230b4fb12569efcc908b4d22c3cee4a19429e2";
pub const SPECIALIST_WALLET: &str = "0xb89d0b6e96e790afa900b53476b8f267a94d1d4f";

/// Select at most one position from one polling cycle.
///
/// Signals in the same market must agree. Agreement never multiplies exposure; the configured
/// primary wallet wins a tie. A conflicting market is skipped and the next chronological market is
/// considered.
#[must_use]
pub fn select_signal(signals: Vec<CandidateSignal>) -> Option<CandidateSignal> {
    select_signal_with_priority(
        signals,
        &[
            PRIMARY_WALLET.into(),
            SECONDARY_WALLET.into(),
            SPECIALIST_WALLET.into(),
        ],
    )
}

/// Select at most one signal using a dynamic active-set priority list.
///
/// All signals for a market must agree on outcome. Agreement never changes sizing or creates more
/// than one request. Wallets not present in `priorities` sort after listed wallets, then by address.
#[must_use]
pub fn select_signal_with_priority(
    signals: Vec<CandidateSignal>,
    priorities: &[String],
) -> Option<CandidateSignal> {
    let mut groups: HashMap<String, Vec<CandidateSignal>> = HashMap::new();
    for signal in signals {
        groups
            .entry(signal.condition_id.clone())
            .or_default()
            .push(signal);
    }
    let mut groups: Vec<_> = groups.into_values().collect();
    groups.sort_by(|left, right| {
        let left_timestamp = left
            .iter()
            .map(|signal| signal.source_timestamp)
            .min()
            .unwrap_or(i64::MAX);
        let right_timestamp = right
            .iter()
            .map(|signal| signal.source_timestamp)
            .min()
            .unwrap_or(i64::MAX);
        left_timestamp
            .cmp(&right_timestamp)
            .then_with(|| left[0].condition_id.cmp(&right[0].condition_id))
    });
    for mut rows in groups {
        let outcomes: HashSet<_> = rows.iter().map(|signal| signal.outcome).collect();
        if outcomes.len() != 1 {
            continue;
        }
        rows.sort_by(|left, right| {
            dynamic_wallet_priority(&left.wallet, priorities)
                .cmp(&dynamic_wallet_priority(&right.wallet, priorities))
                .then_with(|| left.wallet.cmp(&right.wallet))
                .then_with(|| left.source_timestamp.cmp(&right.source_timestamp))
        });
        return rows.into_iter().next();
    }
    None
}

fn dynamic_wallet_priority(wallet: &str, priorities: &[String]) -> usize {
    priorities
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(wallet))
        .unwrap_or(usize::MAX)
}
