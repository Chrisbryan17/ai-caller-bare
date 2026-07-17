use std::time::Duration;

use polymarket_copybot::{DiscoveryApiClient, LeaderboardPeriod, Outcome};
use rust_decimal_macros::dec;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

#[tokio::test]
async fn leaderboard_request_is_crypto_pnl_normalized_and_sorted() {
    let data = MockServer::start().await;
    let gamma = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/leaderboard"))
        .and(query_param("category", "CRYPTO"))
        .and(query_param("timePeriod", "DAY"))
        .and(query_param("orderBy", "PNL"))
        .and(query_param("limit", "50"))
        .and(query_param("offset", "0"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"[
              {"rank":"2","proxyWallet":"0xBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB","userName":"b","vol":20,"pnl":"3.5"},
              {"rank":"1","proxyWallet":"0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","userName":"a","vol":"10.5","pnl":5}
            ]"#,
            "application/json",
        ))
        .mount(&data)
        .await;

    let client = DiscoveryApiClient::new(data.uri(), gamma.uri(), Duration::from_secs(1)).unwrap();
    let snapshot = client
        .fetch_leaderboard(LeaderboardPeriod::Day, 50)
        .await
        .unwrap();
    assert_eq!(snapshot.period, LeaderboardPeriod::Day);
    assert_eq!(snapshot.rows[0].rank, 1);
    assert_eq!(
        snapshot.rows[0].proxy_wallet,
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(snapshot.rows[0].volume, dec!(10.5));
    assert_eq!(snapshot.rows[0].pnl, dec!(5));
}

#[tokio::test]
async fn gamma_resolution_parses_stringified_outcomes_and_prices() {
    let data = MockServer::start().await;
    let gamma = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/markets"))
        .and(query_param("condition_ids", "c1"))
        .and(query_param("closed", "true"))
        .and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"[
              {"conditionId":"c1","closed":true,"outcomes":"[\"Up\",\"Down\"]","outcomePrices":"[\"1\",\"0\"]"}
            ]"#,
            "application/json",
        ))
        .mount(&gamma)
        .await;

    let client = DiscoveryApiClient::new(data.uri(), gamma.uri(), Duration::from_secs(1)).unwrap();
    let rows = client.fetch_resolutions(&["c1".into()]).await.unwrap();
    assert_eq!(rows["c1"].winner, Some(Outcome::Up));
    assert!(rows["c1"].closed);
}

#[tokio::test]
async fn unresolved_gamma_market_has_no_winner() {
    let data = MockServer::start().await;
    let gamma = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/markets"))
        .and(query_param("condition_ids", "c2"))
        .and(query_param("closed", "true"))
        .and(query_param("limit", "50"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"[{"conditionId":"c2","closed":false,"outcomes":"[\"Up\",\"Down\"]","outcomePrices":"[\"0.52\",\"0.48\"]"}]"#,
            "application/json",
        ))
        .mount(&gamma)
        .await;
    let client = DiscoveryApiClient::new(data.uri(), gamma.uri(), Duration::from_secs(1)).unwrap();
    let rows = client.fetch_resolutions(&["c2".into()]).await.unwrap();
    assert_eq!(rows["c2"].winner, None);
    assert!(!rows["c2"].closed);
}
