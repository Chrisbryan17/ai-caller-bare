use std::time::Duration;

use crate::{CopybotError, Result, Trade};

#[derive(Clone, Debug)]
pub struct DataApiClient {
    client: reqwest::Client,
    base_url: String,
}

impl DataApiClient {
    pub fn new(base_url: impl Into<String>, timeout: Duration) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(timeout)
                .pool_idle_timeout(Duration::from_secs(30))
                .tcp_nodelay(true)
                .build()?,
            base_url: base_url.into().trim_end_matches('/').into(),
        })
    }

    pub async fn fetch_trades(&self, wallet: &str, limit: usize) -> Result<Vec<Trade>> {
        if wallet.trim().is_empty() || !(1..=1000).contains(&limit) {
            return Err(CopybotError::InvalidConfiguration(
                "wallet required and limit must be 1..=1000".into(),
            ));
        }
        let limit_string = limit.to_string();
        let response = self
            .client
            .get(format!("{}/trades", self.base_url))
            .query(&[
                ("user", wallet),
                ("limit", limit_string.as_str()),
                ("offset", "0"),
                ("takerOnly", "false"),
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
        let mut rows: Vec<Trade> = response.json().await?;
        rows.sort_by(|a, b| {
            (a.timestamp, &a.transaction_hash, &a.asset).cmp(&(
                b.timestamp,
                &b.transaction_hash,
                &b.asset,
            ))
        });
        Ok(rows)
    }
}
