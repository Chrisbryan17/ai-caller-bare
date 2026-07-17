use std::collections::{BTreeMap, HashMap};

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

use crate::{SignalTrace, WalletRecord, WalletRegistry};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CorrelationEvidence {
    pub shared_condition_fraction: Decimal,
    pub same_outcome_fraction: Decimal,
    pub median_timestamp_distance_seconds: i64,
    pub strongly_correlated: bool,
}

pub struct CorrelationAnalyzer;

impl CorrelationAnalyzer {
    #[must_use]
    pub fn analyze(left: &[SignalTrace], right: &[SignalTrace]) -> CorrelationEvidence {
        let left = trace_map(left);
        let right = trace_map(right);
        let denominator = left.len().min(right.len());
        if denominator == 0 {
            return CorrelationEvidence {
                shared_condition_fraction: Decimal::ZERO,
                same_outcome_fraction: Decimal::ZERO,
                median_timestamp_distance_seconds: i64::MAX,
                strongly_correlated: false,
            };
        }
        let mut shared = 0usize;
        let mut same_outcome = 0usize;
        let mut distances = Vec::new();
        for (condition, left_signal) in &left {
            if let Some(right_signal) = right.get(condition) {
                shared += 1;
                if left_signal.outcome == right_signal.outcome {
                    same_outcome += 1;
                }
                distances.push((left_signal.timestamp - right_signal.timestamp).abs());
            }
        }
        let shared_fraction = decimal(shared) / decimal(denominator);
        let same_fraction = if shared == 0 {
            Decimal::ZERO
        } else {
            decimal(same_outcome) / decimal(shared)
        };
        distances.sort_unstable();
        let median = distances
            .get(distances.len().saturating_sub(1) / 2)
            .copied()
            .unwrap_or(i64::MAX);
        CorrelationEvidence {
            shared_condition_fraction: shared_fraction,
            same_outcome_fraction: same_fraction,
            median_timestamp_distance_seconds: median,
            strongly_correlated: shared_fraction >= dec!(0.70)
                && same_fraction >= dec!(0.80)
                && median <= 5,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RotationContext {
    pub position_open: bool,
    pub submission_in_flight: bool,
    pub state_persisted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveSetProposal {
    pub wallets: Vec<String>,
    pub deferred: bool,
    pub changed: bool,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct ActiveSetManager {
    max_active: usize,
    challenger_multiplier: Decimal,
}

impl Default for ActiveSetManager {
    fn default() -> Self {
        Self {
            max_active: 3,
            challenger_multiplier: dec!(1.10),
        }
    }
}

impl ActiveSetManager {
    #[must_use]
    pub fn with_max_active(max_active: usize) -> Self {
        Self {
            max_active: max_active.clamp(1, 3),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn propose(
        &self,
        registry: &WalletRegistry,
        current: &[String],
        context: RotationContext,
    ) -> ActiveSetProposal {
        let normalized_current: Vec<String> = current
            .iter()
            .map(|wallet| wallet.to_ascii_lowercase())
            .collect();
        if context.position_open || context.submission_in_flight || !context.state_persisted {
            return ActiveSetProposal {
                wallets: normalized_current,
                deferred: true,
                changed: false,
                reason: "unsafe_rotation_point".into(),
            };
        }

        let records: HashMap<String, &WalletRecord> = registry
            .qualified_records()
            .into_iter()
            .map(|record| (record.wallet.clone(), record))
            .collect();
        let mut selected: Vec<String> = normalized_current
            .iter()
            .filter(|wallet| records.contains_key(*wallet))
            .take(self.max_active)
            .cloned()
            .collect();
        let mut candidates: Vec<&WalletRecord> = records.values().copied().collect();
        candidates.sort_by(|left, right| {
            right
                .score()
                .cmp(&left.score())
                .then_with(|| left.wallet.cmp(&right.wallet))
        });

        for candidate in &candidates {
            if selected.len() >= self.max_active {
                break;
            }
            if !selected.contains(&candidate.wallet)
                && !is_correlated_with_selected(candidate, &selected, &records)
            {
                selected.push(candidate.wallet.clone());
            }
        }

        if selected.len() >= self.max_active
            && let Some(challenger) = candidates.iter().find(|candidate| {
                !selected.contains(&candidate.wallet)
                    && !is_correlated_with_selected(candidate, &selected, &records)
            })
        {
            let lowest = selected
                .iter()
                .enumerate()
                .filter_map(|(index, wallet)| {
                    records.get(wallet).map(|record| (index, record.score()))
                })
                .min_by(|left, right| left.1.cmp(&right.1));
            if let Some((index, incumbent_score)) = lowest
                && challenger.score() > incumbent_score * self.challenger_multiplier
            {
                selected[index] = challenger.wallet.clone();
            }
        }

        selected.sort_by(|left, right| {
            let left_score = records
                .get(left)
                .map_or(Decimal::MIN, |record| record.score());
            let right_score = records
                .get(right)
                .map_or(Decimal::MIN, |record| record.score());
            right_score.cmp(&left_score).then_with(|| left.cmp(right))
        });
        let changed = selected != normalized_current;
        ActiveSetProposal {
            wallets: selected,
            deferred: false,
            changed,
            reason: if changed {
                "qualified_rotation".into()
            } else {
                "incumbents_preserved".into()
            },
        }
    }
}

fn trace_map(traces: &[SignalTrace]) -> BTreeMap<String, SignalTrace> {
    let mut mapped = BTreeMap::new();
    for trace in traces {
        mapped
            .entry(trace.condition_id.clone())
            .and_modify(|existing: &mut SignalTrace| {
                if trace.timestamp < existing.timestamp {
                    *existing = trace.clone();
                }
            })
            .or_insert_with(|| trace.clone());
    }
    mapped
}

fn is_correlated_with_selected(
    candidate: &WalletRecord,
    selected: &[String],
    records: &HashMap<String, &WalletRecord>,
) -> bool {
    let candidate_trace = candidate
        .evaluation
        .as_ref()
        .map(|evaluation| evaluation.metrics.signal_trace.as_slice())
        .unwrap_or_default();
    selected.iter().any(|wallet| {
        let selected_trace = records
            .get(wallet)
            .and_then(|record| record.evaluation.as_ref())
            .map(|evaluation| evaluation.metrics.signal_trace.as_slice())
            .unwrap_or_default();
        CorrelationAnalyzer::analyze(candidate_trace, selected_trace).strongly_correlated
    })
}

fn decimal(value: usize) -> Decimal {
    Decimal::from(u64::try_from(value).unwrap_or(u64::MAX))
}
