use std::{collections::HashMap, time::Duration};

use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize};

use crate::{CopybotError, Outcome, Result};

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

fn rank<'de, D>(deserializer: D) -> std::result::Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        String(String),
        Number(u64),
    }

    match Wire::deserialize(deserializer)? {
        Wire::String(value) => value.parse().map_err(serde::de::Error::custom),
        Wire::Number(value) => Ok(value),
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "UPPERCASE")]
pub enum LeaderboardPeriod {
    Day,
    Week,
    Month,
}

impl LeaderboardPeriod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Day => "DAY",
            Self::Week => "WEEK",
            Self::Month => "MONTH",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LeaderboardRow {
    #[serde(deserialize_with = "rank")]
    pub rank: u64,
    pub proxy_wallet: String,
    #[serde(default)]
    pub user_name: String,
    #[serde(rename = "vol", deserialize_with = "decimal")]
    pub volume: Decimal,
    #[serde(deserialize_with = "decimal")]
    pub pnl: Decimal,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LeaderboardSnapshot {
    pub period: LeaderboardPeriod,
    pub rows: Vec<LeaderboardRow>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MarketResolution {
    pub condition_id: String,
    pub closed: bool,
    pub winner: Option<Outcome>,
}

#[derive(Clone, Debug)]
pub struct DiscoveryApiClient {
    client: reqwest::Client,
    data_api_base: String,
    gamma_api_base: String,
}

impl DiscoveryApiClient {
    pub fn new(
        data_api_base: impl Into<String>,
        gamma_api_base: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(timeout)
                .pool_idle_timeout(Duration::from_secs(30))
                .tcp_nodelay(true)
                .build()?,
            data_api_base: data_api_base.into().trim_end_matches('/').into(),
            gamma_api_base: gamma_api_base.into().trim_end_matches('/').into(),
        })
    }

    pub async fn fetch_leaderboard(
        &self,
        period: LeaderboardPeriod,
        limit: usize,
    ) -> Result<LeaderboardSnapshot> {
        if !(1..=50).contains(&limit) {
            return Err(CopybotError::InvalidConfiguration(
                "leaderboard limit must be 1..=50".into(),
            ));
        }
        let limit_string = limit.to_string();
        let response = self
            .client
            .get(format!("{}/v1/leaderboard", self.data_api_base))
            .query(&[
                ("category", "CRYPTO"),
                ("timePeriod", period.as_str()),
                ("orderBy", "PNL"),
                ("limit", limit_string.as_str()),
                ("offset", "0"),
            ])
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(CopybotError::HttpStatus {
                status,
                body: response.text().await.unwrap_or_default(),
            });
        }
        let mut rows: Vec<LeaderboardRow> = response.json().await?;
        for row in &mut rows {
            row.proxy_wallet.make_ascii_lowercase();
        }
        rows.sort_by(|left, right| {
            (left.rank, &left.proxy_wallet).cmp(&(right.rank, &right.proxy_wallet))
        });
        Ok(LeaderboardSnapshot { period, rows })
    }

    pub async fn fetch_resolutions(
        &self,
        condition_ids: &[String],
    ) -> Result<HashMap<String, MarketResolution>> {
        let mut resolutions = HashMap::new();
        for chunk in condition_ids.chunks(50) {
            if chunk.is_empty() {
                continue;
            }
            let mut params: Vec<(&str, String)> =
                vec![("closed", "true".into()), ("limit", "50".into())];
            params.extend(
                chunk
                    .iter()
                    .map(|condition| ("condition_ids", condition.clone())),
            );
            let response = self
                .client
                .get(format!("{}/markets", self.gamma_api_base))
                .query(&params)
                .send()
                .await?;
            let status = response.status();
            if !status.is_success() {
                return Err(CopybotError::HttpStatus {
                    status,
                    body: response.text().await.unwrap_or_default(),
                });
            }
            let markets: Vec<GammaMarket> = response.json().await?;
            for market in markets {
                let resolution = market.into_resolution();
                resolutions.insert(resolution.condition_id.clone(), resolution);
            }
        }
        Ok(resolutions)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GammaMarket {
    condition_id: String,
    #[serde(default)]
    closed: bool,
    #[serde(default)]
    outcomes: Option<String>,
    #[serde(default)]
    outcome_prices: Option<String>,
}

impl GammaMarket {
    fn into_resolution(self) -> MarketResolution {
        let winner = if self.closed {
            parse_winner(self.outcomes.as_deref(), self.outcome_prices.as_deref())
        } else {
            None
        };
        MarketResolution {
            condition_id: self.condition_id,
            closed: self.closed,
            winner,
        }
    }
}

fn parse_winner(outcomes: Option<&str>, prices: Option<&str>) -> Option<Outcome> {
    let outcomes: Vec<String> = serde_json::from_str(outcomes?).ok()?;
    let prices: Vec<String> = serde_json::from_str(prices?).ok()?;
    if outcomes.len() != prices.len() {
        return None;
    }
    outcomes
        .into_iter()
        .zip(prices)
        .filter_map(|(outcome, price)| {
            let price: Decimal = price.parse().ok()?;
            (price >= Decimal::new(999, 3))
                .then(|| Outcome::parse(&outcome).ok())
                .flatten()
        })
        .next()
}
