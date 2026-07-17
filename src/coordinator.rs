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
    let mut groups: HashMap<String, Vec<CandidateSignal>> = HashMap::new();
    for signal in signals {
        groups
            .entry(signal.condition_id.clone())
            .or_default()
            .push(signal);
    }
    let mut groups: Vec<_> = groups.into_values().collect();
    groups.sort_by_key(|rows| {
        rows.iter()
            .map(|signal| signal.source_timestamp)
            .min()
            .unwrap_or(i64::MAX)
    });
    for mut rows in groups {
        let outcomes: HashSet<_> = rows.iter().map(|signal| signal.outcome).collect();
        if outcomes.len() != 1 {
            continue;
        }
        rows.sort_by_key(|signal| wallet_priority(&signal.wallet));
        return rows.into_iter().next();
    }
    None
}

fn wallet_priority(wallet: &str) -> u8 {
    if wallet.eq_ignore_ascii_case(PRIMARY_WALLET) {
        0
    } else if wallet.eq_ignore_ascii_case(SECONDARY_WALLET) {
        1
    } else if wallet.eq_ignore_ascii_case(SPECIALIST_WALLET) {
        2
    } else {
        3
    }
}
