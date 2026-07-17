use rust_decimal::{Decimal, RoundingStrategy};
use rust_decimal_macros::dec;

use crate::{CopybotError, Result};

pub fn crypto_taker_fee_per_share(price: Decimal) -> Result<Decimal> {
    if !(Decimal::ZERO..=Decimal::ONE).contains(&price) {
        return Err(CopybotError::InvalidPrice(price));
    }
    Ok(dec!(0.07) * price * (Decimal::ONE - price))
}

/// Calculates the protocol's Crypto taker fee and rounds the transaction fee to five decimal
/// places, matching Polymarket's documented precision.
pub fn crypto_taker_fee(shares: Decimal, price: Decimal) -> Result<Decimal> {
    if shares < Decimal::ZERO {
        return Err(CopybotError::InvalidConfiguration(
            "fee shares cannot be negative".into(),
        ));
    }
    Ok((shares * crypto_taker_fee_per_share(price)?).round_dp(5))
}

/// Floors a positive binary-market price to a one-cent-aligned ceiling.
///
/// This deliberately rounds toward zero so the executable limit never exceeds the raw source
/// price plus configured slippage. One-cent alignment is accepted by markets with either a one-cent
/// or finer minimum tick.
pub fn conservative_cent_price(raw: Decimal) -> Result<Decimal> {
    if raw <= Decimal::ZERO || raw >= Decimal::ONE {
        return Err(CopybotError::InvalidPrice(raw));
    }
    let price = raw.round_dp_with_strategy(2, RoundingStrategy::ToZero);
    if price <= Decimal::ZERO || price >= Decimal::ONE {
        return Err(CopybotError::InvalidPrice(raw));
    }
    Ok(price)
}

#[derive(Clone, Debug, PartialEq)]
pub struct SizingConfig {
    pub bankroll: Decimal,
    pub estimated_win_probability: Decimal,
    pub kelly_multiplier: Decimal,
    pub max_bankroll_fraction: Decimal,
    pub minimum_shares: Decimal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SizeDecision {
    pub shares: Decimal,
    pub fraction: Decimal,
    pub capital_budget: Decimal,
    pub fee_per_share: Decimal,
    pub total_fee: Decimal,
    pub total_cost: Decimal,
}

#[derive(Clone, Debug)]
pub struct PositionSizer {
    config: SizingConfig,
}

impl PositionSizer {
    pub fn new(config: SizingConfig) -> Result<Self> {
        if config.bankroll <= Decimal::ZERO {
            return Err(CopybotError::InvalidConfiguration(
                "bankroll must be positive".into(),
            ));
        }
        if !(Decimal::ZERO..=Decimal::ONE).contains(&config.estimated_win_probability) {
            return Err(CopybotError::InvalidProbability(
                config.estimated_win_probability,
            ));
        }
        if config.kelly_multiplier <= Decimal::ZERO || config.kelly_multiplier > Decimal::ONE {
            return Err(CopybotError::InvalidConfiguration(
                "kelly multiplier must be in (0,1]".into(),
            ));
        }
        if config.max_bankroll_fraction <= Decimal::ZERO
            || config.max_bankroll_fraction > Decimal::ONE
        {
            return Err(CopybotError::InvalidConfiguration(
                "risk cap must be in (0,1]".into(),
            ));
        }
        if config.minimum_shares <= Decimal::ZERO || !config.minimum_shares.fract().is_zero() {
            return Err(CopybotError::InvalidConfiguration(
                "minimum shares must be a positive integer".into(),
            ));
        }
        Ok(Self { config })
    }

    pub fn size(&self, price: Decimal) -> Result<SizeDecision> {
        if price <= Decimal::ZERO || price >= Decimal::ONE {
            return Err(CopybotError::InvalidPrice(price));
        }
        let fee_per_share = crypto_taker_fee_per_share(price)?;
        let estimated_cost_per_share = price + fee_per_share;
        let edge = self.config.estimated_win_probability - estimated_cost_per_share;
        let loss_payoff = Decimal::ONE - estimated_cost_per_share;
        if edge <= Decimal::ZERO || loss_payoff <= Decimal::ZERO {
            return Err(CopybotError::NoPositiveEdge);
        }
        let fraction = ((edge / loss_payoff) * self.config.kelly_multiplier)
            .min(self.config.max_bankroll_fraction);
        let capital_budget = self.config.bankroll * fraction;
        let mut shares = (capital_budget / estimated_cost_per_share).floor();
        if shares < self.config.minimum_shares {
            return Err(CopybotError::InsufficientBankroll);
        }

        let (mut total_fee, mut total_cost) = totals(shares, price)?;
        while total_cost > capital_budget && shares >= self.config.minimum_shares {
            shares -= Decimal::ONE;
            if shares < self.config.minimum_shares {
                return Err(CopybotError::InsufficientBankroll);
            }
            (total_fee, total_cost) = totals(shares, price)?;
        }

        Ok(SizeDecision {
            shares,
            fraction,
            capital_budget,
            fee_per_share,
            total_fee,
            total_cost,
        })
    }
}

fn totals(shares: Decimal, price: Decimal) -> Result<(Decimal, Decimal)> {
    let fee = crypto_taker_fee(shares, price)?;
    Ok((fee, shares * price + fee))
}
