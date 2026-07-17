use std::str::FromStr as _;

use async_trait::async_trait;
use polymarket_client_sdk_v2::POLYGON;
use polymarket_client_sdk_v2::auth::state::Authenticated;
use polymarket_client_sdk_v2::auth::{LocalSigner, Normal, Signer};
use polymarket_client_sdk_v2::clob::types::{Amount, OrderType, Side};
use polymarket_client_sdk_v2::clob::{Client, Config};
use polymarket_client_sdk_v2::types::U256;

use crate::{
    CopybotError, ExecutionFill, ExecutionRequest, Executor, Result,
    crypto_taker_fee_per_share,
};

pub struct LiveExecutor<S: Signer> {
    signer: S,
    client: Client<Authenticated<Normal>>,
}

impl<S: Signer> LiveExecutor<S> {
    pub fn new(signer: S, client: Client<Authenticated<Normal>>) -> Self {
        Self { signer, client }
    }
}

pub async fn connect_eoa(private_key: &str) -> anyhow::Result<impl Executor> {
    let signer = LocalSigner::from_str(private_key)?.with_chain_id(Some(POLYGON));
    let client = Client::new("https://clob-v2.polymarket.com", Config::default())?
        .authentication_builder(&signer)
        .authenticate()
        .await?;
    Ok(LiveExecutor::new(signer, client))
}

#[async_trait]
impl<S> Executor for LiveExecutor<S>
where
    S: Signer + Send + Sync,
{
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionFill> {
        if request.shares <= rust_decimal::Decimal::ZERO
            || !request.shares.fract().is_zero()
        {
            return Err(CopybotError::InvalidConfiguration(
                "live shares must be a positive integer".into(),
            ));
        }
        let token = U256::from_str(&request.signal.asset_id)
            .map_err(|error| CopybotError::LiveExecution(error.to_string()))?;
        let amount = Amount::shares(request.shares)
            .map_err(|error| CopybotError::LiveExecution(error.to_string()))?;
        let order = self
            .client
            .market_order()
            .token_id(token)
            .amount(amount)
            .price(request.maximum_price)
            .side(Side::Buy)
            .order_type(OrderType::FOK)
            .build()
            .await
            .map_err(|error| CopybotError::LiveExecution(error.to_string()))?;
        let signed = self
            .client
            .sign(&self.signer, order)
            .await
            .map_err(|error| CopybotError::LiveExecution(error.to_string()))?;
        let response = self
            .client
            .post_order(signed)
            .await
            .map_err(|error| CopybotError::LiveExecution(error.to_string()))?;
        let fee = request.shares * crypto_taker_fee_per_share(request.maximum_price)?;
        Ok(ExecutionFill {
            condition_id: request.signal.condition_id,
            asset_id: request.signal.asset_id,
            outcome: request.signal.outcome,
            shares: request.shares,
            fill_price: request.maximum_price,
            fee,
            total_cost: request.shares * request.maximum_price + fee,
            external_id: Some(format!("{response:?}")),
            paper: false,
        })
    }
}
