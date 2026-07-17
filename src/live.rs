use std::str::FromStr as _;

use async_trait::async_trait;
use polymarket_client_sdk_v2::POLYGON;
use polymarket_client_sdk_v2::auth::state::Authenticated;
use polymarket_client_sdk_v2::auth::{LocalSigner, Normal, Signer};
use polymarket_client_sdk_v2::clob::types::request::{BalanceAllowanceRequest, OrdersRequest};
use polymarket_client_sdk_v2::clob::types::{OrderType, Side};
use polymarket_client_sdk_v2::clob::{Client, Config};
use polymarket_client_sdk_v2::types::U256;
use rust_decimal::Decimal;

use crate::{
    CopybotError, ExecutionFill, ExecutionRequest, Executor, PostedBuySummary, PreflightFacts,
    PreflightStatus, Result, validated_live_buy_fill,
};

pub const PRODUCTION_CLOB_HOST: &str = "https://clob-v2.polymarket.com";

pub struct LiveExecutor<S: Signer> {
    signer: S,
    client: Client<Authenticated<Normal>>,
}

impl<S: Signer> LiveExecutor<S> {
    pub fn new(signer: S, client: Client<Authenticated<Normal>>) -> Self {
        Self { signer, client }
    }
}

#[async_trait]
pub trait LiveTradingExecutor: Executor {
    async fn preflight(&self, required_balance: Decimal) -> PreflightStatus;
}

pub async fn connect_eoa(private_key: &str) -> anyhow::Result<Box<dyn LiveTradingExecutor>> {
    connect_eoa_at(private_key, PRODUCTION_CLOB_HOST).await
}

pub async fn connect_eoa_at(
    private_key: &str,
    clob_host: &str,
) -> anyhow::Result<Box<dyn LiveTradingExecutor>> {
    let signer = LocalSigner::from_str(private_key)?.with_chain_id(Some(POLYGON));
    let client = Client::new(clob_host, Config::default())?
        .authentication_builder(&signer)
        .authenticate()
        .await?;
    Ok(Box::new(LiveExecutor::new(signer, client)))
}

#[async_trait]
impl<S> LiveTradingExecutor for LiveExecutor<S>
where
    S: Signer + Send + Sync,
{
    async fn preflight(&self, required_balance: Decimal) -> PreflightStatus {
        if required_balance <= Decimal::ZERO {
            return PreflightStatus::failed("required_balance_is_not_positive");
        }
        if let Err(error) = self.client.ok().await {
            return PreflightStatus::failed(format!("clob_health_failed: {error}"));
        }
        let geoblock = match self.client.check_geoblock().await {
            Ok(response) => response,
            Err(error) => {
                return PreflightStatus::failed(format!("geoblock_check_failed: {error}"));
            }
        };
        let closed_only = match self.client.closed_only_mode().await {
            Ok(response) => response.closed_only,
            Err(error) => {
                return PreflightStatus::failed(format!("closed_only_check_failed: {error}"));
            }
        };
        let balance_allowance = match self
            .client
            .balance_allowance(BalanceAllowanceRequest::default())
            .await
        {
            Ok(response) => response,
            Err(error) => {
                return PreflightStatus::failed(format!("balance_allowance_check_failed: {error}"));
            }
        };
        let orders = match self.client.orders(&OrdersRequest::default(), None).await {
            Ok(response) => response,
            Err(error) => {
                return PreflightStatus::failed(format!("open_orders_check_failed: {error}"));
            }
        };
        let has_positive_allowance = balance_allowance
            .allowances
            .values()
            .any(|value| string_amount_is_positive(value));
        PreflightStatus::from_facts(
            &PreflightFacts {
                geoblocked: geoblock.blocked,
                closed_only,
                balance: balance_allowance.balance,
                has_positive_allowance,
                signer_authenticated: true,
                open_orders: orders.data.len(),
            },
            required_balance,
        )
    }
}

#[async_trait]
impl<S> Executor for LiveExecutor<S>
where
    S: Signer + Send + Sync,
{
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionFill> {
        if request.shares <= Decimal::ZERO || !request.shares.fract().is_zero() {
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

fn string_amount_is_positive(value: &str) -> bool {
    value
        .parse::<Decimal>()
        .is_ok_and(|amount| amount > Decimal::ZERO)
        || value
            .chars()
            .any(|character| character.is_ascii_digit() && character != '0')
}
