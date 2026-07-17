use std::{collections::{BTreeSet, HashMap}, sync::Arc};

use futures::{future::join_all, stream, StreamExt as _};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::{
    CandidateEvaluation, CopybotError, DataApiClient, DiscoveryApiClient, LeaderboardPeriod,
    ReplayEvaluator, Result,
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryConfig {
    pub leaderboard_limit: usize,
    pub trade_limit: usize,
    pub max_concurrency: usize,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            leaderboard_limit: 50,
            trade_limit: 1_000,
            max_concurrency: 4,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryCycleResult {
    pub period_successes: usize,
    pub period_failures: usize,
    pub candidate_wallets: usize,
    pub evaluations: Vec<CandidateEvaluation>,
    pub candidate_failures: Vec<String>,
    pub failed_closed: bool,
}

#[derive(Clone, Debug)]
pub struct DiscoveryCoordinator {
    discovery_api: DiscoveryApiClient,
    data_api: DataApiClient,
    evaluator: ReplayEvaluator,
    config: DiscoveryConfig,
}

impl DiscoveryCoordinator {
    pub fn new(
        discovery_api: DiscoveryApiClient,
        data_api: DataApiClient,
        config: DiscoveryConfig,
    ) -> Result<Self> {
        if !(1..=50).contains(&config.leaderboard_limit) {
            return Err(CopybotError::InvalidConfiguration(
                "leaderboard limit must be 1..=50".into(),
            ));
        }
        if !(1..=1_000).contains(&config.trade_limit) {
            return Err(CopybotError::InvalidConfiguration(
                "trade limit must be 1..=1000".into(),
            ));
        }
        if !(1..=16).contains(&config.max_concurrency) {
            return Err(CopybotError::InvalidConfiguration(
                "discovery concurrency must be 1..=16".into(),
            ));
        }
        Ok(Self {
            discovery_api,
            data_api,
            evaluator: ReplayEvaluator::default(),
            config,
        })
    }

    pub async fn run_cycle(
        &self,
        now: i64,
        skip_until: &HashMap<String, i64>,
    ) -> DiscoveryCycleResult {
        let periods = [
            LeaderboardPeriod::Day,
            LeaderboardPeriod::Week,
            LeaderboardPeriod::Month,
        ];
        let results = join_all(periods.into_iter().map(|period| {
            self.discovery_api
                .fetch_leaderboard(period, self.config.leaderboard_limit)
        }))
        .await;

        let mut result = DiscoveryCycleResult::default();
        let mut candidates = BTreeSet::new();
        for response in results {
            match response {
                Ok(snapshot) => {
                    result.period_successes += 1;
                    for row in snapshot.rows {
                        let wallet = row.proxy_wallet.to_ascii_lowercase();
                        if skip_until.get(&wallet).is_none_or(|until| *until <= now) {
                            candidates.insert(wallet);
                        }
                    }
                }
                Err(_) => result.period_failures += 1,
            }
        }
        result.candidate_wallets = candidates.len();
        if result.period_successes < 2 {
            result.failed_closed = true;
            return result;
        }

        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrency));
        let jobs = stream::iter(candidates.into_iter().map(|wallet| {
            let semaphore = Arc::clone(&semaphore);
            let discovery_api = self.discovery_api.clone();
            let data_api = self.data_api.clone();
            let evaluator = self.evaluator.clone();
            let trade_limit = self.config.trade_limit;
            async move {
                let permit = semaphore.acquire_owned().await.map_err(|_| {
                    CopybotError::InvalidConfiguration("discovery semaphore closed".into())
                })?;
                let evaluation = evaluate_candidate(
                    &discovery_api,
                    &data_api,
                    &evaluator,
                    &wallet,
                    trade_limit,
                    now,
                )
                .await;
                drop(permit);
                evaluation.map(|evaluation| (wallet, evaluation))
            }
        }))
        .buffer_unordered(self.config.max_concurrency);
        futures::pin_mut!(jobs);
        while let Some(candidate) = jobs.next().await {
            match candidate {
                Ok((_, evaluation)) => result.evaluations.push(evaluation),
                Err(error) => result.candidate_failures.push(error.to_string()),
            }
        }
        result
            .evaluations
            .sort_by(|left, right| left.wallet.cmp(&right.wallet));
        result.candidate_failures.sort();
        result
    }
}

async fn evaluate_candidate(
    discovery_api: &DiscoveryApiClient,
    data_api: &DataApiClient,
    evaluator: &ReplayEvaluator,
    wallet: &str,
    trade_limit: usize,
    now: i64,
) -> Result<CandidateEvaluation> {
    let trades = data_api.fetch_trades(wallet, trade_limit).await?;
    let condition_ids: Vec<String> = trades
        .iter()
        .map(|trade| trade.condition_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let resolutions = discovery_api.fetch_resolutions(&condition_ids).await?;
    evaluator.evaluate(wallet, &trades, &resolutions, now)
}
