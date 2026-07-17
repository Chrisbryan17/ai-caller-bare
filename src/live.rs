use std::str::FromStr as _;

use async_trait::async_trait;
use polymarket_client_sdk_v2::POLYGON;
use polymarket_client_sdk_v2::auth::state::Authenticated;
use polymarket_client_sdk_v2::auth::{LocalSigner, Normal, Signer};
use polymarket_client_sdk_v2::clob::types::{OrderType, Side};
use polymarket_client_sdk_v2::clob::{Client, Config};
use polymarket_client_sdk_v2::types::U256;

use crate::{
    CopybotError, ExecutionFill, ExecutionRequest, Executor, PostedBuySummary, Result,
    validated_live_buy_fill,
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

pub async fn connect_eoa(private_key: &str) -> anyhow::Result<LiveExecutor<LocalSigner>> {
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
        let response = self
            .client
            .limit_order()
            .token_id(token)
            .size(request.shares)
            .price(request.maximum_price)
            .side(Side::Buy)
            .order_type(OrderType::FOK)
            .build_sign_and_post(&self.signer)
            .await
            .map_err(|error| CopybotError::LiveExecution(error.to_string()))?;

        validated_live_buy_fill(
            request.signal,
            request.maximum_price,
            PostedBuySummary {
                success: response.success,
                error_msg: response.error_msg,
                making_amount: response.making_amount,
                taking_amount: response.taking_amount,
                order_id: response.order_id,
            },
        )
    }
}
