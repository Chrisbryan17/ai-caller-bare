use std::{fmt, sync::OnceLock};

use regex::Regex;
use reqwest::StatusCode;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CopybotError {
    #[error("unsupported or malformed market slug: {0}")]
    InvalidSlug(String),
    #[error("invalid price: {0}")]
    InvalidPrice(Decimal),
    #[error("invalid probability: {0}")]
    InvalidProbability(Decimal),
    #[error("invalid configuration: {0}")]
    InvalidConfiguration(String),
    #[error("unsupported outcome: {0}")]
    InvalidOutcome(String),
    #[error("strategy wallet does not match trade wallet")]
    WalletMismatch,
    #[error("position has no positive edge at the configured probability")]
    NoPositiveEdge,
    #[error("bankroll is too small for the configured minimum order")]
    InsufficientBankroll,
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Polymarket returned HTTP {status}: {body}")]
    HttpStatus { status: StatusCode, body: String },
    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("live trading is not acknowledged")]
    LiveTradingNotAcknowledged,
    #[error("live trading support was not compiled")]
    LiveTradingNotCompiled,
    #[error("live execution failed: {0}")]
    LiveExecution(String),
}

pub type Result<T> = std::result::Result<T, CopybotError>;

fn decimal<'de, D>(d: D) -> std::result::Result<Decimal, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wire {
        String(String),
        Number(serde_json::Number),
    }
    match Wire::deserialize(d)? {
        Wire::String(v) => v.parse().map_err(serde::de::Error::custom),
        Wire::Number(v) => v.to_string().parse().map_err(serde::de::Error::custom),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Trade {
    pub proxy_wallet: String,
    pub side: String,
    pub asset: String,
    pub condition_id: String,
    #[serde(deserialize_with = "decimal")]
    pub size: Decimal,
    #[serde(deserialize_with = "decimal")]
    pub price: Decimal,
    pub timestamp: i64,
    #[serde(default)]
    pub title: String,
    pub slug: String,
    pub outcome: String,
    #[serde(default)]
    pub transaction_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarketWindow {
    pub symbol: String,
    pub duration_seconds: i64,
    pub start_epoch: i64,
    pub end_epoch: i64,
}

pub fn parse_market_window(slug: &str) -> Result<MarketWindow> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re =
        RE.get_or_init(|| Regex::new(r"^(btc|eth|sol|xrp)-updown-(5m|15m)-(\d{10})$").unwrap());
    let c = re
        .captures(slug)
        .ok_or_else(|| CopybotError::InvalidSlug(slug.into()))?;
    let duration_seconds = match &c[2] {
        "5m" => 300,
        "15m" => 900,
        _ => unreachable!(),
    };
    let start_epoch = c[3]
        .parse()
        .map_err(|_| CopybotError::InvalidSlug(slug.into()))?;
    Ok(MarketWindow {
        symbol: c[1].into(),
        duration_seconds,
        start_epoch,
        end_epoch: start_epoch + duration_seconds,
    })
}

#[must_use]
pub fn trade_key(t: &Trade) -> String {
    let raw = format!(
        "{}|{}|{}|{}|{}|{}|{}",
        t.transaction_hash,
        t.asset,
        t.condition_id,
        t.timestamp,
        t.side.to_ascii_uppercase(),
        t.size.normalize(),
        t.price.normalize()
    );
    format!("{:x}", Sha256::digest(raw.as_bytes()))
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Outcome {
    Up,
    Down,
}

impl Outcome {
    pub fn parse(v: &str) -> Result<Self> {
        match v.trim().to_ascii_lowercase().as_str() {
            "up" => Ok(Self::Up),
            "down" => Ok(Self::Down),
            _ => Err(CopybotError::InvalidOutcome(v.into())),
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Up => "Up",
            Self::Down => "Down",
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CandidateSignal {
    pub wallet: String,
    pub condition_id: String,
    pub asset_id: String,
    pub outcome: Outcome,
    pub source_price: Decimal,
    pub source_timestamp: i64,
    pub market_end_epoch: i64,
    pub slug: String,
    pub title: String,
    pub strategy: String,
    pub estimated_win_probability: Decimal,
}

pub fn validate_live_ack(live: bool, ack: Option<&str>) -> Result<()> {
    if !live {
        return Ok(());
    }
    if ack != Some("I_UNDERSTAND_REAL_MONEY") {
        return Err(CopybotError::LiveTradingNotAcknowledged);
    }
    #[cfg(not(feature = "live-trading"))]
    return Err(CopybotError::LiveTradingNotCompiled);
    #[cfg(feature = "live-trading")]
    Ok(())
}
