use std::time::Duration;

use polymarket_copybot::{
    CandidateSignal, DepthAwarePaperExecutor, ExecutionRequest, Executor, Outcome, crypto_taker_fee,
};
use rust_decimal_macros::dec;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

fn request(
    shares: rust_decimal::Decimal,
    maximum_price: rust_decimal::Decimal,
) -> ExecutionRequest {
    ExecutionRequest {
        signal: CandidateSignal {
            wallet: "0x8888888888888888888888888888888888888888".into(),
            condition_id: "condition".into(),
            asset_id: "123456789".into(),
            outcome: Outcome::Up,
            source_price: dec!(0.40),
            source_timestamp: 1_000,
            market_end_epoch: 2_000,
            slug: "btc-updown-5m-1784241900".into(),
            title: "BTC".into(),
            strategy: "book-test".into(),
            estimated_win_probability: dec!(0.65),
        },
        shares,
        maximum_price,
    }
}

async fn mount_book(server: &MockServer, body: &str) {
    Mock::given(method("GET"))
        .and(path("/book"))
        .and(query_param("token_id", "123456789"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
        .mount(server)
        .await;
}

#[tokio::test]
async fn paper_fill_walks_real_ask_depth_and_uses_vwap() {
    let server = MockServer::start().await;
    mount_book(
        &server,
        r#"{
          "market":"condition",
          "asset_id":"123456789",
          "timestamp":"1000",
          "hash":"hash",
          "bids":[],
          "asks":[{"price":"0.42","size":"4"},{"price":"0.41","size":"3"}],
          "min_order_size":"1",
          "tick_size":"0.01",
          "neg_risk":false,
          "last_trade_price":"0.40"
        }"#,
    )
    .await;

    let executor =
        DepthAwarePaperExecutor::new(server.uri(), Duration::from_secs(1), dec!(0.02)).unwrap();
    let fill = executor
        .execute(request(dec!(5), dec!(0.42)))
        .await
        .unwrap();

    let collateral = dec!(3) * dec!(0.41) + dec!(2) * dec!(0.42);
    let fee = crypto_taker_fee(dec!(3), dec!(0.41)).unwrap()
        + crypto_taker_fee(dec!(2), dec!(0.42)).unwrap();
    assert_eq!(fill.shares, dec!(5));
    assert_eq!(fill.fill_price, collateral / dec!(5));
    assert_eq!(fill.fee, fee);
    assert_eq!(fill.total_cost, collateral + fee);
    assert!(fill.paper);
}

#[tokio::test]
async fn paper_fill_rejects_when_full_fok_depth_is_not_available() {
    let server = MockServer::start().await;
    mount_book(
        &server,
        r#"{
          "market":"condition","asset_id":"123456789","timestamp":"1000","hash":"hash",
          "bids":[],"asks":[{"price":"0.41","size":"4"}],
          "min_order_size":"1","tick_size":"0.01","neg_risk":false,"last_trade_price":"0.40"
        }"#,
    )
    .await;
    let executor =
        DepthAwarePaperExecutor::new(server.uri(), Duration::from_secs(1), dec!(0.02)).unwrap();
    let error = executor
        .execute(request(dec!(5), dec!(0.41)))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("insufficient executable liquidity")
    );
}

#[tokio::test]
async fn paper_limit_is_floored_to_tick_and_never_rounded_up() {
    let server = MockServer::start().await;
    mount_book(
        &server,
        r#"{
          "market":"condition","asset_id":"123456789","timestamp":"1000","hash":"hash",
          "bids":[],"asks":[{"price":"0.42","size":"10"}],
          "min_order_size":"1","tick_size":"0.01","neg_risk":false,"last_trade_price":"0.40"
        }"#,
    )
    .await;
    let executor =
        DepthAwarePaperExecutor::new(server.uri(), Duration::from_secs(1), dec!(0.02)).unwrap();
    let error = executor
        .execute(request(dec!(5), dec!(0.419)))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("insufficient executable liquidity")
    );
}

#[tokio::test]
async fn paper_fill_enforces_market_minimum_order_size() {
    let server = MockServer::start().await;
    mount_book(
        &server,
        r#"{
          "market":"condition","asset_id":"123456789","timestamp":"1000","hash":"hash",
          "bids":[],"asks":[{"price":"0.41","size":"10"}],
          "min_order_size":"6","tick_size":"0.01","neg_risk":false,"last_trade_price":"0.40"
        }"#,
    )
    .await;
    let executor =
        DepthAwarePaperExecutor::new(server.uri(), Duration::from_secs(1), dec!(0.02)).unwrap();
    let error = executor
        .execute(request(dec!(5), dec!(0.41)))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("below market minimum"));
}
