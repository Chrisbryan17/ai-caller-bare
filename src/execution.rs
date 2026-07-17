use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

use crate::{CandidateSignal, CopybotError, Outcome, Result, crypto_taker_fee};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExecutionRequest {
    pub signal: CandidateSignal,
    pub shares: Decimal,
    pub maximum_price: Decimal,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ExecutionFill {
    pub condition_id: String,
    pub asset_id: String,
    pub outcome: Outcome,
    pub shares: Decimal,
    pub fill_price: Decimal,
    pub fee: Decimal,
    pub total_cost: Decimal,
    pub external_id: Option<String>,
    pub paper: bool,
}

/// SDK-independent projection of a posted FOK limit BUY response. Keeping this normalized type in
/// the core lets response validation receive full unit-test coverage without constructing a
/// non-exhaustive SDK response type.
#[derive(Clone, Debug, PartialEq)]
pub struct PostedBuySummary {
    pub success: bool,
    pub error_msg: Option<String>,
    /// Collateral paid by the BUY maker side.
    pub making_amount: Decimal,
    /// Outcome shares received by the BUY taker side.
    pub taking_amount: Decimal,
    pub order_id: String,
}

/// Converts a server-confirmed limit BUY into an audited fill and rejects ambiguous responses.
pub fn validated_live_buy_fill(
    signal: CandidateSignal,
    maximum_price: Decimal,
    response: PostedBuySummary,
) -> Result<ExecutionFill> {
    if maximum_price <= Decimal::ZERO || maximum_price >= Decimal::ONE {
        return Err(CopybotError::InvalidPrice(maximum_price));
    }
    if !response.success {
        return Err(CopybotError::LiveExecution(
            "order response reported success=false".into(),
        ));
    }
    if let Some(message) = response.error_msg.as_deref().map(str::trim)
        && !message.is_empty()
    {
        return Err(CopybotError::LiveExecution(format!(
            "order response contained error: {message}"
        )));
    }
    if response.making_amount <= Decimal::ZERO || response.taking_amount <= Decimal::ZERO {
        return Err(CopybotError::LiveExecution(
            "FOK order returned no positive matched amounts".into(),
        ));
    }
    let fill_price = response.making_amount / response.taking_amount;
    if fill_price <= Decimal::ZERO || fill_price >= Decimal::ONE {
        return Err(CopybotError::InvalidPrice(fill_price));
    }
    if fill_price > maximum_price {
        return Err(CopybotError::LiveExecution(format!(
            "server fill price {fill_price} exceeded hard limit {maximum_price}"
        )));
    }
    let fee = crypto_taker_fee(response.taking_amount, fill_price)?;
    Ok(ExecutionFill {
        condition_id: signal.condition_id,
        asset_id: signal.asset_id,
        outcome: signal.outcome,
        shares: response.taking_amount,
        fill_price,
        fee,
        total_cost: response.making_amount + fee,
        external_id: (!response.order_id.trim().is_empty()).then_some(response.order_id),
        paper: false,
    })
}

#[async_trait]
pub trait Executor: Send + Sync {
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionFill>;
}

#[derive(Clone, Debug)]
pub struct PaperExecutor {
    slippage: Decimal,
}

impl PaperExecutor {
    pub fn new(slippage: Decimal) -> Result<Self> {
        if slippage < Decimal::ZERO || slippage > dec!(0.25) {
            return Err(CopybotError::InvalidConfiguration(
                "paper slippage must be in [0,0.25]".into(),
            ));
        }
        Ok(Self { slippage })
    }
}

#[async_trait]
impl Executor for PaperExecutor {
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionFill> {
        if request.shares <= Decimal::ZERO || !request.shares.fract().is_zero() {
            return Err(CopybotError::InvalidConfiguration(
                "shares must be a positive integer".into(),
            ));
        }
        if request.maximum_price <= Decimal::ZERO || request.maximum_price >= Decimal::ONE {
            return Err(CopybotError::InvalidPrice(request.maximum_price));
        }
        let fill_price = (request.signal.source_price + self.slippage).min(request.maximum_price);
        if fill_price < request.signal.source_price {
            return Err(CopybotError::InvalidConfiguration(
                "price cap below source price".into(),
            ));
        }
        let fee = crypto_taker_fee(request.shares, fill_price)?;
        Ok(ExecutionFill {
            condition_id: request.signal.condition_id,
            asset_id: request.signal.asset_id,
            outcome: request.signal.outcome,
            shares: request.shares,
            fill_price,
            fee,
            total_cost: request.shares * fill_price + fee,
            external_id: None,
            paper: true,
        })
    }
}
