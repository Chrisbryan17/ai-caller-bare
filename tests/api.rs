use std::time::Duration;

use polymarket_copybot::{CopybotError, DataApiClient};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

#[tokio::test]
async fn data_api_uses_wallet_filter_and_sorts_oldest_first() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/trades"))
        .and(query_param("user", "0xabc"))
        .and(query_param("limit", "2"))
        .and(query_param("offset", "0"))
        .and(query_param("takerOnly", "false"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"[
          {"proxyWallet":"0xabc","side":"BUY","asset":"2","conditionId":"c","size":5,"price":"0.2","timestamp":20,"title":"later","slug":"btc-updown-5m-1784241900","outcome":"Up","transactionHash":"b"},
          {"proxyWallet":"0xabc","side":"BUY","asset":"1","conditionId":"c","size":"5","price":0.2,"timestamp":10,"title":"earlier","slug":"btc-updown-5m-1784241900","outcome":"Up","transactionHash":"a"}
        ]"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    let client = DataApiClient::new(server.uri(), Duration::from_secs(1)).unwrap();
    let trades = client.fetch_trades("0xabc", 2).await.unwrap();
    assert_eq!(trades.len(), 2);
    assert_eq!(trades[0].timestamp, 10);
    assert_eq!(trades[1].timestamp, 20);
}

#[tokio::test]
async fn data_api_preserves_non_success_status_and_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/trades"))
        .respond_with(ResponseTemplate::new(429).set_body_string("slow down"))
        .mount(&server)
        .await;
    let client = DataApiClient::new(server.uri(), Duration::from_secs(1)).unwrap();
    match client.fetch_trades("0xabc", 2).await.unwrap_err() {
        CopybotError::HttpStatus { status, body } => {
            assert_eq!(status.as_u16(), 429);
            assert_eq!(body, "slow down");
        }
        other => panic!("unexpected error: {other}"),
    }
}
