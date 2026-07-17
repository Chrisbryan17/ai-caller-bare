use std::collections::{HashMap, HashSet};

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

use crate::{
    MarketResolution, Outcome, Result, StrategyConfig, StrategyEngine, Trade, crypto_taker_fee,
    parse_market_window,
};

const DAY_SECONDS: i64 = 86_400;
const LOOKBACK_SECONDS: i64 = 8 * DAY_SECONDS;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum StrategyFamily {
    FirstLargeBuy,
    ConfirmedFlow,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignalTrace {
    pub condition_id: String,
    pub outcome: Outcome,
    pub timestamp: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ReplayMetrics {
    pub resolved_signals: usize,
    pub wins: usize,
    pub losses: usize,
    pub recent_twenty_four_hours: usize,
    pub recent_six_hours: usize,
    pub net_pnl_one_cent: Decimal,
    pub net_pnl_two_cent: Decimal,
    pub turnover_one_cent: Decimal,
    pub roi_one_cent: Decimal,
    pub max_loss_streak: usize,
    pub max_drawdown: Decimal,
    pub median_price: Decimal,
    pub median_lead_seconds: i64,
    pub two_cent_profitable_fraction: Decimal,
    pub latest_signal_epoch: i64,
    pub signal_trace: Vec<SignalTrace>,
}

impl Default for ReplayMetrics {
    fn default() -> Self {
        Self {
            resolved_signals: 0,
            wins: 0,
            losses: 0,
            recent_twenty_four_hours: 0,
            recent_six_hours: 0,
            net_pnl_one_cent: Decimal::ZERO,
            net_pnl_two_cent: Decimal::ZERO,
            turnover_one_cent: Decimal::ZERO,
            roi_one_cent: Decimal::ZERO,
            max_loss_streak: 0,
            max_drawdown: Decimal::ZERO,
            median_price: Decimal::ZERO,
            median_lead_seconds: 0,
            two_cent_profitable_fraction: Decimal::ZERO,
            latest_signal_epoch: 0,
            signal_trace: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CandidateEvaluation {
    pub wallet: String,
    pub family: StrategyFamily,
    pub estimated_win_probability: Decimal,
    pub score: Decimal,
    pub eligible: bool,
    pub rejection_reasons: Vec<String>,
    pub metrics: ReplayMetrics,
}

#[derive(Clone, Debug)]
pub struct ReplayEvaluator {
    one_cent_slippage: Decimal,
    two_cent_slippage: Decimal,
}

impl Default for ReplayEvaluator {
    fn default() -> Self {
        Self {
            one_cent_slippage: dec!(0.01),
            two_cent_slippage: dec!(0.02),
        }
    }
}

impl ReplayEvaluator {
    pub fn evaluate(
        &self,
        wallet: &str,
        trades: &[Trade],
        resolutions: &HashMap<String, MarketResolution>,
        now: i64,
    ) -> Result<CandidateEvaluation> {
        let hedged = is_pervasively_hedged(wallet, trades, now);
        let first = self.evaluate_family(
            wallet,
            trades,
            resolutions,
            now,
            StrategyFamily::FirstLargeBuy,
        )?;
        let flow = self.evaluate_family(
            wallet,
            trades,
            resolutions,
            now,
            StrategyFamily::ConfirmedFlow,
        )?;
        let mut selected = if flow.eligible && (!first.eligible || flow.score > first.score) {
            flow
        } else if first.eligible || first.score >= flow.score {
            first
        } else {
            flow
        };
        if hedged {
            selected.eligible = false;
            add_reason(&mut selected.rejection_reasons, "two_sided_hedging");
        }
        Ok(selected)
    }

    fn evaluate_family(
        &self,
        wallet: &str,
        trades: &[Trade],
        resolutions: &HashMap<String, MarketResolution>,
        now: i64,
        family: StrategyFamily,
    ) -> Result<CandidateEvaluation> {
        let probability = dec!(0.60);
        let config = match family {
            StrategyFamily::FirstLargeBuy => StrategyConfig::FirstLargeBuy {
                wallet: wallet.to_ascii_lowercase(),
                minimum_notional: dec!(25),
                minimum_lead_seconds: 90,
                estimated_win_probability: probability,
            },
            StrategyFamily::ConfirmedFlow => StrategyConfig::ConfirmedFlow {
                wallet: wallet.to_ascii_lowercase(),
                minimum_cumulative_notional: dec!(100),
                minimum_directional_share: dec!(0.80),
                minimum_price: dec!(0.40),
                maximum_price: dec!(0.55),
                minimum_lead_seconds: 90,
                estimated_win_probability: probability,
            },
        };
        let mut engine = StrategyEngine::new(config);
        let mut ordered: Vec<&Trade> = trades
            .iter()
            .filter(|trade| {
                trade.proxy_wallet.eq_ignore_ascii_case(wallet)
                    && trade.timestamp >= now - LOOKBACK_SECONDS
                    && parse_market_window(&trade.slug).is_ok()
            })
            .collect();
        ordered.sort_by(|left, right| {
            (left.timestamp, &left.transaction_hash, &left.asset).cmp(&(
                right.timestamp,
                &right.transaction_hash,
                &right.asset,
            ))
        });

        let mut signals = Vec::new();
        for trade in ordered {
            if let Ok(Some(signal)) = engine.ingest(trade) {
                signals.push(signal);
            }
        }

        let mut metrics = ReplayMetrics::default();
        let mut prices = Vec::new();
        let mut leads = Vec::new();
        let mut cumulative = Decimal::ZERO;
        let mut peak = Decimal::ZERO;
        let mut loss_streak = 0usize;
        let mut two_cent_positive = 0usize;
        let mut high_price = 0usize;

        for signal in signals {
            let Some(resolution) = resolutions.get(&signal.condition_id) else {
                continue;
            };
            let Some(winner) = resolution.winner else {
                continue;
            };
            if !resolution.closed {
                continue;
            }
            let price_one = (signal.source_price + self.one_cent_slippage).min(dec!(0.99));
            let price_two = (signal.source_price + self.two_cent_slippage).min(dec!(0.99));
            let won = signal.outcome == winner;
            let fee_one = crypto_taker_fee(Decimal::ONE, price_one)?;
            let fee_two = crypto_taker_fee(Decimal::ONE, price_two)?;
            let pnl_one = if won {
                Decimal::ONE - price_one - fee_one
            } else {
                -price_one - fee_one
            };
            let pnl_two = if won {
                Decimal::ONE - price_two - fee_two
            } else {
                -price_two - fee_two
            };

            metrics.resolved_signals += 1;
            metrics.turnover_one_cent += price_one + fee_one;
            metrics.net_pnl_one_cent += pnl_one;
            metrics.net_pnl_two_cent += pnl_two;
            metrics.latest_signal_epoch = metrics.latest_signal_epoch.max(signal.source_timestamp);
            if signal.source_timestamp >= now - DAY_SECONDS {
                metrics.recent_twenty_four_hours += 1;
            }
            if signal.source_timestamp >= now - 6 * 3_600 {
                metrics.recent_six_hours += 1;
            }
            if won {
                metrics.wins += 1;
                loss_streak = 0;
            } else {
                metrics.losses += 1;
                loss_streak += 1;
                metrics.max_loss_streak = metrics.max_loss_streak.max(loss_streak);
            }
            if pnl_two > Decimal::ZERO {
                two_cent_positive += 1;
            }
            if signal.source_price >= dec!(0.95) {
                high_price += 1;
            }
            cumulative += pnl_one;
            peak = peak.max(cumulative);
            metrics.max_drawdown = metrics.max_drawdown.max(peak - cumulative);
            prices.push(signal.source_price);
            leads.push(signal.market_end_epoch - signal.source_timestamp);
            metrics.signal_trace.push(SignalTrace {
                condition_id: signal.condition_id,
                outcome: signal.outcome,
                timestamp: signal.source_timestamp,
            });
        }

        if metrics.turnover_one_cent > Decimal::ZERO {
            metrics.roi_one_cent = metrics.net_pnl_one_cent / metrics.turnover_one_cent;
        }
        if metrics.resolved_signals > 0 {
            metrics.two_cent_profitable_fraction = decimal_from_usize(two_cent_positive)
                / decimal_from_usize(metrics.resolved_signals);
        }
        metrics.median_price = median_decimal(&mut prices);
        metrics.median_lead_seconds = median_i64(&mut leads);

        let estimated_win_probability = if metrics.resolved_signals == 0 {
            dec!(0.50)
        } else {
            (decimal_from_usize(metrics.wins) + dec!(2))
                / (decimal_from_usize(metrics.resolved_signals) + dec!(4))
        };
        let inactivity_hours = if metrics.latest_signal_epoch == 0 {
            dec!(168)
        } else {
            Decimal::from((now - metrics.latest_signal_epoch).max(0)) / dec!(3600)
        };
        let score = metrics.roi_one_cent * dec!(1000)
            + decimal_from_usize(metrics.resolved_signals) * dec!(10)
            + decimal_from_usize(metrics.recent_twenty_four_hours) * dec!(20)
            + metrics.two_cent_profitable_fraction * dec!(100)
            - decimal_from_usize(metrics.max_loss_streak) * dec!(30)
            - metrics.max_drawdown * dec!(100)
            - inactivity_hours * dec!(5);

        let mut rejection_reasons = Vec::new();
        if metrics.resolved_signals < 5 {
            rejection_reasons.push("insufficient_resolved_sample".into());
        }
        if metrics.recent_twenty_four_hours < 2 {
            rejection_reasons.push("insufficient_recent_activity".into());
        }
        if metrics.recent_six_hours < 1 {
            rejection_reasons.push("inactive_six_hours".into());
        }
        if metrics.net_pnl_one_cent <= Decimal::ZERO {
            rejection_reasons.push("non_positive_one_cent_edge".into());
        }
        if metrics.resolved_signals > 0
            && decimal_from_usize(high_price) / decimal_from_usize(metrics.resolved_signals)
                > dec!(0.80)
        {
            rejection_reasons.push("unusable_high_price_profile".into());
        }
        let eligible = rejection_reasons.is_empty();

        Ok(CandidateEvaluation {
            wallet: wallet.to_ascii_lowercase(),
            family,
            estimated_win_probability,
            score,
            eligible,
            rejection_reasons,
            metrics,
        })
    }
}

fn is_pervasively_hedged(wallet: &str, trades: &[Trade], now: i64) -> bool {
    let mut totals: HashMap<String, HashMap<Outcome, Decimal>> = HashMap::new();
    for trade in trades.iter().filter(|trade| {
        trade.proxy_wallet.eq_ignore_ascii_case(wallet)
            && trade.side.eq_ignore_ascii_case("BUY")
            && trade.timestamp >= now - LOOKBACK_SECONDS
            && parse_market_window(&trade.slug).is_ok()
    }) {
        let Ok(outcome) = Outcome::parse(&trade.outcome) else {
            continue;
        };
        *totals
            .entry(trade.condition_id.clone())
            .or_default()
            .entry(outcome)
            .or_default() += trade.size * trade.price;
    }
    let total: Decimal = totals
        .values()
        .flat_map(|outcomes| outcomes.values())
        .copied()
        .sum();
    if total <= Decimal::ZERO {
        return false;
    }
    let two_sided: Decimal = totals
        .values()
        .filter(|outcomes| outcomes.len() > 1)
        .flat_map(|outcomes| outcomes.values())
        .copied()
        .sum();
    two_sided / total >= dec!(0.50)
}

fn median_decimal(values: &mut [Decimal]) -> Decimal {
    if values.is_empty() {
        return Decimal::ZERO;
    }
    values.sort();
    values[values.len() / 2]
}

fn median_i64(values: &mut [i64]) -> i64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    values[values.len() / 2]
}

fn decimal_from_usize(value: usize) -> Decimal {
    Decimal::from(u64::try_from(value).unwrap_or(u64::MAX))
}

fn add_reason(reasons: &mut Vec<String>, reason: &str) {
    let existing: HashSet<&str> = reasons.iter().map(String::as_str).collect();
    if !existing.contains(reason) {
        reasons.push(reason.into());
    }
}
