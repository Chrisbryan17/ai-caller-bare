use std::collections::HashMap;

use rust_decimal::Decimal;

use crate::{CandidateSignal, CopybotError, Outcome, Result, Trade, parse_market_window};

#[derive(Clone, Debug, PartialEq)]
pub enum StrategyConfig {
    FirstLargeBuy {
        wallet: String,
        minimum_notional: Decimal,
        minimum_lead_seconds: i64,
        estimated_win_probability: Decimal,
    },
    ConfirmedFlow {
        wallet: String,
        minimum_cumulative_notional: Decimal,
        minimum_directional_share: Decimal,
        minimum_price: Decimal,
        maximum_price: Decimal,
        minimum_lead_seconds: i64,
        estimated_win_probability: Decimal,
    },
}

impl StrategyConfig {
    pub fn wallet(&self) -> &str {
        match self {
            Self::FirstLargeBuy { wallet, .. } | Self::ConfirmedFlow { wallet, .. } => wallet,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct State {
    emitted: bool,
    cumulative: HashMap<Outcome, Decimal>,
    assets: HashMap<Outcome, String>,
    prices: HashMap<Outcome, Decimal>,
}

#[derive(Clone, Debug)]
pub struct StrategyEngine {
    config: StrategyConfig,
    markets: HashMap<String, State>,
}

impl StrategyEngine {
    pub fn new(config: StrategyConfig) -> Self {
        Self {
            config,
            markets: HashMap::new(),
        }
    }

    pub fn ingest(&mut self, t: &Trade) -> Result<Option<CandidateSignal>> {
        if !t.proxy_wallet.eq_ignore_ascii_case(self.config.wallet()) {
            return Err(CopybotError::WalletMismatch);
        }
        if !t.side.eq_ignore_ascii_case("BUY")
            || t.size <= Decimal::ZERO
            || t.price <= Decimal::ZERO
            || t.price >= Decimal::ONE
        {
            return Ok(None);
        }
        let window = parse_market_window(&t.slug)?;
        let lead = window.end_epoch - t.timestamp;
        let outcome = Outcome::parse(&t.outcome)?;
        let state = self.markets.entry(t.condition_id.clone()).or_default();
        if state.emitted {
            return Ok(None);
        }

        let signal = match &self.config {
            StrategyConfig::FirstLargeBuy {
                minimum_notional,
                minimum_lead_seconds,
                estimated_win_probability,
                ..
            } => (t.size * t.price >= *minimum_notional && lead >= *minimum_lead_seconds).then(
                || {
                    signal(
                        t,
                        outcome,
                        window.end_epoch,
                        "first_large_buy",
                        *estimated_win_probability,
                    )
                },
            ),
            StrategyConfig::ConfirmedFlow {
                minimum_cumulative_notional,
                minimum_directional_share,
                minimum_price,
                maximum_price,
                minimum_lead_seconds,
                estimated_win_probability,
                ..
            } => {
                *state.cumulative.entry(outcome).or_default() += t.size * t.price;
                state.assets.insert(outcome, t.asset.clone());
                state.prices.insert(outcome, t.price);
                let total: Decimal = state.cumulative.values().copied().sum();
                let leader = state
                    .cumulative
                    .iter()
                    .max_by(|a, b| a.1.cmp(b.1))
                    .map(|(candidate, _)| *candidate);
                leader.and_then(|leader| {
                    let notional = state.cumulative[&leader];
                    let share = if total.is_zero() {
                        Decimal::ZERO
                    } else {
                        notional / total
                    };
                    let price = state.prices[&leader];
                    (notional >= *minimum_cumulative_notional
                        && share >= *minimum_directional_share
                        && price >= *minimum_price
                        && price <= *maximum_price
                        && lead >= *minimum_lead_seconds)
                        .then(|| CandidateSignal {
                            wallet: t.proxy_wallet.clone(),
                            condition_id: t.condition_id.clone(),
                            asset_id: state.assets[&leader].clone(),
                            outcome: leader,
                            source_price: price,
                            source_timestamp: t.timestamp,
                            market_end_epoch: window.end_epoch,
                            slug: t.slug.clone(),
                            title: t.title.clone(),
                            strategy: "confirmed_flow".into(),
                            estimated_win_probability: *estimated_win_probability,
                        })
                })
            }
        };
        if signal.is_some() {
            state.emitted = true;
        }
        Ok(signal)
    }
}

fn signal(
    t: &Trade,
    outcome: Outcome,
    end: i64,
    strategy: &str,
    probability: Decimal,
) -> CandidateSignal {
    CandidateSignal {
        wallet: t.proxy_wallet.clone(),
        condition_id: t.condition_id.clone(),
        asset_id: t.asset.clone(),
        outcome,
        source_price: t.price,
        source_timestamp: t.timestamp,
        market_end_epoch: end,
        slug: t.slug.clone(),
        title: t.title.clone(),
        strategy: strategy.into(),
        estimated_win_probability: probability,
    }
}
