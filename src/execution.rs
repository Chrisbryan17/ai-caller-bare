use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};

use crate::{
    CandidateSignal, CopybotError, Outcome, Result, crypto_taker_fee_per_share,
};

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
        let fill_price =
            (request.signal.source_price + self.slippage).min(request.maximum_price);
        if fill_price < request.signal.source_price {
            return Err(CopybotError::InvalidConfiguration(
                "price cap below source price".into(),
            ));
        }
        let fee = request.shares * crypto_taker_fee_per_share(fill_price)?;
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
