use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    CopybotError, ExecutionFill, ExecutionRequest, Executor, Result, crypto_taker_fee,
};

fn decimal<'de, D>(deserializer: D) -> std::result::Result<Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        String(String),
        Number(serde_json::Number),
    }

    match Wire::deserialize(deserializer)? {
        Wire::String(value) => value.parse().map_err(serde::de::Error::custom),
        Wire::Number(value) => value.to_string().parse().map_err(serde::de::Error::custom),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OrderBookLevel {
    #[serde(deserialize_with = "decimal")]
    pub price: Decimal,
    #[serde(deserialize_with = "decimal")]
    pub size: Decimal,
}

#[derive(Clone, Debug, Deserialize)]
struct OrderBookWire {
    asset_id: String,
    #[serde(default)]
    asks: Vec<OrderBookLevel>,
    #[serde(deserialize_with = "decimal")]
    min_order_size: Decimal,
    #[serde(deserialize_with = "decimal")]
    tick_size: Decimal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExecutableBuyQuote {
    pub shares: Decimal,
    pub aligned_maximum_price: Decimal,
    pub average_price: Decimal,
    pub collateral: Decimal,
    pub fee: Decimal,
    pub total_cost: Decimal,
}

#[derive(Clone, Debug)]
pub struct OrderBookClient {
    client: reqwest::Client,
    base_url: String,
}

impl OrderBookClient {
    pub fn new(base_url: impl Into<String>, timeout: Duration) -> Result<Self> {
        let base_url = base_url.into().trim_end_matches('/').to_owned();
        if base_url.is_empty() {
            return Err(CopybotError::InvalidConfiguration(
                "order-book base URL cannot be empty".into(),
            ));
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(timeout)
                .pool_idle_timeout(Duration::from_secs(30))
                .tcp_nodelay(true)
                .build()?,
            base_url,
        })
    }

    async fn fetch_book(&self, token_id: &str) -> Result<OrderBookWire> {
        if token_id.trim().is_empty() {
            return Err(CopybotError::InvalidConfiguration(
                "order-book token id cannot be empty".into(),
            ));
        }
        let response = self
            .client
            .get(format!("{}/book", self.base_url))
            .query(&[("token_id", token_id)])
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(CopybotError::HttpStatus {
                status,
                body: response.text().await.unwrap_or_default(),
            });
        }
        Ok(response.json().await?)
    }

    pub async fn quote_fok_buy(
        &self,
        request: &ExecutionRequest,
        paper_slippage: Decimal,
    ) -> Result<ExecutableBuyQuote> {
        validate_request(request, paper_slippage)?;
        let mut book = self.fetch_book(&request.signal.asset_id).await?;
        if book.asset_id != request.signal.asset_id {
            return Err(CopybotError::InvalidConfiguration(
                "order-book asset id did not match requested token".into(),
            ));
        }
        if book.min_order_size <= Decimal::ZERO || book.tick_size <= Decimal::ZERO {
            return Err(CopybotError::InvalidConfiguration(
                "order book returned invalid minimum size or tick size".into(),
            ));
        }
        if request.shares < book.min_order_size {
            return Err(CopybotError::InvalidConfiguration(format!(
                "requested shares are below market minimum {}",
                book.min_order_size
            )));
        }

        let raw_cap = request
            .maximum_price
            .min(request.signal.source_price + paper_slippage);
        let aligned_cap = (raw_cap / book.tick_size).floor() * book.tick_size;
        if aligned_cap < request.signal.source_price
            || aligned_cap <= Decimal::ZERO
            || aligned_cap >= Decimal::ONE
        {
            return Err(CopybotError::InvalidConfiguration(
                "price cap below source price after tick alignment".into(),
            ));
        }

        book.asks.sort_by(|left, right| {
            left.price
                .cmp(&right.price)
                .then_with(|| left.size.cmp(&right.size))
        });
        let mut remaining = request.shares;
        let mut collateral = Decimal::ZERO;
        let mut fee = Decimal::ZERO;
        for level in book.asks {
            if level.price <= Decimal::ZERO
                || level.price >= Decimal::ONE
                || level.size <= Decimal::ZERO
            {
                return Err(CopybotError::InvalidConfiguration(
                    "order book contained an invalid ask level".into(),
                ));
            }
            if level.price > aligned_cap {
                break;
            }
            let filled = remaining.min(level.size);
            if filled <= Decimal::ZERO {
                continue;
            }
            collateral += filled * level.price;
            fee += crypto_taker_fee(filled, level.price)?;
            remaining -= filled;
            if remaining.is_zero() {
                break;
            }
        }
        if !remaining.is_zero() {
            return Err(CopybotError::InvalidConfiguration(
                "insufficient executable liquidity for full FOK paper fill".into(),
            ));
        }
        let average_price = collateral / request.shares;
        Ok(ExecutableBuyQuote {
            shares: request.shares,
            aligned_maximum_price: aligned_cap,
            average_price,
            collateral,
            fee,
            total_cost: collateral + fee,
        })
    }
}

fn validate_request(request: &ExecutionRequest, paper_slippage: Decimal) -> Result<()> {
    if request.shares <= Decimal::ZERO || !request.shares.fract().is_zero() {
        return Err(CopybotError::InvalidConfiguration(
            "shares must be a positive integer".into(),
        ));
    }
    if request.maximum_price <= Decimal::ZERO || request.maximum_price >= Decimal::ONE {
        return Err(CopybotError::InvalidPrice(request.maximum_price));
    }
    if request.signal.source_price <= Decimal::ZERO || request.signal.source_price >= Decimal::ONE {
        return Err(CopybotError::InvalidPrice(request.signal.source_price));
    }
    if !(Decimal::ZERO..=dec!(0.25)).contains(&paper_slippage) {
        return Err(CopybotError::InvalidConfiguration(
            "paper slippage must be in [0,0.25]".into(),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct DepthAwarePaperExecutor {
    order_book: OrderBookClient,
    paper_slippage: Decimal,
}

impl DepthAwarePaperExecutor {
    pub fn new(
        base_url: impl Into<String>,
        timeout: Duration,
        paper_slippage: Decimal,
    ) -> Result<Self> {
        if !(Decimal::ZERO..=dec!(0.25)).contains(&paper_slippage) {
            return Err(CopybotError::InvalidConfiguration(
                "paper slippage must be in [0,0.25]".into(),
            ));
        }
        Ok(Self {
            order_book: OrderBookClient::new(base_url, timeout)?,
            paper_slippage,
        })
    }
}

#[async_trait]
impl Executor for DepthAwarePaperExecutor {
    async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionFill> {
        let quote = self
            .order_book
            .quote_fok_buy(&request, self.paper_slippage)
            .await?;
        Ok(ExecutionFill {
            condition_id: request.signal.condition_id,
            asset_id: request.signal.asset_id,
            outcome: request.signal.outcome,
            shares: quote.shares,
            fill_price: quote.average_price,
            fee: quote.fee,
            total_cost: quote.total_cost,
            external_id: None,
            paper: true,
        })
    }
}
