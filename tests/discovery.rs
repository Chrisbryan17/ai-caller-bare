use std::time::Duration;

use polymarket_copybot::{
    DataApiClient, DiscoveryApiClient, DiscoveryConfig, DiscoveryCoordinator, LeaderboardPeriod,
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

async fn mount_leaderboard(server: &MockServer, period: LeaderboardPeriod, status: u16) {
    let body = if status == 200 {
        r#"[{"rank":1,"proxyWallet":"0x1111111111111111111111111111111111111111","userName":"active","vol":1000,"pnl":100}]"#
    } else {
        "failure"
    };
    Mock::given(method("GET"))
        .and(path("/v1/leaderboard"))
        .and(query_param("timePeriod", period.as_str()))
        .respond_with(ResponseTemplate::new(status).set_body_raw(body, "application/json"))
        .mount(server)
        .await;
}

fn trades_json() -> String {
    let mut rows = Vec::new();
    for index in 0..6_i64 {
        let start = 1_800_000_000 + index * 300;
        rows.push(format!(
            r#"{{"proxyWallet":"0x1111111111111111111111111111111111111111","side":"BUY","asset":"a{index}","conditionId":"c{index}","size":100,"price":0.30,"timestamp":{},"title":"BTC","slug":"btc-updown-5m-{start}","outcome":"Up","transactionHash":"tx{index}"}}"#,
            start + 120
        ));
    }
    format!("[{}]", rows.join(","))
}

fn resolutions_json() -> String {
    let rows: Vec<_> = (0..6)
        .map(|index| {
            format!(
                r#"{{"conditionId":"c{index}","closed":true,"outcomes":"[\"Up\",\"Down\"]","outcomePrices":"[\"1\",\"0\"]"}}"#
            )
        })
        .collect();
    format!("[{}]", rows.join(","))
}

async fn coordinator(data: &MockServer, gamma: &MockServer) -> DiscoveryCoordinator {
    let discovery =
        DiscoveryApiClient::new(data.uri(), gamma.uri(), Duration::from_secs(1)).unwrap();
    let trades = DataApiClient::new(data.uri(), Duration::from_secs(1)).unwrap();
    DiscoveryCoordinator::new(
        discovery,
        trades,
        DiscoveryConfig {
            leaderboard_limit: 50,
            trade_limit: 1000,
            max_concurrency: 4,
        },
    )
    .unwrap()
}

#[tokio::test]
async fn discovery_deduplicates_periods_and_evaluates_active_wallet() {
    let data = MockServer::start().await;
    let gamma = MockServer::start().await;
    for period in [
        LeaderboardPeriod::Day,
        LeaderboardPeriod::Week,
        LeaderboardPeriod::Month,
    ] {
        mount_leaderboard(&data, period, 200).await;
    }
    Mock::given(method("GET"))
        .and(path("/trades"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(trades_json(), "application/json"),
        )
        .mount(&data)
        .await;
    Mock::given(method("GET"))
        .and(path("/markets"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(resolutions_json(), "application/json"),
        )
        .mount(&gamma)
        .await;

    let result = coordinator(&data, &gamma)
        .await
        .run_cycle(1_800_002_000, &Default::default())
        .await;
    assert!(!result.failed_closed);
    assert_eq!(result.period_successes, 3);
    assert_eq!(result.candidate_wallets, 1);
    assert_eq!(result.evaluations.len(), 1);
    assert!(result.evaluations[0].eligible);
}

#[tokio::test]
async fn one_failed_period_is_tolerated_but_two_fail_closed() {
    let data = MockServer::start().await;
    let gamma = MockServer::start().await;
    mount_leaderboard(&data, LeaderboardPeriod::Day, 200).await;
    mount_leaderboard(&data, LeaderboardPeriod::Week, 500).await;
    mount_leaderboard(&data, LeaderboardPeriod::Month, 500).await;

    let result = coordinator(&data, &gamma)
        .await
        .run_cycle(1_800_002_000, &Default::default())
        .await;
    assert!(result.failed_closed);
    assert_eq!(result.period_successes, 1);
    assert!(result.evaluations.is_empty());
}
